//! Per-node sorted neighbor list codecs.
//! Layout of every byte codec (varint/svb/bp): per node a byte-aligned record
//!   [deg: LEB128] [first: LEB128 zigzag(n0 - v)] [rest...]
//! records are concatenated in node order; index = Elias–Fano over the n+1 byte offsets.
//! zstd/lz4: raw u32 targets cut into chunks of ~CHUNK entries at node boundaries,
//! each compressed independently; index = EF over entry offsets + per-chunk (first node, byte offset).
use crate::csr::*;
use anyhow::{Result, bail};
use bitpacking::{BitPacker, BitPacker1x, BitPacker4x};
use mem_dbg::{MemSize, SizeFlags};
use rayon::prelude::*;
use sux::prelude::*;
use sux::traits::IndexedSeq;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Codec {
    Raw,
    Varint,
    Svb,
    Bp,
    Zstd(i32),
    Lz4,
    /// delta+LEB128 records, then zstd over chunks of ~VCHUNK entries (node aligned)
    VarZstd(i32),
}
pub const VCHUNK: u64 = 65536;
pub const CHUNK: u64 = 16384; // entries (64 KiB raw) per zstd/lz4 chunk

impl Codec {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "raw" => Codec::Raw,
            "varint" => Codec::Varint,
            "svb" => Codec::Svb,
            "bp" => Codec::Bp,
            "zstd1" => Codec::Zstd(1),
            "zstd3" => Codec::Zstd(3),
            "zstd9" => Codec::Zstd(9),
            "lz4" => Codec::Lz4,
            "vzstd3" => Codec::VarZstd(3),
            "vzstd9" => Codec::VarZstd(9),
            _ => bail!("unknown codec {s}"),
        })
    }
    fn is_chunked(self) -> bool {
        matches!(self, Codec::Zstd(_) | Codec::Lz4 | Codec::VarZstd(_))
    }
}

#[inline(always)]
pub fn put_varint(mut x: u64, out: &mut Vec<u8>) {
    while x >= 0x80 {
        out.push((x as u8) | 0x80);
        x >>= 7;
    }
    out.push(x as u8);
}
#[inline(always)]
pub fn get_varint(d: &[u8], pos: &mut usize) -> u64 {
    let b = unsafe { *d.get_unchecked(*pos) };
    *pos += 1;
    if b < 0x80 {
        return b as u64;
    }
    let mut x = (b & 0x7f) as u64;
    let mut sh = 7;
    loop {
        let b = unsafe { *d.get_unchecked(*pos) };
        *pos += 1;
        x |= ((b & 0x7f) as u64) << sh;
        if b < 0x80 {
            return x;
        }
        sh += 7;
    }
}
#[inline(always)]
fn zz(x: i64) -> u64 {
    ((x << 1) ^ (x >> 63)) as u64
}
#[inline(always)]
fn unzz(x: u64) -> i64 {
    ((x >> 1) as i64) ^ -((x & 1) as i64)
}

pub fn varint_list_u64(v: u64, l: impl Iterator<Item = u64>, out: &mut Vec<u8>) {
    let l: Vec<u64> = l.collect();
    put_varint(l.len() as u64, out);
    if l.is_empty() {
        return;
    }
    put_varint(zz(l[0] as i64 - v as i64), out);
    for w in l.windows(2) {
        put_varint(w[1] - w[0] - 1, out);
    }
}

