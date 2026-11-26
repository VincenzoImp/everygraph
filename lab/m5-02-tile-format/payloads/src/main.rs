//! m5-02 (#68): the content of every lab tile of a map, written as the `.egmap` tile payload
//! (everygraph-map), as Arrow IPC and as MapLibre Tile (mlt-core), and the bytes of each, raw and
//! gzip. Load-independent counts only.
//!
//!     payloads MAP.pmtiles LAYOUT_DIR OUT.json
//!
//! The lab tiles (lab/e-map/gmap/src/tiles.rs, payload GMT1) hold bubbles and nodes chosen per
//! tile. Each becomes a `Tile` of docs/spec/egmap.md: its regions, the regions of its nodes, and
//! their ancestors up to the root (the spec's tile holds every ancestor); `zmin` from the zoom
//! rule with the lab's thresholds (3 px for a region, 1.2 px for a node, lab/e-map/gmap/src/tiles.rs
//! l. 10 and 191), capped at the tile's zoom; rank among siblings by node count; colour of the top
//! region modulo 5; node names the original ids in decimal. No labels (the lab has none), no
//! edges (the spec's tiles hold none). The three payloads hold exactly these fields.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::sync::Arc;

use arrow_array::{ArrayRef, Float32Array, RecordBatch, StringArray, UInt8Array, UInt32Array, UInt64Array};
use arrow_ipc::writer::StreamWriter;
use everygraph_map::egmap::{Node, Region, RegionKind, Tile, ZoomRule};
use flate2::Compression;
use mlt_core::encoder::{EncoderConfig, Network, Profile};
use mlt_core::geo_types::Coord;
use mlt_core::{GeometryType, LayerWriter, PropKind};
use rayon::prelude::*;

const F_AGG: u8 = 8;
const F_SUBDIV: u8 = 16;
const RULE: ZoomRule = ZoomRule { region_px: 3.0, open_px: 3.0, node_px: 1.2, label_off_px: 1e9 };

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}
fn f32_at(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn gz(b: &[u8]) -> usize {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), Compression::new(6));
    e.write_all(b).unwrap();
    e.finish().unwrap().len()
}
fn gunzip(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(b).read_to_end(&mut out).unwrap();
    out
}
fn varint(b: &[u8], p: &mut usize) -> u64 {
    let mut r = 0;
    let mut s = 0;
    loop {
        let x = b[*p];
        *p += 1;
        r |= u64::from(x & 0x7F) << s;
        s += 7;
        if x < 0x80 {
            return r;
        }
    }
}
fn directory(raw: &[u8]) -> Vec<(u64, u64, u64, u64)> {
    let b = gunzip(raw);
    let mut p = 0;
    let n = varint(&b, &mut p) as usize;
    let mut ids = vec![0u64; n];
    let mut last = 0;
    for id in ids.iter_mut() {
        last += varint(&b, &mut p);
        *id = last;
    }
    let runs: Vec<u64> = (0..n).map(|_| varint(&b, &mut p)).collect();
    let lens: Vec<u64> = (0..n).map(|_| varint(&b, &mut p)).collect();
    let mut offs = vec![0u64; n];
    for i in 0..n {
        let o = varint(&b, &mut p);
        offs[i] = if o == 0 && i > 0 { offs[i - 1] + lens[i - 1] } else { o - 1 };
    }
    (0..n).map(|i| (ids[i], runs[i], lens[i], offs[i])).collect()
}

#[derive(Clone, Copy)]
struct TNode {
    parent: u32,
    start: u32,
    end: u32,
    comm: u32,
    depth: u8,
    flags: u8,
    x: f32,
    y: f32,
    r: f32,
    internal: u64,
    volume: u64,
}

struct Sizes {
    z: u8,
    raw: [usize; 4],
    gz: [usize; 4],
    added: usize,
}

