// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reading a column section against its layout (`docs/spec/eg-columns.md` § 7).

use everygraph_core::ByteSource;

use super::arrow::{Frame, Plan};
use super::{ColumnError, ColumnLayout};

/// Bytes compared at a time.
const CHUNK: usize = 1 << 16;

/// A column section checked against its layout: its Arrow framing is exactly the one
/// the layout implies, so the values lie where [`ColumnLayout::batches`] says, for
/// Everygraph and for any Arrow reader alike.
///
/// Opening reads every byte of the section that is not a buffer (the schema, each
/// batch's metadata page, the padding, the footer): about 4 KiB per batch. The values
/// are then read through [`source`](Self::source), zero-copy when the source is in
/// memory or mapped.
#[derive(Debug)]
pub struct Column<'a, S> {
    layout: &'a ColumnLayout,
    source: S,
}

impl<'a, S: ByteSource> Column<'a, S> {
    /// Checks `source`, the column section, against `layout`.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Invalid`] when the section's length or any byte outside its
    /// buffers differs from what the layout implies; [`ColumnError::Read`] when the
    /// source fails.
    pub fn open(layout: &'a ColumnLayout, source: S) -> Result<Self, ColumnError> {
        if source.len() != layout.column_len() {
            return Err(ColumnError::Invalid(
                "the column's length is not the one its layout implies",
            ));
        }
        let shape = layout.shape();
        let (mut plan, head) = Plan::new(shape.clone());
        expect(&source, 0, &head)?;
        if shape.dictionary.is_some() {
            check_frame(&source, &plan.dictionary())?;
        }
        for (i, batch) in (0..).zip(layout.batches()) {
            check_frame(&source, &plan.batch(shape.batch_len(i), batch.null_count))?;
        }
        expect(&source, plan.position(), &plan.tail())?;
        Ok(Self { layout, source })
    }

    /// The layout.
    #[must_use]
    pub fn layout(&self) -> &'a ColumnLayout {
        self.layout
    }

    /// The column section.
    #[must_use]
    pub fn source(&self) -> &S {
        &self.source
    }

    /// The string of dictionary code `code`, read into `buf`.
    ///
    /// # Errors
    ///
    /// [`ColumnError::Input`] for a column that is not dictionary-encoded;
    /// [`ColumnError::Invalid`] for a code past the dictionary, offsets out of order or
    /// past the strings, or bytes that are not UTF-8; [`ColumnError::Read`].
    pub fn dictionary_entry<'b>(
        &self,
        code: u64,
        buf: &'b mut Vec<u8>,
    ) -> Result<&'b str, ColumnError> {
        let d = self
            .layout
            .dictionary()
            .ok_or(ColumnError::Input("not a dictionary-encoded column"))?;
        if code >= d.values {
            return Err(ColumnError::Invalid("a code past the dictionary"));
        }
        let mut pair = [0u8; 8];
        self.source.read_at(d.offsets.start + 4 * code, &mut pair)?;
        let start = i32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]);
        let end = i32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]);
        let (Ok(start), Ok(end)) = (u64::try_from(start), u64::try_from(end)) else {
            return Err(ColumnError::Invalid("a negative dictionary offset"));
        };
        if start > end || end > d.data.end - d.data.start {
            return Err(ColumnError::Invalid(
                "dictionary offsets out of order or past the strings",
            ));
        }
        buf.clear();
        buf.resize(usize::try_from(end - start).unwrap_or(0), 0);
        self.source.read_at(d.data.start + start, buf)?;
        std::str::from_utf8(buf)
            .map_err(|_| ColumnError::Invalid("a dictionary string that is not UTF-8"))
    }
}

fn check_frame<S: ByteSource>(source: &S, frame: &Frame) -> Result<(), ColumnError> {
    expect(source, frame.at, &frame.metadata)?;
    let mut at = frame.at + frame.metadata.len() as u64;
    for b in frame.buffers.iter().filter(|b| !b.is_empty()) {
        zeros(source, at, b.start)?;
        at = b.end;
    }
    zeros(source, at, frame.end)
}

fn expect<S: ByteSource>(source: &S, at: u64, bytes: &[u8]) -> Result<(), ColumnError> {
    let mut got = vec![0; bytes.len()];
    source.read_at(at, &mut got)?;
    if got == bytes {
        Ok(())
    } else {
        Err(ColumnError::Invalid(
            "the Arrow framing is not the one the layout implies",
        ))
    }
}

fn zeros<S: ByteSource>(source: &S, mut at: u64, end: u64) -> Result<(), ColumnError> {
    let mut buf = vec![0u8; CHUNK.min(usize::try_from(end - at).unwrap_or(CHUNK))];
    while at < end {
        let n = buf.len().min(usize::try_from(end - at).unwrap_or(CHUNK));
        source.read_at(at, &mut buf[..n])?;
        if buf[..n].iter().any(|&b| b != 0) {
            return Err(ColumnError::Invalid(
                "padding in a column section is not zero",
            ));
        }
        at += n as u64;
    }
    Ok(())
}