fn encode_list(c: Codec, v: usize, l: &[u32], out: &mut Vec<u8>, tmp: &mut Vec<u32>) {
    put_varint(l.len() as u64, out);
    if l.is_empty() {
        return;
    }
    let first = zz(l[0] as i64 - v as i64);
    match c {
        Codec::Varint => {
            put_varint(first, out);
            for w in l.windows(2) {
                put_varint((w[1] - w[0] - 1) as u64, out);
            }
        }
        Codec::Svb => {
            tmp.clear();
            tmp.push(first as u32); // fits: |n0 - v| < 2^31
            for w in l.windows(2) {
                tmp.push(w[1] - w[0] - 1);
            }
            crate::svb::encode(tmp, out);
        }
        Codec::Bp => {
            put_varint(first, out);
            let mut prev = l[0];
            let mut rest = &l[1..];
            let bp4 = BitPacker4x::new();
            let bp1 = BitPacker1x::new();
            while rest.len() >= 128 {
                let blk = &rest[..128];
                let nb = bp4.num_bits_strictly_sorted(Some(prev), blk);
                out.push(nb);
                let s = out.len();
                out.resize(s + 16 * nb as usize, 0);
                let w = bp4.compress_strictly_sorted(Some(prev), blk, &mut out[s..], nb);
                debug_assert_eq!(w, 16 * nb as usize);
                prev = blk[127];
                rest = &rest[128..];
            }
            while rest.len() >= 32 {
                let blk = &rest[..32];
                let nb = bp1.num_bits_strictly_sorted(Some(prev), blk);
                out.push(nb);
                let s = out.len();
                out.resize(s + 4 * nb as usize, 0);
                bp1.compress_strictly_sorted(Some(prev), blk, &mut out[s..], nb);
                prev = blk[31];
                rest = &rest[32..];
            }
            for &x in rest {
                put_varint((x - prev - 1) as u64, out);
                prev = x;
            }
        }
        _ => unreachable!(),
    }
}

/// Decodes the record at `pos` into buf; returns slice length.
#[inline(always)]
fn decode_list(c: Codec, v: usize, d: &[u8], pos: &mut usize, buf: &mut Vec<u32>) -> usize {
    let deg = get_varint(d, pos) as usize;
    if deg == 0 {
        return 0;
    }
    if buf.len() < deg + 8 {
        buf.resize(deg + 8, 0);
    }
    match c {
        Codec::Varint => {
            let mut x = (v as i64 + unzz(get_varint(d, pos))) as u32;
            buf[0] = x;
            for i in 1..deg {
                x = x + 1 + get_varint(d, pos) as u32;
                unsafe { *buf.get_unchecked_mut(i) = x };
            }
        }
        Codec::Svb => {
            let used = crate::svb::decode(&d[*pos..], deg, &mut buf[..]);
            *pos += used;
            let b = &mut buf[..deg];
            let mut x = (v as i64 + unzz(b[0] as u64)) as u32;
            b[0] = x;
            for y in b[1..].iter_mut() {
                x = x + 1 + *y;
                *y = x;
            }
        }
        Codec::Bp => {
            let mut prev = (v as i64 + unzz(get_varint(d, pos))) as u32;
            buf[0] = prev;
            let mut i = 1;
            let bp4 = BitPacker4x::new();
            let bp1 = BitPacker1x::new();
            while deg - i >= 128 {
                let nb = d[*pos];
                *pos += 1;
                let r = bp4.decompress_strictly_sorted(Some(prev), &d[*pos..], &mut buf[i..i + 128], nb);
                *pos += r;
                prev = buf[i + 127];
                i += 128;
            }
            while deg - i >= 32 {
                let nb = d[*pos];
                *pos += 1;
                let r = bp1.decompress_strictly_sorted(Some(prev), &d[*pos..], &mut buf[i..i + 32], nb);
                *pos += r;
                prev = buf[i + 31];
                i += 32;
            }
            while i < deg {
                prev = prev + 1 + get_varint(d, pos) as u32;
                buf[i] = prev;
                i += 1;
            }
        }
        _ => unreachable!(),
    }
    deg
}

pub struct EncodedBuild {
    pub codec: Codec,
    pub data: Vec<u8>,
    pub idx: Vec<u64>,     // byte offsets (n+1) for byte codecs; entry offsets for chunked; raw: entry offsets
    pub chunks: Vec<u64>, // chunked: pairs (first node, byte offset), plus sentinel
}

