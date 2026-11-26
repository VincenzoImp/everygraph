//! Issue #172 (m1-15) step 2: a budgeted block cache over a store file, the prototype of
//! `docs/research/sota-io-memory.md` C1.
//!
//! - Fixed-size frames (a power of two, at least 4 KiB) in anonymous memory, reserved in the
//!   accountant (`acct::res`), so the budget covers them and global reclaim cannot take them
//!   (the scopes run with MemorySwapMax=0).
//! - CLOCK eviction per shard. Frame `f` belongs to shard `(f >> GROUP_BITS) % shards`, so a
//!   run of neighbouring frames shares a lock.
//! - A miss reads the 4 KiB-aligned frame with `pread`, with `O_DIRECT` when asked: then no
//!   page cache is used and the bytes read are the misses times the frame size.
//! - With `coalesce`, the caller announces each block of nodes (`prefetch`): its missing
//!   frames are read with one `pread` per run. Not done: batching across blocks, io_uring.
//!
//! No `unsafe`: the frame memory is a `Vec<u8>`, aligned for `O_DIRECT` by skipping to the
//! first 4 KiB boundary inside it.
use crate::acct::Res;
use std::fs::File;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;

const GROUP_BITS: u32 = 4;
const ALIGN: usize = 4096;
const EMPTY: u32 = u32::MAX;

struct Shard {
    /// frame id held by each slot (u64::MAX: empty)
    frame_of: Vec<u64>,
    referenced: Vec<bool>,
    hand: usize,
    mem: Vec<u8>,
    base: usize,
}

pub struct BlockCache {
    file: File,
    file_len: u64,
    frame: usize,
    shift: u32,
    shards: Vec<Mutex<Shard>>,
    /// slot of every frame of the file within its shard (EMPTY: not cached). Read and
    /// written only under the frame's shard lock; atomics only to share the vector.
    slot_of: Vec<AtomicU32>,
    pub hits: AtomicU64,
    pub misses: AtomicU64,
    pub coalesced_reads: AtomicU64,
    /// `prefetch` does something only when this is set (`--cache-prefetch`)
    pub coalesce: bool,
    pub read_bytes: AtomicU64,
    pub direct: bool,
    pub bytes: u64,
    on: AtomicBool,
    res: Mutex<Option<Res>>,
}

impl BlockCache {
    /// A cache of `bytes` (rounded down to whole frames, at least one per shard) over `path`.
    pub fn open(path: &Path, bytes: u64, frame: usize, direct: bool, coalesce: bool) -> std::io::Result<BlockCache> {
        assert!(frame.is_power_of_two() && frame >= ALIGN, "frame size must be a power of two >= 4 KiB");
        let mut o = std::fs::OpenOptions::new();
        o.read(true);
        if direct {
            o.custom_flags(libc::O_DIRECT);
        }
        let file = o.open(path)?;
        let file_len = file.metadata()?.len();
        let nframes = file_len.div_ceil(frame as u64) as usize;
        let slots = ((bytes / frame as u64) as usize).max(1);
        let nshards = (slots / 64).clamp(1, 4096);
        let per = slots.div_ceil(nshards);
        let total = (per * nshards * frame + nshards * ALIGN) as u64 + (nframes * 4) as u64;
        let res = crate::acct::res("level-0 block cache (m1-15)", total);
        let shards = (0..nshards)
            .map(|_| {
                let mem = vec![0u8; per * frame + ALIGN];
                let base = (ALIGN - mem.as_ptr() as usize % ALIGN) % ALIGN;
                Mutex::new(Shard { frame_of: vec![u64::MAX; per], referenced: vec![false; per], hand: 0, mem, base })
            })
            .collect();
        let slot_of = (0..nframes).map(|_| AtomicU32::new(EMPTY)).collect();
        Ok(BlockCache {
            file,
            file_len,
            frame,
            shift: frame.trailing_zeros(),
            shards,
            slot_of,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            coalesced_reads: AtomicU64::new(0),
            coalesce,
            read_bytes: AtomicU64::new(0),
            direct,
            bytes: total,
            on: AtomicBool::new(true),
            res: Mutex::new(Some(res)),
        })
    }

    /// False once `release` ran: callers then read the mapping instead.
    #[inline(always)]
    pub fn on(&self) -> bool {
        self.on.load(Relaxed)
    }

    /// Frees the frames and gives the reservation back. Callers must have stopped using the
    /// cache (between phases): a reader racing with this would see an empty shard and panic.
    pub fn release(&self) {
        self.on.store(false, Relaxed);
        for sh in &self.shards {
            let mut sh = sh.lock().unwrap();
            sh.mem = Vec::new();
            sh.frame_of = Vec::new();
            sh.referenced = Vec::new();
        }
        self.res.lock().unwrap().take();
    }

    /// Appends bytes `[start, end)` of the file to `out`.
    pub fn read_into(&self, start: u64, end: u64, out: &mut Vec<u8>) {
        let end = end.min(self.file_len);
        let mut pos = start;
        while pos < end {
            let f = pos >> self.shift;
            let in_frame = (pos - (f << self.shift)) as usize;
            let take = ((end - pos) as usize).min(self.frame - in_frame);
            self.with_frame(f, |b| out.extend_from_slice(&b[in_frame..in_frame + take]));
            pos += take as u64;
        }
    }

