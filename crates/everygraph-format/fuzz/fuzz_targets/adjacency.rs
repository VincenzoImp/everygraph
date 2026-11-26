// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The adjacency reader on untrusted bytes: it must never panic or read out of bounds,
//! and every decoder must return exactly what the scalar decoder returns.
//!
//! The first byte picks the mode:
//! - even: the rest is raw input, a little-endian `u16` length of the index part, the
//!   index part, and the data part;
//! - odd: the rest describes a small CSR (lists cut from the bytes), which is encoded
//!   and then damaged — truncated, or with bytes overwritten — as the input's tail says.
//!   This reaches records that decode deep before they go wrong, which raw input rarely
//!   does.

#![no_main]

use everygraph_core::NodeId;
use everygraph_format::adjacency::{Adjacency, AdjacencyIndex, Batch, Decoder, encode};
use libfuzzer_sys::fuzz_target;

type Outcome = Result<Vec<u32>, everygraph_format::adjacency::DecodeError>;

fn outcomes(adj: Adjacency<'_>) -> Vec<Outcome> {
    let index = adj.index();
    let mut out = Vec::new();
    let mut buf = Vec::new();
    for v in 0..index.nodes().min(4096) {
        let id = NodeId::new(v);
        out.push(adj.neighbors(id, &mut buf).map(<[u32]>::to_vec));
        out.push(adj.neighbors_prefix(id, 5, &mut buf).map(<[u32]>::to_vec));
    }
    let mut batch = Batch::default();
    for b in 0..index.blocks().min(64) {
        let nodes = index.block_nodes(b);
        let mut at = nodes.start;
        while at < nodes.end {
            match adj.decode_nodes(at..nodes.end, 7, &mut batch) {
                Ok(next) => {
                    out.push(Ok(batch.targets().to_vec()));
                    at = next;
                }
                Err(e) => {
                    out.push(Err(e));
                    break;
                }
            }
        }
    }
    if index.entries() <= 1 << 20 {
        let mut offsets = vec![0; index.nodes() as usize + 1];
        let mut targets = vec![0; index.entries() as usize];
        out.push(adj.decode_all(&mut offsets, &mut targets).map(|()| targets));
    }
    out
}

fn check(index: &[u8], data: &[u8]) {
    let Ok(index) = AdjacencyIndex::parse(index) else {
        return;
    };
    // Bound the work per input: the header may claim many nodes in a short index only
    // if the index really is that long.
    if index.nodes() > 1 << 16 {
        return;
    }
    let Ok(adj) = Adjacency::new(&index, data) else {
        return;
    };
    let reference = outcomes(adj.with_decoder(Decoder::SCALAR));
    for decoder in Decoder::available() {
        assert_eq!(outcomes(adj.with_decoder(decoder)), reference, "{}", decoder.name());
    }
    apis_agree(adj);
}

/// A block, or the whole graph, decodes only if each of its lists decodes on its own,
/// and then to the same lists.
fn apis_agree(adj: Adjacency<'_>) {
    let index = adj.index();
    let mut buf = Vec::new();
    let lists: Vec<Outcome> = (0..index.nodes())
        .map(|v| adj.neighbors(NodeId::new(v), &mut buf).map(<[u32]>::to_vec))
        .collect();
    let mut batch = Batch::default();
    for b in 0..index.blocks() {
        let nodes = index.block_nodes(b);
        let own = &lists[nodes.start as usize..nodes.end as usize];
        let got = adj.decode_nodes(nodes, usize::MAX, &mut batch);
        assert_eq!(got.is_ok(), own.iter().all(Result::is_ok), "block {b}");
        for ((_, l), want) in batch.iter().zip(own) {
            assert_eq!(Ok(l.to_vec()), *want, "block {b}");
        }
    }
    if index.entries() <= 1 << 20 {
        let mut offsets = vec![0; index.nodes() as usize + 1];
        let mut targets = vec![0; index.entries() as usize];
        let all = adj.decode_all(&mut offsets, &mut targets);
        assert_eq!(all.is_ok(), lists.iter().all(Result::is_ok));
        if all.is_ok() {
            let want: Vec<u32> = lists.into_iter().flat_map(Result::unwrap).collect();
            assert_eq!(targets, want);
        }
    }
}

fn structured(bytes: &[u8]) {
    let (lists, damage) = bytes.split_at(bytes.len() / 2);
    let mut offsets = vec![0u64];
    let mut targets: Vec<u32> = Vec::new();
    for chunk in lists.chunks(9) {
        let mut x = u32::from(chunk[0]) << (chunk[0] % 25);
        let mut list = Vec::new();
        for &b in &chunk[1..] {
            x = x.wrapping_add(1 + (u32::from(b) << (b % 29)));
            if list.last().is_some_and(|&l| x <= l) {
                break;
            }
            list.push(x);
        }
        targets.extend(&list);
        offsets.push(targets.len() as u64);
    }
    let Ok(mut e) = encode(&offsets, &targets) else {
        return;
    };
    let mut d = damage.chunks_exact(3);
    if let Some(first) = d.next() {
        if first[0] & 1 == 1 && !e.data.is_empty() {
            let cut = usize::from(u16::from_le_bytes([first[1], first[2]])) % e.data.len();
            e.data.truncate(cut);
        }
    }
    for hit in d {
        let part = if hit[0] & 1 == 0 { &mut e.data } else { &mut e.index };
        if !part.is_empty() {
            let i = usize::from(hit[0] >> 1) * 257 % part.len();
            part[i] = hit[1] ^ hit[2];
        }
    }
    check(&e.index, &e.data);
}

fuzz_target!(|bytes: &[u8]| {
    let Some((&mode, rest)) = bytes.split_first() else {
        return;
    };
    if mode & 1 == 1 {
        structured(rest);
    } else if rest.len() >= 2 {
        let len = usize::from(u16::from_le_bytes([rest[0], rest[1]])).min(rest.len() - 2);
        let (index, data) = rest[2..].split_at(len);
        check(index, data);
    }
});