pub fn encode(g: &Csr, c: Codec) -> EncodedBuild {
    let n = g.n;
    match c {
        Codec::Raw => {
            let b: &[u8] = unsafe { std::slice::from_raw_parts(g.targets.as_ptr() as *const u8, g.m * 4) };
            EncodedBuild { codec: c, data: b.to_vec(), idx: g.offsets.to_vec(), chunks: vec![] }
        }
        Codec::Zstd(_) | Codec::Lz4 => {
            // chunk boundaries at node boundaries
            let mut cb = vec![0usize];
            let mut lo = 0;
            while lo < n {
                let goal = g.offsets[lo] + CHUNK;
                let hi = g.offsets.partition_point(|&o| o < goal).max(lo + 1).min(n);
                cb.push(hi);
                lo = hi;
            }
            let parts: Vec<Vec<u8>> = cb.par_windows(2).map(|w| {
                let t = &g.targets[g.offsets[w[0]] as usize..g.offsets[w[1]] as usize];
                let b: &[u8] = unsafe { std::slice::from_raw_parts(t.as_ptr() as *const u8, t.len() * 4) };
                match c {
                    Codec::Zstd(l) => zstd::bulk::compress(b, l).unwrap(),
                    _ => lz4_flex::block::compress(b),
                }
            }).collect();
            let mut data = Vec::new();
            let mut chunks = Vec::new();
            for (i, p) in parts.iter().enumerate() {
                chunks.push(cb[i] as u64);
                chunks.push(data.len() as u64);
                data.extend_from_slice(p);
            }
            chunks.push(n as u64);
            chunks.push(data.len() as u64);
            EncodedBuild { codec: c, data, idx: g.offsets.to_vec(), chunks }
        }
        Codec::VarZstd(l) => {
            let v = encode(g, Codec::Varint);
            let mut cb = vec![0usize];
            let mut lo = 0;
            while lo < n {
                let goal = g.offsets[lo] + VCHUNK;
                let hi = g.offsets.partition_point(|&o| o < goal).max(lo + 1).min(n);
                cb.push(hi);
                lo = hi;
            }
            let parts: Vec<Vec<u8>> = cb.par_windows(2).map(|w| zstd::bulk::compress(&v.data[v.idx[w[0]] as usize..v.idx[w[1]] as usize], l).unwrap()).collect();
            let mut data = Vec::new();
            let mut chunks = Vec::new();
            for (i, p) in parts.iter().enumerate() {
                chunks.push(cb[i] as u64);
                chunks.push(data.len() as u64);
                data.extend_from_slice(p);
            }
            chunks.push(n as u64);
            chunks.push(data.len() as u64);
            EncodedBuild { codec: c, data, idx: v.idx, chunks }
        }
        _ => {
            let ch = make_chunks(g.offsets, 1 << 16);
            let parts: Vec<(Vec<u8>, Vec<u32>)> = ch.par_iter().map(|&(lo, hi)| {
                let mut out = Vec::new();
                let mut rel = Vec::with_capacity(hi - lo);
                let mut tmp = Vec::new();
                for v in lo..hi {
                    rel.push(out.len() as u32);
                    encode_list(c, v, g.list(v), &mut out, &mut tmp);
                }
                (out, rel)
            }).collect();
            let total: usize = parts.iter().map(|p| p.0.len()).sum();
            let mut data = Vec::with_capacity(total + 64);
            let mut idx = Vec::with_capacity(n + 1);
            for (o, rel) in parts {
                let base = data.len() as u64;
                idx.extend(rel.iter().map(|&r| base + r as u64));
                data.extend_from_slice(&o);
            }
            idx.push(data.len() as u64);
            data.extend_from_slice(&[0u8; 64]); // padding for SIMD over-read safety
            EncodedBuild { codec: c, data, idx, chunks: vec![] }
        }
    }
}

impl EncodedBuild {
    pub fn write(&self, prefix: &str) -> Result<()> {
        std::fs::write(format!("{prefix}.bin"), &self.data)?;
        write_u64s(&format!("{prefix}.idx"), &self.idx)?;
        if !self.chunks.is_empty() {
            write_u64s(&format!("{prefix}.chk"), &self.chunks)?;
        }
        Ok(())
    }
}

thread_local! { static VBUF: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) }; }

pub struct Encoded {
    pub codec: Codec,
    _mm: memmap2::Mmap,
    pub data: &'static [u8],
    ef: EfSeq,
    raw_off: Vec<u64>, // only for Raw (plain u64 offsets, as in glab-csr-v0)
    chunk_first: Vec<u32>,
    chunk_byte: Vec<u64>,
}