fn mlt(t: &Tile, profile: Profile) -> Vec<u8> {
    let cfg = EncoderConfig::from_profile(profile);
    let scale = f64::from(1u32 << t.z);
    let point = |x: f32, y: f32| Coord {
        x: ((f64::from(x) * scale - f64::from(t.x)) * 4096.0).round() as i32,
        y: ((f64::from(y) * scale - f64::from(t.y)) * 4096.0).round() as i32,
    };
    let mut out = Vec::new();
    if !t.regions.is_empty() {
        let mut w = LayerWriter::new("regions", 4096).unwrap();
        let k: Vec<_> = [
            ("parent", PropKind::U32),
            ("x", PropKind::F32),
            ("y", PropKind::F32),
            ("r", PropKind::F32),
            ("nodes", PropKind::U64),
            ("internal", PropKind::U64),
            ("volume", PropKind::U64),
            ("communities", PropKind::U32),
            ("rank", PropKind::U32),
            ("partner", PropKind::U32),
            ("kind", PropKind::U8),
            ("depth", PropKind::U8),
            ("colour", PropKind::U8),
            ("zmin", PropKind::U8),
        ]
        .into_iter()
        .map(|(n, kind)| w.add_property(n, kind).unwrap())
        .collect();
        for g in &t.regions {
            let mut f = w.feature(GeometryType::Point);
            f.id(Some(u64::from(g.id)));
            f.property(k[0], g.parent).unwrap();
            f.property(k[1], g.x).unwrap();
            f.property(k[2], g.y).unwrap();
            f.property(k[3], g.r).unwrap();
            f.property(k[4], g.nodes).unwrap();
            f.property(k[5], g.internal).unwrap();
            f.property(k[6], g.volume).unwrap();
            f.property(k[7], g.communities).unwrap();
            f.property(k[8], g.rank).unwrap();
            f.property(k[9], g.partner).unwrap();
            f.property(k[10], g.kind as u8).unwrap();
            f.property(k[11], g.depth).unwrap();
            f.property(k[12], g.colour).unwrap();
            f.property(k[13], g.zmin).unwrap();
            f.points([point(g.x, g.y)]).unwrap();
            f.finish().unwrap();
        }
        out.extend(w.encode(cfg).unwrap());
    }
    if !t.nodes.is_empty() {
        let mut w = LayerWriter::new("nodes", 4096).unwrap();
        let k: Vec<_> = [
            ("region", PropKind::U32),
            ("x", PropKind::F32),
            ("y", PropKind::F32),
            ("r", PropKind::F32),
            ("name", PropKind::Str),
            ("degree", PropKind::U64),
            ("zmin", PropKind::U8),
        ]
        .into_iter()
        .map(|(n, kind)| w.add_property(n, kind).unwrap())
        .collect();
        for v in &t.nodes {
            let mut f = w.feature(GeometryType::Point);
            f.id(Some(u64::from(v.id)));
            f.property(k[0], v.region).unwrap();
            f.property(k[1], v.x).unwrap();
            f.property(k[2], v.y).unwrap();
            f.property(k[3], v.r).unwrap();
            f.property(k[4], t.strings[v.name as usize].as_str()).unwrap();
            f.property(k[5], v.degree).unwrap();
            f.property(k[6], v.zmin).unwrap();
            f.points([point(v.x, v.y)]).unwrap();
            f.finish().unwrap();
        }
        out.extend(w.encode(cfg).unwrap());
    }
    out
}

