// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Where the dictionary build puts bytes: the [`Scratch`] trait, the bounded parallel
//! rounds every phase runs in, and the external scatter that writes arrays in the order
//! of an index the records arrive out of.

use std::io::{self, Write};
use std::num::NonZeroUsize;

use everygraph_core::{ByteSource, MemoryAccountant, MemoryKind, MemorySource, par};

use super::DictionaryError;
use super::hash::index;

/// Storage for what the dictionary build does not keep in memory: runs spilled by the
/// distinct-key pass, the intermediate files between phases, and the sections of the
/// finished dictionary.
///
/// A writer is filled sequentially and then sealed into a [`ByteSource`] that reads the
/// same bytes back. Dropping a source frees its storage. Native builds back this with
/// files (in `everygraph-io`, or the page cache); [`MemoryScratch`] keeps everything in
/// memory, for tests, small inputs and the browser. What a scratch holds is outside the
/// memory budget: it stands for the disk.
pub trait Scratch: Sync {
    /// What a sealed writer reads back as.
    type Source: ByteSource + Send + Sync;
    /// An append-only writer.
    type Writer: Write + Send;

    /// A new, empty writer. `purpose` names what it will hold, for file names and
    /// error messages.
    ///
    /// # Errors
    ///
    /// The storage could not be created.
    fn create(&self, purpose: &'static str) -> io::Result<Self::Writer>;

    /// Ends `writer` and returns its bytes as a source.
    ///
    /// # Errors
    ///
    /// The bytes could not be flushed or reopened.
    fn seal(&self, writer: Self::Writer) -> io::Result<Self::Source>;
}

/// A [`Scratch`] that keeps every writer in memory.
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryScratch;

impl Scratch for MemoryScratch {
    type Source = MemorySource<Vec<u8>>;
    type Writer = Vec<u8>;

    fn create(&self, _purpose: &'static str) -> io::Result<Vec<u8>> {
        Ok(Vec::new())
    }

    fn seal(&self, writer: Vec<u8>) -> io::Result<Self::Source> {
        Ok(MemorySource::new(writer))
    }
}

/// Size of the write buffer in front of every scratch writer.
pub(crate) const WRITE_BUFFER: usize = 64 << 10;

/// A buffered scratch writer that counts what it wrote.
pub(crate) struct Spill<W: Write> {
    inner: io::BufWriter<W>,
    pub(crate) bytes: u64,
}

impl<W: Write> Spill<W> {
    pub(crate) fn new<Sc: Scratch<Writer = W>>(
        scratch: &Sc,
        purpose: &'static str,
    ) -> io::Result<Self> {
        Ok(Self {
            inner: io::BufWriter::with_capacity(WRITE_BUFFER, scratch.create(purpose)?),
            bytes: 0,
        })
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.bytes += widen(bytes.len());
        self.inner.write_all(bytes)
    }

    pub(crate) fn seal<Sc: Scratch<Writer = W>>(self, scratch: &Sc) -> io::Result<Sc::Source> {
        let inner = self
            .inner
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?;
        scratch.seal(inner)
    }
}

/// `usize` to `u64`: lossless on every supported target.
pub(crate) fn widen(len: usize) -> u64 {
    u64::try_from(len).expect("usize fits in u64")
}

/// Reads `len` bytes at `offset` of `source` into a new buffer.
pub(crate) fn read_range(
    source: &dyn ByteSource,
    offset: u64,
    len: u64,
) -> Result<Vec<u8>, DictionaryError> {
    let len = usize::try_from(len)
        .map_err(|_| DictionaryError::Corrupt("a section larger than memory"))?;
    let mut buf = vec![0_u8; len];
    source.read_at(offset, &mut buf)?;
    Ok(buf)
}

/// Reads a whole source.
pub(crate) fn read_all(source: &dyn ByteSource) -> Result<Vec<u8>, DictionaryError> {
    read_range(source, 0, source.len())
}

/// Memory reserved against the accountant, released on drop.
pub(crate) struct Reservation<'a> {
    accountant: &'a dyn MemoryAccountant,
    purpose: &'static str,
    bytes: u64,
}

impl<'a> Reservation<'a> {
    pub(crate) fn new(
        accountant: &'a dyn MemoryAccountant,
        purpose: &'static str,
        bytes: u64,
    ) -> Result<Self, DictionaryError> {
        accountant.try_reserve(purpose, MemoryKind::Anonymous, bytes)?;
        Ok(Self {
            accountant,
            purpose,
            bytes,
        })
    }

