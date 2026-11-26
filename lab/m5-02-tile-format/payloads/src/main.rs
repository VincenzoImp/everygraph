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
use everygraph_core::{Memory, MemorySource};
use everygraph_map::egmap::{MapInfo, MapReader, MapWriter, Metadata, Node, Region, RegionKind, Tile, Values, ZoomRule};
use flate2::Compression;
use mlt_core::encoder::{EncoderConfig, Network, Profile};
use mlt_core::geo_types::Coord;
use mlt_core::{GeometryType, LayerWriter, PropKind};
use rayon::prelude::*;

const F_AGG: u8 = 8;
const F_SUBDIV: u8 = 16;
const RULE: ZoomRule = ZoomRule { region_px: 3.0, open_px: 3.0, node_px: 1.2, label_off_px: 1e9 };

fn size(r: f32, px: f32) -> u8 {
    (0..=16).find(|&z| f64::from(r) * 256.0 * f64::from(1u32 << z) >= f64::from(px)).unwrap_or(17)
}
fn region_zmin(r: f32, parent: Option<(f32, u8)>, zmax: u8) -> u8 {
    let after = parent.map_or(0, |(pr, pz)| pz.max(size(pr, RULE.open_px)));
    size(r, RULE.region_px).max(after).min(zmax)
}
fn node_zmin(r: f32, region: (f32, u8), zmax: u8) -> u8 {
    size(r, RULE.node_px).max(region.1.max(size(region.0, RULE.open_px))).min(zmax)
}
/// Every corner of the tile inside the disc (docs/spec/egmap.md § 3.5).
fn covers(x: f32, y: f32, r: f32, z: u8, tx: u32, ty: u32) -> bool {
    let w = 1.0 / f64::from(1u32 << z);
    let (x0, x1, y0, y1) = (f64::from(tx) * w, f64::from(tx + 1) * w, f64::from(ty) * w, f64::from(ty + 1) * w);
    let (cx, cy) = (f64::from(x), f64::from(y));
    let (ex, ey) = ((cx - x0).max(x1 - cx), (cy - y0).max(y1 - cy));
    ex * ex + ey * ey <= f64::from(r) * f64::from(r)
}
fn kind_of(t: &TNode) -> RegionKind {
    if t.flags & F_AGG != 0 {
        RegionKind::Aggregate
    } else if t.flags & F_SUBDIV != 0 {
        RegionKind::Subcommunity
    } else {
        RegionKind::Community
    }
}

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
            ("x", PropKind::F32),
            ("y", PropKind::F32),
            ("r", PropKind::F32),
            ("nodes", PropKind::U64),
            ("kind", PropKind::U8),
            ("colour", PropKind::U8),
            ("zmin", PropKind::U8),
        ]
        .into_iter()
        .map(|(n, kind)| w.add_property(n, kind).unwrap())
        .collect();
        for g in &t.regions {
            let mut f = w.feature(GeometryType::Point);
            f.id(Some(u64::from(g.id)));
            f.property(k[0], g.x).unwrap();
            f.property(k[1], g.y).unwrap();
            f.property(k[2], g.r).unwrap();
            f.property(k[3], g.nodes).unwrap();
            f.property(k[4], g.kind as u8).unwrap();
            f.property(k[5], g.colour).unwrap();
            f.property(k[6], g.zmin).unwrap();
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
            ("colour", PropKind::U8),
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
            f.property(k[6], v.colour).unwrap();
            f.property(k[7], v.zmin).unwrap();
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
            ("x", f32s(|g| g.x)),
            ("y", f32s(|g| g.y)),
            ("r", f32s(|g| g.r)),
            ("nodes", u64s(|g| g.nodes)),
            ("kind", u8s(|g| g.kind as u8)),
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
            ("colour", Arc::new(UInt8Array::from_iter_values(n.iter().map(|v| v.colour))) as ArrayRef),
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
        zmin[i] = region_zmin(tn[i].r, parent, zmax);
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
            let at = 72 + 48 * nb;
            let mut nodes = Vec::new();
            let mut names = Vec::new();
            for j in 0..nnode as usize {
                let p = at + 36 * j;
                let lp = u32_at(&b, p + 20) as usize;
                names.push(u64_at(&b, p + 28).to_string());
                let rz = zmin[lp].min(z);
                nodes.push(Node {
                    id: u32_at(&b, p),
                    region: bfs[lp],
                    x: f32_at(&b, p + 4),
                    y: f32_at(&b, p + 8),
                    r: f32_at(&b, p + 12),
                    name: j as u32,
                    degree: u64::from(u32_at(&b, p + 16)),
                    colour: (top[lp] % 5) as u8,
                    zmin: node_zmin(f32_at(&b, p + 12), (tn[lp].r, zmin[lp]), zmax).min(z).max(rz),
                });
            }
            // The spec places a region only where its disc does not cover the tile.
            let before = ids.len();
            ids.retain(|&i| {
                let t = &tn[i as usize];
                !covers(t.x, t.y, t.r, z, tx, ty)
            });
            let dropped = before - ids.len();
            let mut regions: Vec<Region> = ids
                .iter()
                .map(|&i| {
                    let t = &tn[i as usize];
                    Region {
                        id: bfs[i as usize],
                        x: t.x,
                        y: t.y,
                        r: t.r,
                        nodes: u64::from(t.end - t.start),
                        kind: kind_of(t),
                        colour: (top[i as usize] % 5) as u8,
                        zmin: zmin[i as usize].min(z),
                    }
                })
                .collect();
            regions.sort_by_key(|g| g.id);
            nodes.sort_by_key(|v| v.id);
            let mut sorted = names.clone();
            sorted.sort();
            for v in &mut nodes {
                v.name = sorted.binary_search(&names[v.name as usize]).unwrap() as u32;
            }
            let mut tile = Tile::new(z, tx, ty);
            tile.omitted.regions = u64::from(om_b);
            tile.omitted.region_nodes = om_bn;
            tile.omitted.nodes = u64::from(om_n);
            tile.regions = regions;
            tile.nodes = nodes;
            tile.strings = sorted;
            let added = dropped;
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
        "{{\"map\":\"{map}\",\"tiles\":{},\"tiles_encoded\":{},\"refused\":{:?},\"regions_dropped_covering_the_tile\":{},\"payloads\":{{",
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
    // The region table (§ 5.3) in chunks of 65,536 values, through the product writer.
    let mut by_bfs = vec![0usize; nt];
    for (i, &k) in bfs.iter().enumerate() {
        by_bfs[k as usize] = i;
    }
    let col = |f: &dyn Fn(&TNode) -> u64| -> Vec<u64> { by_bfs.iter().map(|&i| f(&tn[i])).collect() };
    let parent: Vec<u32> = by_bfs.iter().map(|&i| if i == 0 { 0 } else { bfs[tn[i].parent as usize] }).collect();
    let kind: Vec<u8> = by_bfs.iter().map(|&i| if i == 0 { RegionKind::Root as u8 } else { kind_of(&tn[i]) as u8 }).collect();
    let nodes = col(&|t| u64::from(t.end - t.start));
    let internal = col(&|t| t.internal);
    let volume = col(&|t| t.volume.max(2 * t.internal));
    let communities: Vec<u32> = by_bfs.iter().map(|&i| if i != 0 && tn[i].flags & F_AGG != 0 { tn[i].comm.max(1) } else { 0 }).collect();
    let partner = vec![0u32; nt];
    let info = MapInfo { nodes: nodes[0], regions: nt as u64, edges: 0, summary: 0 };
    let mut w = MapWriter::create(MemorySource::new(Vec::new()), &Memory::unlimited(), info).unwrap();
    for (name, values) in [
        ("regions.parent", Values::U32(&parent)),
        ("regions.kind", Values::U8(&kind)),
        ("regions.nodes", Values::U64(&nodes)),
        ("regions.internal", Values::U64(&internal)),
        ("regions.volume", Values::U64(&volume)),
        ("regions.communities", Values::U32(&communities)),
        ("regions.partner", Values::U32(&partner)),
    ] {
        w.begin_array(name, values.element(), 65_536).unwrap();
        w.write_values(values).unwrap();
        w.end_array().unwrap();
    }
    let meta = Metadata { name: String::new(), description: String::new(), attribution: String::new(), zoom: RULE };
    let file = w.finish(&meta).unwrap().into_inner();
    let reader = MapReader::open(MemorySource::new(file)).unwrap();
    js += "],\"region_table\":{\"regions\":";
    js += &nt.to_string();
    for s in reader.sections() {
        js += &format!(",\"{}\":{}", s.name, s.len);
    }
    js += &format!(",\"total\":{}}}}}", reader.sections().iter().map(|s| s.len).sum::<u64>());
    std::fs::write(out, &js).unwrap();
    println!("{js}");
}