fn arrow(t: &Tile) -> Vec<u8> {
    let mut out = Vec::new();
    let mut stream = |cols: Vec<(&str, ArrayRef)>| {
        let batch = RecordBatch::try_from_iter(cols).unwrap();
        let mut w = StreamWriter::try_new(&mut out, &batch.schema()).unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();
    };
    let r = &t.regions;
    if !r.is_empty() {
        let u32s = |f: fn(&Region) -> u32| Arc::new(UInt32Array::from_iter_values(r.iter().map(f))) as ArrayRef;
        let u64s = |f: fn(&Region) -> u64| Arc::new(UInt64Array::from_iter_values(r.iter().map(f))) as ArrayRef;
        let f32s = |f: fn(&Region) -> f32| Arc::new(Float32Array::from_iter_values(r.iter().map(f))) as ArrayRef;
        let u8s = |f: fn(&Region) -> u8| Arc::new(UInt8Array::from_iter_values(r.iter().map(f))) as ArrayRef;
        stream(vec![
            ("id", u32s(|g| g.id)),
            ("parent", u32s(|g| g.parent)),
            ("x", f32s(|g| g.x)),
            ("y", f32s(|g| g.y)),
            ("r", f32s(|g| g.r)),
            ("nodes", u64s(|g| g.nodes)),
            ("internal", u64s(|g| g.internal)),
            ("volume", u64s(|g| g.volume)),
            ("communities", u32s(|g| g.communities)),
            ("rank", u32s(|g| g.rank)),
            ("partner", u32s(|g| g.partner)),
            ("kind", u8s(|g| g.kind as u8)),
            ("depth", u8s(|g| g.depth)),
            ("colour", u8s(|g| g.colour)),
            ("zmin", u8s(|g| g.zmin)),
        ]);
    }
    let n = &t.nodes;
    if !n.is_empty() {
        let u32s = |f: fn(&Node) -> u32| Arc::new(UInt32Array::from_iter_values(n.iter().map(f))) as ArrayRef;
        let f32s = |f: fn(&Node) -> f32| Arc::new(Float32Array::from_iter_values(n.iter().map(f))) as ArrayRef;
        let names = Arc::new(StringArray::from_iter_values(n.iter().map(|v| t.strings[v.name as usize].as_str()))) as ArrayRef;
        stream(vec![
            ("id", u32s(|v| v.id)),
            ("region", u32s(|v| v.region)),
            ("x", f32s(|v| v.x)),
            ("y", f32s(|v| v.y)),
            ("r", f32s(|v| v.r)),
            ("name", names),
            ("degree", Arc::new(UInt64Array::from_iter_values(n.iter().map(|v| v.degree))) as ArrayRef),
            ("zmin", Arc::new(UInt8Array::from_iter_values(n.iter().map(|v| v.zmin))) as ArrayRef),
        ]);
    }
    out
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (map, layout, out) = (&a[1], &a[2], &a[3]);
    let f = std::fs::read(map).unwrap();
    let (root_off, root_len) = (u64_at(&f, 8) as usize, u64_at(&f, 16) as usize);
    let (leaf_off, data_off) = (u64_at(&f, 40) as usize, u64_at(&f, 56) as usize);
    let zmax = f[101];
    let mut entries = Vec::new();
    for e in directory(&f[root_off..root_off + root_len]) {
        if e.1 == 0 {
            let s = leaf_off + e.3 as usize;
            entries.extend(directory(&f[s..s + e.2 as usize]));
        } else {
            entries.push(e);
        }
    }
    let raw = std::fs::read(format!("{layout}/tnodes.bin")).unwrap();
    let tn: Vec<TNode> = raw
        .chunks_exact(56)
        .map(|b| TNode {
            parent: u32_at(b, 0),
            start: u32_at(b, 4),
            end: u32_at(b, 8),
            comm: u32_at(b, 12),
            depth: b[17],
            flags: b[18],
            x: f32_at(b, 20),
            y: f32_at(b, 24),
            r: f32_at(b, 28),
            internal: u64_at(b, 40),
            volume: u64_at(b, 48),
        })
        .collect();
    let nt = tn.len();
    // rank among siblings, top region, zmin by the rule, top-down by depth
    let mut kids: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (i, t) in tn.iter().enumerate().skip(1) {
        kids.entry(t.parent).or_default().push(i as u32);
    }
    let mut rank = vec![0u32; nt];
    for ks in kids.values_mut() {
        ks.sort_by_key(|&c| (std::cmp::Reverse(tn[c as usize].end - tn[c as usize].start), c));
        for (i, &c) in ks.iter().enumerate() {
            rank[c as usize] = i as u32 + 1;
        }
    }
    let mut order: Vec<usize> = (1..nt).collect();
    order.sort_by_key(|&i| (tn[i].depth, i));
    // The spec numbers regions breadth first (a parent before its children); the lab does not.
    let mut bfs = vec![0u32; nt];
    for (k, &i) in order.iter().enumerate() {
        bfs[i] = k as u32 + 1;
    }
    let mut top = vec![0u32; nt];
    let mut zmin = vec![0u8; nt];
    for &i in &order {
        let p = tn[i].parent as usize;
        top[i] = if p == 0 { i as u32 } else { top[p] };
        let parent = (p != 0).then(|| (tn[p].r, zmin[p]));
        zmin[i] = RULE.region_zmin(tn[i].r, parent, zmax);
    }
    let sizes: Vec<Result<Sizes, String>> = entries
        .par_iter()
        .map(|&(_, _, len, off)| {
            let s = data_off + off as usize;
            let b = gunzip(&f[s..s + len as usize]);
            assert_eq!(&b[..4], b"GMT1");
            let z = b[4];
            let (tx, ty, nctx, nbub, nnode) = (u32_at(&b, 8), u32_at(&b, 12), u32_at(&b, 16), u32_at(&b, 20), u32_at(&b, 24));
            let (om_b, om_n, om_bn) = (u32_at(&b, 32), u32_at(&b, 36), u64_at(&b, 40));
            let mut ids: BTreeSet<u32> = BTreeSet::new();
            let nb = (nctx + nbub) as usize;
            for i in 0..nb {
                ids.insert(u32_at(&b, 72 + 48 * i));
            }
            let lab_regions = ids.len();
            let at = 72 + 48 * nb;
            let mut nodes = Vec::new();
            let mut strings = Vec::new();
            for j in 0..nnode as usize {
                let p = at + 36 * j;
                let lp = u32_at(&b, p + 20);
                ids.insert(lp);
                strings.push(u64_at(&b, p + 28).to_string());
                let rz = zmin[lp as usize].min(z);
                nodes.push(Node {
                    id: u32_at(&b, p),
                    region: bfs[lp as usize],
                    x: f32_at(&b, p + 4),
                    y: f32_at(&b, p + 8),
                    r: f32_at(&b, p + 12),
                    name: j as u32,
                    degree: u64::from(u32_at(&b, p + 16)),
                    zmin: RULE.node_zmin(f32_at(&b, p + 12), (tn[lp as usize].r, zmin[lp as usize]), zmax).min(z).max(rz),
                });
            }
            let mut all = ids.clone();
            for &i in &ids {
                let mut p = tn[i as usize].parent;
                while p != 0 {
                    all.insert(p);
                    p = tn[p as usize].parent;
                }
            }
            let regions: Vec<Region> = all
                .iter()
                .map(|&i| {
                    let t = &tn[i as usize];
                    let agg = t.flags & F_AGG != 0;
                    Region {
                        id: bfs[i as usize],
                        parent: bfs[t.parent as usize],
                        x: t.x,
                        y: t.y,
                        r: t.r,
                        nodes: u64::from(t.end - t.start),
                        internal: t.internal,
                        volume: t.volume.max(2 * t.internal),
                        communities: if agg { t.comm.max(1) } else { 0 },
                        rank: rank[i as usize],
                        partner: 0,
                        kind: if agg { RegionKind::Aggregate } else if t.flags & F_SUBDIV != 0 { RegionKind::Subdivision } else { RegionKind::Community },
                        depth: t.depth,
                        colour: (top[i as usize] % 5) as u8,
                        zmin: zmin[i as usize].min(z),
                    }
                })
                .collect();
            let mut regions = regions;
            regions.sort_by_key(|g| g.id);
            nodes.sort_by_key(|v| v.id);
            let mut tile = Tile::new(z, tx, ty);
            tile.omitted.regions = u64::from(om_b);
            tile.omitted.region_nodes = om_bn;
            tile.omitted.nodes = u64::from(om_n);
            tile.regions = regions;
            tile.nodes = nodes;
            tile.strings = strings;
            let added = tile.regions.len() - lab_regions;
            let eg = tile.encode().map_err(|e| e.to_string())?;
            let back = Tile::decode(&eg).map_err(|e| e.to_string())?;
            assert_eq!(back, tile);
            let ar = arrow(&tile);
            let web = mlt(&tile, Profile::Web(Network::Cellular4G));
            let min = mlt(&tile, Profile::MinSize);
            Ok(Sizes {
                z,
                raw: [eg.len(), ar.len(), web.len(), min.len()],
                gz: [gz(&eg), gz(&ar), gz(&web), gz(&min)],
                added,
            })
        })
        .collect();
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    let ok: Vec<&Sizes> = sizes
        .iter()
        .filter_map(|s| match s {
            Ok(s) => Some(s),
            Err(e) => {
                *errors.entry(e.chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect()).or_default() += 1;
                None
            }
        })
        .collect();
    let names = ["egmap", "arrow_ipc", "mlt_web_4g", "mlt_min_size"];
    let stat = |k: usize, gzip: bool| {
        let mut v: Vec<usize> = ok.iter().map(|s| if gzip { s.gz[k] } else { s.raw[k] }).collect();
        v.sort_unstable();
        let total: usize = v.iter().sum();
        format!(
            "{{\"total\":{total},\"median\":{},\"p99\":{},\"max\":{}}}",
            v[v.len() / 2],
            v[v.len() * 99 / 100],
            v[v.len() - 1]
        )
    };
    let mut js = format!(
        "{{\"map\":\"{map}\",\"tiles\":{},\"tiles_encoded\":{},\"refused\":{:?},\"regions_added_as_ancestors\":{},\"payloads\":{{",
        entries.len(),
        ok.len(),
        errors,
        ok.iter().map(|s| s.added).sum::<usize>()
    );
    for (k, n) in names.iter().enumerate() {
        js += &format!("{}\"{n}\":{{\"raw\":{},\"gzip\":{}}}", if k > 0 { "," } else { "" }, stat(k, false), stat(k, true));
    }
    js += "},\"gzip_by_zoom\":[";
    for z in 0..=zmax {
        let t: Vec<usize> = (0..4).map(|k| ok.iter().filter(|s| s.z == z).map(|s| s.gz[k]).sum()).collect();
        js += &format!("{}{{\"z\":{z},\"egmap\":{},\"arrow_ipc\":{},\"mlt_web_4g\":{},\"mlt_min_size\":{}}}", if z > 0 { "," } else { "" }, t[0], t[1], t[2], t[3]);
    }
    js += "]}";
    std::fs::write(out, &js).unwrap();
    println!("{js}");
}