    /// Moves up to `bytes` of this reservation into a new one with the same purpose,
    /// without asking the accountant (so nobody can take them in between).
    pub(crate) fn split_off(&mut self, bytes: u64) -> Self {
        let moved = bytes.min(self.bytes);
        self.bytes -= moved;
        Self {
            accountant: self.accountant,
            purpose: self.purpose,
            bytes: moved,
        }
    }

    /// Gives back everything above `keep` bytes.
    pub(crate) fn shrink_to(&mut self, keep: u64) {
        if self.bytes > keep {
            self.accountant
                .release(self.purpose, MemoryKind::Anonymous, self.bytes - keep);
            self.bytes = keep;
        }
    }

    /// Reserves `more` bytes in addition.
    pub(crate) fn grow(&mut self, more: u64) -> Result<(), DictionaryError> {
        self.accountant
            .try_reserve(self.purpose, MemoryKind::Anonymous, more)?;
        self.bytes += more;
        Ok(())
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(crate) fn purpose(&self) -> &'static str {
        self.purpose
    }

    /// What the accountant has left.
    pub(crate) fn available(&self) -> u64 {
        self.accountant.available()
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.accountant
            .release(self.purpose, MemoryKind::Anonymous, self.bytes);
    }
}

impl std::fmt::Debug for Reservation<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reservation")
            .field("purpose", &self.purpose)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

/// Runs `run` on every task, in parallel rounds, and hands the results to `consume`
/// in task order.
///
/// Each task is given a reservation of its `cost` (an estimate) before its round
/// starts; it may grow it while it runs and returns it with its result, so that the
/// memory a result holds stays reserved until it is consumed. A round takes the next
/// tasks for as long as their reservations are granted. A task refused memory while
/// it runs, in a round with others, is run again in a smaller round (with the tasks
/// after it); a task refused while it runs alone is an error. The results do not
/// depend on how the tasks were grouped into rounds.
pub(crate) fn in_rounds<'a, T, R, F, C>(
    accountant: &'a dyn MemoryAccountant,
    purpose: &'static str,
    tasks: &[T],
    cost: impl Fn(&T) -> u64,
    run: F,
    mut consume: C,
) -> Result<(), DictionaryError>
where
    T: Sync,
    R: Send,
    F: Fn(&T, Reservation<'a>) -> Result<(R, Reservation<'a>), DictionaryError> + Send + Sync,
    C: FnMut(usize, R) -> Result<(), DictionaryError>,
{
    let mut next = 0;
    // Most tasks a round may take; halved when a task is refused memory mid-way.
    let mut cap = tasks.len();
    while next < tasks.len() {
        let mut grants = Vec::new();
        for task in &tasks[next..tasks.len().min(next + cap)] {
            match Reservation::new(accountant, purpose, cost(task)) {
                Ok(r) => grants.push(std::sync::Mutex::new(Some(r))),
                Err(e) if grants.is_empty() => return Err(e),
                Err(_) => break,
            }
        }
        let window = &tasks[next..next + grants.len()];
        let one = NonZeroUsize::MIN;
        let results = par::map_ranges(window.len(), one, |r| {
            let grant = grants[r.start]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .expect("each grant is taken once");
            run(&window[r.start], grant)
        });
        let alone = window.len() == 1;
        let mut done = 0;
        for result in results {
            match result {
                Ok((r, reservation)) => {
                    consume(next + done, r)?;
                    drop(reservation);
                }
                // Refused memory while others ran: retry from here in smaller rounds.
                // What came after it in this round is computed again.
                Err(DictionaryError::Budget(_)) if !alone => break,
                Err(e) => return Err(e),
            }
            done += 1;
        }
        if done < window.len() {
            cap = (window.len() / 2).max(1);
        } else {
            cap = cap.saturating_mul(2);
        }
        next += done;
    }
    Ok(())
}

/// Writes records that arrive in any order into an array ordered by their target
/// index, through range files: records are appended to the file of their target's
/// range, and each range is then loaded, ordered and written out in turn.
///
/// Every target in `0..n` must receive exactly one record. Payloads are `width` bytes,
/// or of any length when `width` is `None` (then the record carries its length and the
/// output gets an offsets array).
pub(crate) struct Scatter<'a, Sc: Scratch> {
    n: u64,
    range_len: u64,
    width: Option<usize>,
    ranges: Vec<Spill<Sc::Writer>>,
    _buffers: Reservation<'a>,
}