impl Encoded {
    pub fn data_path(&self, prefix: &str) -> String {
        format!("{prefix}.bin")
    }
    pub fn load(prefix: &str, c: Codec, n: usize) -> Result<Self> {
        let f = std::fs::File::open(format!("{prefix}.bin"))?;
        let mm = unsafe { memmap2::Mmap::map(&f)? };
        let data: &'static [u8] = unsafe { std::slice::from_raw_parts(mm.as_ptr(), mm.len()) };
        let idx = read_u64s(&format!("{prefix}.idx"))?;
        assert_eq!(idx.len(), n + 1);
        let u = *idx.last().unwrap() as usize;
        let mut b = EliasFanoBuilder::new(n + 1, u);
        for &x in &idx {
            b.push(x as usize);
        }
        let ef = b.build_with_seq();
        let (mut cf, mut cbyte) = (vec![], vec![]);
        if c.is_chunked() {
            let ch = read_u64s(&format!("{prefix}.chk"))?;
            for p in ch.chunks_exact(2) {
                cf.push(p[0] as u32);
                cbyte.push(p[1]);
            }
        }
        let raw_off = if c == Codec::Raw { idx } else { vec![] };
        Ok(Encoded { codec: c, _mm: mm, data, ef, raw_off, chunk_first: cf, chunk_byte: cbyte })
    }
    pub fn index_bytes(&self) -> usize {
        match self.codec {
            Codec::Raw => self.raw_off.len() * 8,
            Codec::Zstd(_) | Codec::Lz4 | Codec::VarZstd(_) => self.ef.mem_size(SizeFlags::default()) + self.chunk_first.len() * 12,
            _ => self.ef.mem_size(SizeFlags::default()),
        }
    }
    pub fn index_note(&self) -> &'static str {
        match self.codec {
            Codec::Raw => "u64 offsets (glab-csr-v0)",
            Codec::Zstd(_) | Codec::Lz4 => "EF over entry offsets + chunk table (u32 first node + u64 byte offset)",
            Codec::VarZstd(_) => "EF over uncompressed varint byte offsets + chunk table",
            _ => "Elias-Fano (sux EfSeq) over byte offsets",
        }
    }
    pub fn byte_off_per_node(&self) -> Vec<u64> {
        let n = self.ef.len() - 1;
        match self.codec {
            Codec::Raw => self.raw_off.iter().map(|&x| x * 4).collect(),
            Codec::Zstd(_) | Codec::Lz4 | Codec::VarZstd(_) => {
                let mut v = vec![0u64; n + 1];
                for c in 0..self.chunk_first.len() - 1 {
                    for x in self.chunk_first[c]..self.chunk_first[c + 1] {
                        v[x as usize] = self.chunk_byte[c];
                    }
                }
                v[n] = *self.chunk_byte.last().unwrap();
                v
            }
            _ => (0..=n).map(|i| self.ef.get(i) as u64).collect(),
        }
    }
    /// VarZstd: decompress chunk c into vb (bytes); returns uncompressed byte offset of the chunk start.
    fn vdecompress(&self, c: usize, vb: &mut Vec<u8>) -> usize {
        let b0 = self.ef.get(self.chunk_first[c] as usize);
        let b1 = self.ef.get(self.chunk_first[c + 1] as usize);
        vb.clear();
        vb.resize(b1 - b0 + 16, 0);
        let src = &self.data[self.chunk_byte[c] as usize..self.chunk_byte[c + 1] as usize];
        let w = zstd::bulk::decompress_to_buffer(src, &mut vb[..b1 - b0]).unwrap();
        debug_assert_eq!(w, b1 - b0);
        b0
    }
    #[inline]
    fn decompress_chunk(&self, c: usize, buf: &mut Vec<u32>) -> usize {
        let e0 = self.ef.get(self.chunk_first[c] as usize);
        let e1 = self.ef.get(self.chunk_first[c + 1] as usize);
        let cnt = e1 - e0;
        if buf.len() < cnt {
            buf.resize(cnt, 0);
        }
        let src = &self.data[self.chunk_byte[c] as usize..self.chunk_byte[c + 1] as usize];
        let dst: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, cnt * 4) };
        let w = match self.codec {
            Codec::Zstd(_) => zstd::bulk::decompress_to_buffer(src, dst).unwrap(),
            _ => lz4_flex::block::decompress_into(src, dst).unwrap(),
        };
        debug_assert_eq!(w, cnt * 4);
        e0
    }
}