    fn with_frame<F: FnOnce(&[u8])>(&self, f: u64, use_it: F) {
        let mut sh = self.shard(f);
        let fr = self.frame;
        let s = self.slot_of[f as usize].load(Relaxed);
        if s != EMPTY {
            self.hits.fetch_add(1, Relaxed);
            sh.referenced[s as usize] = true;
            let o = sh.base + s as usize * fr;
            use_it(&sh.mem[o..o + fr]);
            return;
        }
        self.misses.fetch_add(1, Relaxed);
        let v = self.take_slot(&mut sh);
        let o = sh.base + v * fr;
        let got = self.read_frames(f, &mut sh.mem[o..o + fr]);
        self.read_bytes.fetch_add(got as u64, Relaxed);
        self.install(&mut sh, v, f);
        use_it(&sh.mem[o..o + fr]);
    }

    /// Reads the missing frames of bytes `[start, end)` before they are used: one `pread` per
    /// run of neighbouring missing frames within a shard group. Called once per block of
    /// nodes, whose records are contiguous, this is the block's coalesced request.
    pub fn prefetch(&self, start: u64, end: u64) {
        let end = end.min(self.file_len);
        if start >= end {
            return;
        }
        let (mut f, last) = (start >> self.shift, (end - 1) >> self.shift);
        while f <= last {
            let group_last = (((f >> GROUP_BITS) + 1) << GROUP_BITS) - 1;
            let b = last.min(group_last);
            let mut sh = self.shard(f);
            while f <= b {
                let s = self.slot_of[f as usize].load(Relaxed);
                if s != EMPTY {
                    sh.referenced[s as usize] = true;
                    f += 1;
                    continue;
                }
                let mut e = f;
                while e < b && self.slot_of[e as usize + 1].load(Relaxed) == EMPTY {
                    e += 1;
                }
                let k = (e - f + 1) as usize;
                let fr = self.frame;
                let mut tmp = vec![0u8; k * fr + ALIGN];
                let base = (ALIGN - tmp.as_ptr() as usize % ALIGN) % ALIGN;
                let got = self.read_frames(f, &mut tmp[base..base + k * fr]);
                self.read_bytes.fetch_add(got as u64, Relaxed);
                self.misses.fetch_add(k as u64, Relaxed);
                self.coalesced_reads.fetch_add(1, Relaxed);
                for i in 0..k {
                    let v = self.take_slot(&mut sh);
                    let o = sh.base + v * fr;
                    sh.mem[o..o + fr].copy_from_slice(&tmp[base + i * fr..base + (i + 1) * fr]);
                    self.install(&mut sh, v, f + i as u64);
                }
                f = e + 1;
            }
        }
    }

    fn shard(&self, f: u64) -> std::sync::MutexGuard<'_, Shard> {
        self.shards[((f >> GROUP_BITS) % self.shards.len() as u64) as usize].lock().unwrap()
    }

    /// CLOCK: the first slot not referenced since the hand last passed it; its old frame is
    /// forgotten.
    fn take_slot(&self, sh: &mut Shard) -> usize {
        let n = sh.frame_of.len();
        let v = loop {
            let h = sh.hand;
            sh.hand = (h + 1) % n;
            if sh.referenced[h] {
                sh.referenced[h] = false;
            } else {
                break h;
            }
        };
        let old = sh.frame_of[v];
        if old != u64::MAX {
            self.slot_of[old as usize].store(EMPTY, Relaxed);
        }
        v
    }

    fn install(&self, sh: &mut Shard, v: usize, f: u64) {
        sh.frame_of[v] = f;
        sh.referenced[v] = true;
        self.slot_of[f as usize].store(v as u32, Relaxed);
    }

    /// Fills `buf` (whole frames, 4 KiB-aligned) from frame `f` on; zeros past the end of the
    /// file. Returns the bytes read.
    fn read_frames(&self, f: u64, buf: &mut [u8]) -> usize {
        let off = f << self.shift;
        let mut got = 0usize;
        while got < buf.len() {
            match self.file.read_at(&mut buf[got..], off + got as u64) {
                Ok(0) => break,
                Ok(k) => got += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => panic!("block cache: pread at {}: {e}", off + got as u64),
            }
        }
        buf[got..].fill(0);
        got
    }

    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({
            "bytes": self.bytes, "frame_bytes": self.frame, "shards": self.shards.len(),
            "o_direct": self.direct, "hits": self.hits.load(Relaxed), "misses": self.misses.load(Relaxed), "coalesced_reads": self.coalesced_reads.load(Relaxed), "coalesce": self.coalesce,
            "read_bytes": self.read_bytes.load(Relaxed),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_equal_the_file_under_eviction() {
        crate::acct::set_budget_for_tests(u64::MAX);
        let dir = std::env::temp_dir().join(format!("bcache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        let data: Vec<u8> = (0..300_001u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        // 8 frames of 4 KiB over a 293 KiB file: most reads miss and evict
        let c = BlockCache::open(&p, 8 * 4096, 4096, false, true).unwrap();
        let mut x = 12345u64;
        let mut out = Vec::new();
        for _ in 0..20_000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let a = (x >> 33) % data.len() as u64;
            let b = (a + (x >> 20) % 9000).min(data.len() as u64 + 50);
            out.clear();
            if x % 3 == 0 {
                c.prefetch(a, b);
            }
            c.read_into(a, b, &mut out);
            assert_eq!(&out[..], &data[a as usize..(b as usize).min(data.len())]);
        }
        assert!(c.misses.load(Relaxed) > 0 && c.hits.load(Relaxed) > 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