impl<'a, Sc: Scratch> Scatter<'a, Sc> {
    /// A scatter over `0..n` whose ranges each hold about `range_bytes` of records.
    pub(crate) fn new(
        scratch: &Sc,
        accountant: &'a dyn MemoryAccountant,
        n: u64,
        width: Option<usize>,
        record_bytes: u64,
        range_bytes: u64,
    ) -> Result<Self, DictionaryError> {
        let per_range = (range_bytes / record_bytes.max(1)).max(1 << 12);
        let count = n.div_ceil(per_range).max(1);
        let range_len = n.div_ceil(count).max(1);
        let count = n.div_ceil(range_len).max(1);
        let buffers = Reservation::new(
            accountant,
            "id dictionary write buffers",
            count * widen(WRITE_BUFFER),
        )?;
        let ranges = (0..count)
            .map(|_| Spill::new(scratch, "id dictionary scatter"))
            .collect::<io::Result<_>>()?;
        Ok(Self {
            n,
            range_len,
            width,
            ranges,
            _buffers: buffers,
        })
    }

    pub(crate) fn range_len(&self) -> u64 {
        self.range_len
    }

    pub(crate) fn range_count(&self) -> usize {
        self.ranges.len()
    }

    /// The range a target falls in.
    pub(crate) fn range_of(&self, target: u32) -> usize {
        index(u64::from(target) / self.range_len)
    }

    /// Encodes one record into `out` (the caller groups records by range).
    pub(crate) fn encode(width: Option<usize>, target: u32, payload: &[u8], out: &mut Vec<u8>) {
        out.extend_from_slice(&target.to_le_bytes());
        if width.is_none() {
            let len = u32::try_from(payload.len()).expect("a key shorter than 4 GiB");
            out.extend_from_slice(&len.to_le_bytes());
        }
        out.extend_from_slice(payload);
    }

    /// Appends encoded records, all for range `range`.
    pub(crate) fn append(&mut self, range: usize, records: &[u8]) -> io::Result<()> {
        self.ranges[range].write(records)
    }

    /// Writes the array: payloads in target order into `out` and, for variable-width
    /// payloads, `n + 1` little-endian `u64` offsets (from `offset_base`) into
    /// `offsets`.
    pub(crate) fn finish(
        self,
        scratch: &Sc,
        accountant: &dyn MemoryAccountant,
        out: &mut Spill<Sc::Writer>,
        mut offsets: Option<&mut Spill<Sc::Writer>>,
    ) -> Result<(), DictionaryError> {
        let Self {
            n,
            range_len,
            width,
            ranges,
            _buffers,
        } = self;
        let sources = ranges
            .into_iter()
            .map(|s| s.seal(scratch))
            .collect::<io::Result<Vec<_>>>()?;
        let mut position = 0_u64;
        if let Some(o) = offsets.as_deref_mut() {
            o.write(&0_u64.to_le_bytes())?;
        }
        let tasks: Vec<usize> = (0..sources.len()).collect();
        in_rounds(
            accountant,
            "id dictionary scatter",
            &tasks,
            |&r| sources[r].len().saturating_mul(3).saturating_add(1 << 16),
            |&r, grant| {
                let base = widen(r) * range_len;
                let len = range_len.min(n - base);
                Ok((place(&read_all(&sources[r])?, width, base, len)?, grant))
            },
            |_, (bytes, lens)| {
                out.write(&bytes)?;
                if let Some(o) = offsets.as_deref_mut() {
                    for l in lens {
                        position += u64::from(l);
                        o.write(&position.to_le_bytes())?;
                    }
                }
                Ok(())
            },
        )
    }
}

