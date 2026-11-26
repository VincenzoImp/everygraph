// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A FlatBuffers encoder for the few Arrow tables a column section holds
//! (`docs/spec/eg-columns.md` § 3.2).
//!
//! The layout is fixed by the tree alone: the root offset, then each table as its
//! vtable, its fields in the order given, then its children, depth first. Every offset
//! points forward, no vtable is shared and no field is left out for equalling its
//! default, so equal trees give equal bytes.

/// A field of a table.
pub(super) enum Field<'a> {
    U8(u8),
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    Str(&'a str),
    Table(Table<'a>),
    Tables(Vec<Table<'a>>),
    /// A vector of structs aligned to 8 bytes: their count and their bytes.
    Structs(usize, Vec<u8>),
}

/// A table: `(slot, field)` pairs, slots in increasing order.
pub(super) struct Table<'a>(pub(super) Vec<(u16, Field<'a>)>);

/// The bytes of a FlatBuffer whose root is `root`.
pub(super) fn encode(root: &Table<'_>) -> Vec<u8> {
    let mut buf = vec![0; 4];
    let at = table(&mut buf, root);
    patch(&mut buf, 0, at);
    buf
}

fn pad(buf: &mut Vec<u8>, align: usize) {
    buf.resize(buf.len().next_multiple_of(align), 0);
}

/// Writes at `at` the forward offset to `target`. The buffers encoded here stay far
/// below 4 GiB (§ 3.1 bounds the footer, the largest of them, by the batch count).
fn patch(buf: &mut [u8], at: usize, target: usize) {
    let rel = u32::try_from(target - at).unwrap_or(u32::MAX);
    buf[at..at + 4].copy_from_slice(&rel.to_le_bytes());
}

fn u16_of(x: usize) -> u16 {
    u16::try_from(x).unwrap_or(u16::MAX)
}

fn table(buf: &mut Vec<u8>, t: &Table<'_>) -> usize {
    let slots = t.0.last().map_or(0, |(s, _)| usize::from(*s) + 1);
    pad(buf, 4);
    let vtable = buf.len();
    buf.extend_from_slice(&u16_of(4 + 2 * slots).to_le_bytes());
    buf.extend(std::iter::repeat_n(0, 2 + 2 * slots));
    pad(buf, 4);
    let start = buf.len();
    let soffset = i32::try_from(start - vtable).unwrap_or(i32::MAX);
    buf.extend_from_slice(&soffset.to_le_bytes());
    let mut children = Vec::new();
    for (slot, field) in &t.0 {
        let size = match field {
            Field::U8(_) | Field::Bool(_) => 1,
            Field::I16(_) => 2,
            Field::I64(_) => 8,
            _ => 4,
        };
        pad(buf, size);
        let at = 2 * usize::from(*slot) + 4 + vtable;
        let field_at = u16_of(buf.len() - start);
        buf[at..at + 2].copy_from_slice(&field_at.to_le_bytes());
        match field {
            Field::U8(x) => buf.push(*x),
            Field::Bool(x) => buf.push(u8::from(*x)),
            Field::I16(x) => buf.extend_from_slice(&x.to_le_bytes()),
            Field::I32(x) => buf.extend_from_slice(&x.to_le_bytes()),
            Field::I64(x) => buf.extend_from_slice(&x.to_le_bytes()),
            _ => {
                children.push((buf.len(), field));
                buf.extend_from_slice(&[0; 4]);
            }
        }
    }
    let size = u16_of(buf.len() - start);
    buf[vtable + 2..vtable + 4].copy_from_slice(&size.to_le_bytes());
    for (at, field) in children {
        let target = child(buf, field);
        patch(buf, at, target);
    }
    start
}

fn child(buf: &mut Vec<u8>, field: &Field<'_>) -> usize {
    match field {
        Field::Str(s) => {
            pad(buf, 4);
            let at = buf.len();
            buf.extend_from_slice(&len32(s.len()));
            buf.extend_from_slice(s.as_bytes());
            buf.push(0);
            at
        }
        Field::Table(t) => table(buf, t),
        Field::Tables(ts) => {
            pad(buf, 4);
            let at = buf.len();
            buf.extend_from_slice(&len32(ts.len()));
            let first = buf.len();
            buf.extend(std::iter::repeat_n(0, 4 * ts.len()));
            for (i, t) in ts.iter().enumerate() {
                let target = table(buf, t);
                patch(buf, first + 4 * i, target);
            }
            at
        }
        Field::Structs(count, bytes) => {
            // The elements, after the 4-byte length, start at a multiple of 8.
            while buf.len() % 8 != 4 {
                buf.push(0);
            }
            let at = buf.len();
            buf.extend_from_slice(&len32(*count));
            buf.extend_from_slice(bytes);
            at
        }
        Field::U8(_) | Field::Bool(_) | Field::I16(_) | Field::I32(_) | Field::I64(_) => {
            unreachable!("scalars are written inline")
        }
    }
}

fn len32(n: usize) -> [u8; 4] {
    u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes()
}