impl Lists for Encoded {
    fn work_chunks(&self, default: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
        if self.codec.is_chunked() {
            self.chunk_first.windows(2).map(|w| (w[0] as usize, w[1] as usize)).collect()
        } else {
            default
        }
    }
    fn for_range<F: FnMut(usize, &[u32])>(&self, lo: usize, hi: usize, buf: &mut Vec<u32>, mut f: F) {
        match self.codec {
            Codec::Raw => {
                let t: &[u32] = unsafe { std::slice::from_raw_parts(self.data.as_ptr() as *const u32, self.data.len() / 4) };
                for v in lo..hi {
                    f(v, unsafe { t.get_unchecked(self.raw_off[v] as usize..self.raw_off[v + 1] as usize) });
                }
            }
            Codec::VarZstd(_) => {
                VBUF.with(|vb| {
                    let vb = &mut *vb.borrow_mut();
                    let mut c = self.chunk_first.partition_point(|&x| x as usize <= lo) - 1;
                    let mut v = lo;
                    while v < hi {
                        let b0 = self.vdecompress(c, vb);
                        let cend = (self.chunk_first[c + 1] as usize).min(hi);
                        let mut pos = self.ef.get(v) - b0;
                        while v < cend {
                            let d = decode_list(Codec::Varint, v, vb, &mut pos, buf);
                            f(v, &buf[..d]);
                            v += 1;
                        }
                        c += 1;
                    }
                })
            }
            Codec::Zstd(_) | Codec::Lz4 => {
                let mut c = self.chunk_first.partition_point(|&x| x as usize <= lo) - 1;
                let mut v = lo;
                while v < hi {
                    let e0 = self.decompress_chunk(c, buf);
                    let cend = (self.chunk_first[c + 1] as usize).min(hi);
                    let mut s = self.ef.get(v);
                    while v < cend {
                        let e = self.ef.get(v + 1);
                        f(v, &buf[s - e0..e - e0]);
                        s = e;
                        v += 1;
                    }
                    c += 1;
                }
            }
            c => {
                let mut pos = self.ef.get(lo);
                for v in lo..hi {
                    let d = decode_list(c, v, self.data, &mut pos, buf);
                    f(v, &buf[..d]);
                }
            }
        }
    }
    fn get<'b>(&self, v: usize, buf: &'b mut Vec<u32>) -> &'b [u32] {
        match self.codec {
            Codec::Raw => {
                let t: &[u32] = unsafe { std::slice::from_raw_parts(self.data.as_ptr() as *const u32, self.data.len() / 4) };
                buf.clear();
                buf.extend_from_slice(&t[self.raw_off[v] as usize..self.raw_off[v + 1] as usize]);
                buf
            }
            Codec::VarZstd(_) => VBUF.with(|vb| {
                let vb = &mut *vb.borrow_mut();
                let c = self.chunk_first.partition_point(|&x| x as usize <= v) - 1;
                let b0 = self.vdecompress(c, vb);
                let mut pos = self.ef.get(v) - b0;
                let d = decode_list(Codec::Varint, v, vb, &mut pos, buf);
                &buf[..d]
            }),
            Codec::Zstd(_) | Codec::Lz4 => {
                let c = self.chunk_first.partition_point(|&x| x as usize <= v) - 1;
                let e0 = self.decompress_chunk(c, buf);
                let (s, e) = (self.ef.get(v) - e0, self.ef.get(v + 1) - e0);
                buf.copy_within(s..e, 0);
                &buf[..e - s]
            }
            c => {
                let mut pos = self.ef.get(v);
                let d = decode_list(c, v, self.data, &mut pos, buf);
                &buf[..d]
            }
        }
    }
}