/// Orders the records of one range `base..base + len`: payloads concatenated in target
/// order, and their lengths for variable-width payloads.
fn place(
    records: &[u8],
    width: Option<usize>,
    base: u64,
    len: u64,
) -> Result<(Vec<u8>, Vec<u32>), DictionaryError> {
    let len = index(len);
    let missing = || DictionaryError::Corrupt("scatter: a target got no record or two");
    let mut at = 0;
    let next = |at: &mut usize| -> Result<(usize, &[u8]), DictionaryError> {
        let field = |at: &mut usize| -> Result<u32, DictionaryError> {
            let bytes = records
                .get(*at..*at + 4)
                .ok_or(DictionaryError::Corrupt("scatter: truncated"))?;
            *at += 4;
            Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
        };
        let target = u64::from(field(at)?);
        let size = match width {
            Some(w) => w,
            None => index(u64::from(field(at)?)),
        };
        let payload = records
            .get(*at..*at + size)
            .ok_or(DictionaryError::Corrupt("scatter: truncated"))?;
        *at += size;
        let slot = target
            .checked_sub(base)
            .filter(|t| *t < widen(len))
            .ok_or(DictionaryError::Corrupt("scatter: target out of range"))?;
        Ok((index(slot), payload))
    };
    match width {
        Some(w) => {
            let mut out = vec![0_u8; len * w];
            let mut filled = vec![false; len];
            while at < records.len() {
                let (t, payload) = next(&mut at)?;
                if std::mem::replace(&mut filled[t], true) {
                    return Err(missing());
                }
                out[t * w..(t + 1) * w].copy_from_slice(payload);
            }
            if filled.iter().any(|f| !f) {
                return Err(missing());
            }
            Ok((out, Vec::new()))
        }
        None => {
            let mut items: Vec<(usize, &[u8])> = Vec::with_capacity(len);
            while at < records.len() {
                items.push(next(&mut at)?);
            }
            items.sort_unstable_by_key(|(t, _)| *t);
            if items.len() != len || items.iter().enumerate().any(|(i, (t, _))| i != *t) {
                return Err(missing());
            }
            let mut out = Vec::with_capacity(items.iter().map(|(_, p)| p.len()).sum());
            let mut lens = Vec::with_capacity(len);
            for (_, p) in items {
                out.extend_from_slice(p);
                lens.push(u32::try_from(p.len()).expect("a key shorter than 4 GiB"));
            }
            Ok((out, lens))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::test_support::Budget;

    fn scatter_and_read(
        n: u32,
        width: Option<usize>,
        range_bytes: u64,
        budget: u64,
    ) -> (Vec<u8>, Vec<u8>) {
        let scratch = MemoryScratch;
        let budget = Budget::new(budget);
        let mut s = Scatter::new(&scratch, &budget, u64::from(n), width, 8, range_bytes).unwrap();
        // Targets in a scrambled order; payload = the target's bytes, repeated for
        // variable width by (target % 3).
        let payload = |t: u32| -> Vec<u8> {
            match width {
                Some(w) => t.to_le_bytes().iter().copied().cycle().take(w).collect(),
                None => t.to_le_bytes().repeat(usize::try_from(t % 3).unwrap()),
            }
        };
        let mut by_range: Vec<Vec<u8>> = vec![Vec::new(); s.range_count()];
        for i in 0..n {
            let t = u32::try_from((u64::from(i) * 7919) % u64::from(n)).unwrap();
            Scatter::<'_, MemoryScratch>::encode(
                width,
                t,
                &payload(t),
                &mut by_range[s.range_of(t)],
            );
        }
        for (r, bytes) in by_range.iter().enumerate() {
            s.append(r, bytes).unwrap();
        }
        let mut out = Spill::new(&scratch, "out").unwrap();
        let mut offsets = Spill::new(&scratch, "offsets").unwrap();
        s.finish(
            &scratch,
            &budget,
            &mut out,
            width.is_none().then_some(&mut offsets),
        )
        .unwrap();
        assert_eq!(budget.available(), budget.total(), "everything released");
        let expected: Vec<u8> = (0..n).flat_map(payload).collect();
        let out = read_all(&out.seal(&scratch).unwrap()).unwrap();
        assert_eq!(out, expected);
        (out, read_all(&offsets.seal(&scratch).unwrap()).unwrap())
    }

    #[test]
    fn scatter_orders_fixed_and_variable_payloads_whatever_the_range_size() {
        for range_bytes in [1, 1 << 15, 1 << 30] {
            let (a, _) = scatter_and_read(10_007, Some(6), range_bytes, 1 << 30);
            let (b, _) = scatter_and_read(10_007, Some(6), 1 << 30, 1 << 30);
            assert_eq!(a, b);
            let (_, offsets) = scatter_and_read(5000, None, range_bytes, 1 << 30);
            assert_eq!(offsets.len(), 8 * 5001);
        }
        scatter_and_read(0, Some(4), 1, 1 << 20);
    }

    #[test]
    fn rounds_refuse_a_task_larger_than_the_budget_and_release_on_success() {
        let budget = Budget::new(100);
        let tasks = [10_u64, 20, 30, 40, 50];
        let mut seen = Vec::new();
        in_rounds(
            &budget,
            "t",
            &tasks,
            |&c| c,
            |&c, g| Ok((c * 2, g)),
            |i, r| {
                seen.push((i, r));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(seen, vec![(0, 20), (1, 40), (2, 60), (3, 80), (4, 100)]);
        assert_eq!(budget.available(), 100);
        let err = in_rounds(
            &budget,
            "t",
            &[101_u64],
            |&c| c,
            |_, g| Ok(((), g)),
            |_, ()| Ok(()),
        )
        .unwrap_err();
        assert!(matches!(err, DictionaryError::Budget(_)));
        assert_eq!(budget.peak(), 100);
    }
}
