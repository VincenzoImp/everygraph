//! Semantic-zoom tile pyramid ("glab-map-tile-v0" payload) in a real PMTiles v3 container.
use crate::common::*;
use flate2::{write::GzEncoder, Compression};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::io::Write;
use std::time::Instant;

const NONE: u32 = u32::MAX;
const BUB_MIN_PX: f32 = 3.0;
const BUB_MAX_PX: f32 = 384.0;

fn gz(data: &[u8]) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), Compression::new(6));
    e.write_all(data).unwrap();
    e.finish().unwrap()
}
pub fn zxy_to_id(z: u32, x: u32, y: u32) -> u64 {
    let acc: u64 = ((1u64 << (2 * z)) - 1) / 3;
    let n: i64 = 1 << z;
    let (mut x, mut y) = (x as i64, y as i64);
    let mut d: u64 = 0;
    let mut s: i64 = n / 2;
    while s > 0 {
        let rx = ((x & s) > 0) as i64;
        let ry = ((y & s) > 0) as i64;
        d += (s * s) as u64 * ((3 * rx) ^ ry) as u64;
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    acc + d
}
fn varint(v: &mut Vec<u8>, mut x: u64) {
    while x >= 0x80 {
        v.push((x as u8) | 0x80);
        x >>= 7;
    }
    v.push(x as u8);
}
#[derive(Clone, Copy)]
struct Entry {
    id: u64,
    off: u64,
    len: u32,
    run: u32,
}
fn ser_dir(es: &[Entry]) -> Vec<u8> {
    let mut v = vec![];
    varint(&mut v, es.len() as u64);
    let mut last = 0;
    for e in es {
        varint(&mut v, e.id - last);
        last = e.id;
    }
    for e in es {
        varint(&mut v, e.run as u64);
    }
    for e in es {
        varint(&mut v, e.len as u64);
    }
    for (i, e) in es.iter().enumerate() {
        if i > 0 && e.off == es[i - 1].off + es[i - 1].len as u64 {
            varint(&mut v, 0);
        } else {
            varint(&mut v, e.off + 1);
        }
    }
    gz(&v)
}
/// returns (root_dir_bytes, leaf_dirs_bytes)
fn build_dirs(es: &[Entry]) -> (Vec<u8>, Vec<u8>) {
    let root = ser_dir(es);
    if root.len() <= 16384 - 127 {
        return (root, vec![]);
    }
    let mut leaf_size = 4096usize;
    loop {
        let mut leaves = vec![];
        let mut roots = vec![];
        for ch in es.chunks(leaf_size) {
            let b = ser_dir(ch);
            roots.push(Entry { id: ch[0].id, off: leaves.len() as u64, len: b.len() as u32, run: 0 });
            leaves.extend_from_slice(&b);
        }
        let root = ser_dir(&roots);
        if root.len() <= 16384 - 127 {
            return (root, leaves);
        }
        leaf_size *= 2;
    }
}

struct TileOut {
    id: u64,
    gz: Vec<u8>,
    raw: u32,
    n_bub: u32,
    n_node: u32,
    n_edge: u32,
    w_total: u64,
    w_shown: u64,
}

pub fn run() {
    let a = args();
    let graph = args_s(&a, "graph");
    let lay = args_s(&a, "layout");
    let out = args_s(&a, "out");
    let bub_budget = argf(&a, "bubbles", 300.0) as usize;
    let node_budget = argf(&a, "nodes", 400.0) as usize;
    let edge_budget = argf(&a, "edges", 600.0) as usize;
    let t_all = Instant::now();
    let csr = Csr::load(&graph);
    let n = csr.n;
    let ids: Vec<u64> = read_vec(&format!("{graph}/ids.u64"));
    let pos: Vec<[f32; 2]> = read_vec(&format!("{lay}/pos.f32"));
    let order: Vec<u32> = read_vec(&format!("{lay}/order.u32"));
    let tn: Vec<TNode> = read_vec(&format!("{lay}/tnodes.bin"));
    let lp: Vec<u32> = read_vec(&format!("{lay}/leafparent.u32"));
    let top: Vec<u32> = read_vec(&format!("{lay}/top.u32"));
    let ntn = tn.len();
    let t_load = t_all.elapsed().as_secs_f64();
    let t_build = Instant::now();
    // max zoom: 10th percentile node radius should be >= 4 px
    let mut nrs: Vec<f32> = tn.iter().filter(|t| t.flags & F_LEAFP != 0 && t.flags & F_DEAD == 0).map(|t| t.node_r).collect();
    nrs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let nr10 = nrs[nrs.len() / 10];
    let zauto = ((4.0 / (256.0 * nr10)).log2().ceil() as i32).clamp(2, 14) as u32;
    let zmax = argf(&a, "zmax", zauto as f64) as u32;
    let mut all: Vec<TileOut> = vec![];
    let mut zstats = vec![];
    let mut node_zmin = vec![zmax as u8; n];
    for z in 0..=zmax {
        let tz = Instant::now();
        let nt = 1u32 << z;
        let scale = 256.0 * nt as f32;
        let tkey = |x: f32, y: f32| -> u32 {
            let tx = ((x * nt as f32) as u32).min(nt - 1);
            let ty = ((y * nt as f32) as u32).min(nt - 1);
            ty << 16 | tx
        };
        // 1. bubbles
        let mut cand: Vec<(u32, u32, u32)> = vec![];
        let mut ctx: Vec<u32> = vec![];
        for (ti, t) in tn.iter().enumerate() {
            if ti == 0 || t.flags & F_DEAD != 0 {
                continue;
            }
            let rp = t.r * scale;
            if rp > BUB_MAX_PX {
                ctx.push(ti as u32);
            } else if rp >= BUB_MIN_PX {
                cand.push((tkey(t.x, t.y), u32::MAX - (t.end - t.start), ti as u32));
            }
        }
        cand.par_sort_unstable();
        let mut disp = vec![false; ntn];
        let mut tiles: FxHashMap<u32, (Vec<u32>, Vec<u32>, u32, u64, u32)> = FxHashMap::default(); // bubbles, nodes, omitted bubbles, omitted bubble nodes, omitted nodes
        {
            let mut i = 0;
            while i < cand.len() {
                let mut j = i;
                while j < cand.len() && cand[j].0 == cand[i].0 {
                    j += 1;
                }
                let e = tiles.entry(cand[i].0).or_default();
                for (r, c) in cand[i..j].iter().enumerate() {
                    if r < bub_budget {
                        disp[c.2 as usize] = true;
                        e.0.push(c.2);
                    } else {
                        e.2 += 1;
                        e.3 += (u32::MAX - c.1) as u64;
                    }
                }
                i = j;
            }
        }
        // 2. nodes
        let mut ndisp = vec![false; n];
        let mut ncand: Vec<(u32, u32, u32)> = (0..n)
            .into_par_iter()
            .filter_map(|u| {
                let rp = tn[lp[u] as usize].node_r * scale;
                if z == zmax || rp >= 1.2 {
                    Some((tkey(pos[u][0], pos[u][1]), u32::MAX - csr.deg(u) as u32, u as u32))
                } else {
                    None
                }
            })
            .collect();
        ncand.par_sort_unstable();
        {
            let mut i = 0;
            while i < ncand.len() {
                let mut j = i;
                while j < ncand.len() && ncand[j].0 == ncand[i].0 {
                    j += 1;
                }
                let e = tiles.entry(ncand[i].0).or_default();
                for (r, c) in ncand[i..j].iter().enumerate() {
                    if z == zmax || r < node_budget {
                        ndisp[c.2 as usize] = true;
                        if node_zmin[c.2 as usize] as u32 > z {
                            node_zmin[c.2 as usize] = z as u8;
                        }
                        e.1.push(c.2);
                    } else {
                        e.4 += 1;
                    }
                }
                i = j;
            }
        }
        drop(ncand);
        // 3. representative entity of every node at this zoom
        let rep: Vec<u32> = (0..n)
            .into_par_iter()
            .map(|u| {
                if ndisp[u] {
                    return u as u32;
                }
                let mut t = lp[u] as usize;
                while t != 0 && !disp[t] {
                    t = tn[t].parent as usize;
                }
                if t == 0 { NONE } else { (n + t) as u32 }
            })
            .collect();
        // 4/5. per-tile aggregation + encoding
        let mut tl: Vec<(u32, (Vec<u32>, Vec<u32>, u32, u64, u32))> = tiles.into_iter().collect();
        tl.sort_by_key(|t| t.0);
        let scan = |members: &[u32], me: u32, local: u32, m: &mut FxHashMap<u64, u32>, wctx: &mut u64| {
            for &u in members {
                if rep[u as usize] != me {
                    continue;
                }
                for &v in csr.adj(u as usize) {
                    let rv = rep[v as usize];
                    if rv == me {
                        continue;
                    }
                    if rv == NONE {
                        *wctx += 1;
                    } else {
                        *m.entry((local as u64) << 32 | rv as u64).or_insert(0) += 1;
                    }
                }
            }
        };
        let mut outs: Vec<TileOut> = tl
            .par_iter()
            .map(|(key, (bubs, nodes, om_b, om_bn, om_n))| {
                let (tx, ty) = (key & 0xffff, key >> 16);
                let (x0, y0, w) = (tx as f32 / nt as f32, ty as f32 / nt as f32, 1.0 / nt as f32);
                // context bubbles intersecting this tile
                let myctx: Vec<u32> = ctx
                    .iter()
                    .copied()
                    .filter(|&t| {
                        let t = &tn[t as usize];
                        let cxn = t.x.clamp(x0, x0 + w);
                        let cyn = t.y.clamp(y0, y0 + w);
                        (t.x - cxn).powi(2) + (t.y - cyn).powi(2) <= t.r * t.r
                    })
                    .collect();
                let nctx = myctx.len() as u32;
                let mut m: FxHashMap<u64, u32> = FxHashMap::default();
                let mut wctx = 0u64;
                for (bi, &t) in bubs.iter().enumerate() {
                    let tt = &tn[t as usize];
                    let members = &order[tt.start as usize..tt.end as usize];
                    let me = (n + t as usize) as u32;
                    let local = nctx + bi as u32;
                    if members.len() > 200_000 {
                        let (m2, w2) = members
                            .par_chunks(50_000)
                            .map(|ch| {
                                let mut mm = FxHashMap::default();
                                let mut ww = 0u64;
                                scan(ch, me, local, &mut mm, &mut ww);
                                (mm, ww)
                            })
                            .reduce(
                                || (FxHashMap::default(), 0u64),
                                |mut x, y| {
                                    for (k, v) in y.0 {
                                        *x.0.entry(k).or_insert(0) += v;
                                    }
                                    (x.0, x.1 + y.1)
                                },
                            );
                        for (k, v) in m2 {
                            *m.entry(k).or_insert(0) += v;
                        }
                        wctx += w2;
                    } else {
                        scan(members, me, local, &mut m, &mut wctx);
                    }
                }
                for (ni, &u) in nodes.iter().enumerate() {
                    scan(&[u], u, nctx + bubs.len() as u32 + ni as u32, &mut m, &mut wctx);
                }
                let epos = |code: u32| -> [f32; 2] {
                    if (code as usize) < n { pos[code as usize] } else { [tn[code as usize - n].x, tn[code as usize - n].y] }
                };
                let lpos = |local: u32| -> [f32; 2] {
                    let l = (local - nctx) as usize;
                    if l < bubs.len() { [tn[bubs[l] as usize].x, tn[bubs[l] as usize].y] } else { pos[nodes[l - bubs.len()] as usize] }
                };
                let mut ev: Vec<(u32, f32, u32, u32)> = m
                    .iter()
                    .map(|(&k, &wt)| {
                        let (l, d) = ((k >> 32) as u32, k as u32);
                        let (p, q) = (lpos(l), epos(d));
                        (u32::MAX - wt, (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2), l, d)
                    })
                    .collect();
                ev.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap()).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)));
                let w_total: u64 = ev.iter().map(|e| (u32::MAX - e.0) as u64).sum::<u64>() + wctx;
                ev.truncate(edge_budget);
                let w_shown: u64 = ev.iter().map(|e| (u32::MAX - e.0) as u64).sum();
                // encode
                let mut b: Vec<u8> = Vec::with_capacity(64 + (myctx.len() + bubs.len()) * 48 + nodes.len() * 36 + ev.len() * 20);
                b.extend_from_slice(b"GMT0");
                b.extend_from_slice(&[z as u8, 0, 0, 0]);
                for v in [tx, ty, nctx, bubs.len() as u32, nodes.len() as u32, ev.len() as u32, *om_b, *om_n] {
                    b.extend_from_slice(&v.to_le_bytes());
                }
                for v in [*om_bn, w_total, w_shown, wctx] {
                    b.extend_from_slice(&v.to_le_bytes());
                }
                for &t in myctx.iter().chain(bubs.iter()) {
                    let tt = &tn[t as usize];
                    let topc = top[order[tt.start as usize] as usize].min(65535) as u16;
                    b.extend_from_slice(&t.to_le_bytes());
                    b.extend_from_slice(&tt.parent.to_le_bytes());
                    b.extend_from_slice(&tt.x.to_le_bytes());
                    b.extend_from_slice(&tt.y.to_le_bytes());
                    b.extend_from_slice(&tt.r.to_le_bytes());
                    b.extend_from_slice(&(tt.end - tt.start).to_le_bytes());
                    b.extend_from_slice(&tt.internal.to_le_bytes());
                    b.extend_from_slice(&tt.volume.to_le_bytes());
                    b.extend_from_slice(&[tt.level as u8, tt.flags]);
                    b.extend_from_slice(&topc.to_le_bytes());
                    // m1-05: communities inside an aggregate (0 for every other region)
                    b.extend_from_slice(&(if tt.flags & F_AGG != 0 { tt.comm } else { 0 }).to_le_bytes());
                }
                for &u in nodes.iter() {
                    let u = u as usize;
                    b.extend_from_slice(&(u as u32).to_le_bytes());
                    b.extend_from_slice(&pos[u][0].to_le_bytes());
                    b.extend_from_slice(&pos[u][1].to_le_bytes());
                    b.extend_from_slice(&tn[lp[u] as usize].node_r.to_le_bytes());
                    b.extend_from_slice(&(csr.deg(u) as u32).to_le_bytes());
                    b.extend_from_slice(&lp[u].to_le_bytes());
                    b.extend_from_slice(&(top[u].min(65535) as u16).to_le_bytes());
                    b.extend_from_slice(&[0, 0]);
                    b.extend_from_slice(&ids[u].to_le_bytes());
                }
                for e in &ev {
                    let q = epos(e.3);
                    let code = if (e.3 as usize) < n { e.3 } else { 0x8000_0000 | (e.3 - n as u32) };
                    b.extend_from_slice(&e.2.to_le_bytes());
                    b.extend_from_slice(&code.to_le_bytes());
                    b.extend_from_slice(&q[0].to_le_bytes());
                    b.extend_from_slice(&q[1].to_le_bytes());
                    b.extend_from_slice(&(u32::MAX - e.0).to_le_bytes());
                }
                TileOut { id: zxy_to_id(z, tx, ty), raw: b.len() as u32, gz: gz(&b), n_bub: bubs.len() as u32 + nctx, n_node: nodes.len() as u32, n_edge: ev.len() as u32, w_total, w_shown }
            })
            .collect();
        outs.sort_by_key(|t| t.id);
        let (raw, gzb): (u64, u64) = outs.iter().fold((0, 0), |s, t| (s.0 + t.raw as u64, s.1 + t.gz.len() as u64));
        let (nb, nn, ne): (u64, u64, u64) = outs.iter().fold((0, 0, 0), |s, t| (s.0 + t.n_bub as u64, s.1 + t.n_node as u64, s.2 + t.n_edge as u64));
        let (wt, ws): (u64, u64) = outs.iter().fold((0, 0), |s, t| (s.0 + t.w_total, s.1 + t.w_shown));
        eprintln!("z{z}: {} tiles, {} bubbles, {} nodes, {} edges, raw {:.1} MB gz {:.1} MB, shown weight {:.3}, {:.2}s", outs.len(), nb, nn, ne, raw as f64 / 1e6, gzb as f64 / 1e6, ws as f64 / wt.max(1) as f64, tz.elapsed().as_secs_f64());
        zstats.push(format!(
            "{{\"z\":{z},\"tiles\":{},\"bubbles\":{nb},\"nodes\":{nn},\"edges\":{ne},\"raw_bytes\":{raw},\"gz_bytes\":{gzb},\"edge_weight_total\":{wt},\"edge_weight_shown\":{ws},\"seconds\":{:.3}}}",
            outs.len(),
            tz.elapsed().as_secs_f64()
        ));
        all.extend(outs);
    }
    let build_s = t_build.elapsed().as_secs_f64();
    // container
    let tw = Instant::now();
    let mut entries = Vec::with_capacity(all.len());
    let mut off = 0u64;
    for t in &all {
        entries.push(Entry { id: t.id, off, len: t.gz.len() as u32, run: 1 });
        off += t.gz.len() as u64;
    }
    let tile_bytes = off;
    let (root, leaves) = build_dirs(&entries);
    // search index: records sorted by original id (u64 id, f32 x, f32 y), 16 B/node, plus sparse top index
    const BLOCK: usize = 2048;
    let mut perm: Vec<u32> = (0..n as u32).collect();
    perm.par_sort_unstable_by_key(|&u| ids[u as usize]);
    let mut srec: Vec<u8> = Vec::with_capacity(n * 16);
    let mut stop: Vec<u8> = vec![];
    for (i, &u) in perm.iter().enumerate() {
        if i % BLOCK == 0 {
            stop.extend_from_slice(&ids[u as usize].to_le_bytes());
        }
        srec.extend_from_slice(&ids[u as usize].to_le_bytes());
        srec.extend_from_slice(&pos[u as usize][0].to_le_bytes());
        srec.extend_from_slice(&pos[u as usize][1].to_le_bytes());
    }
    let m_und = csr.tgt.len() / 2;
    // search section offsets are relative to the end of the tile-data section (header: data offset + length)
    let meta = gz(format!(
        "{{\"name\":\"{graph}\",\"format\":\"glab-map-tile-v0\",\"n\":{n},\"m\":{m_und},\"zmax\":{zmax},\"tile_px\":256,\"budgets\":{{\"bubbles\":{bub_budget},\"nodes\":{node_budget},\"edges\":{edge_budget}}},\"search\":{{\"top_rel\":0,\"top_length\":{},\"rec_rel\":{},\"rec_length\":{},\"block\":{BLOCK},\"rec_bytes\":16,\"n\":{n}}}}}",
        stop.len(),
        stop.len(),
        srec.len()
    )
    .as_bytes());
    let tile_off = 127 + root.len() as u64 + meta.len() as u64 + leaves.len() as u64;
    let mut h = Vec::with_capacity(127);
    h.extend_from_slice(b"PMTiles");
    h.push(3);
    let root_off = 127u64;
    let meta_off = root_off + root.len() as u64;
    let leaf_off = meta_off + meta.len() as u64;
    for v in [root_off, root.len() as u64, meta_off, meta.len() as u64, leaf_off, leaves.len() as u64, tile_off, tile_bytes, all.len() as u64, all.len() as u64, all.len() as u64] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    h.extend_from_slice(&[1, 2, 2, 0, 0, zmax as u8]); // clustered, internal gzip, tile gzip, tile type unknown, min/max zoom
    for v in [-1_800_000_000i32, -850_000_000, 1_800_000_000, 850_000_000] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    h.push(0);
    h.extend_from_slice(&0i32.to_le_bytes());
    h.extend_from_slice(&0i32.to_le_bytes());
    assert_eq!(h.len(), 127);
    let mut f = std::io::BufWriter::new(std::fs::File::create(&out).unwrap());
    f.write_all(&h).unwrap();
    f.write_all(&root).unwrap();
    f.write_all(&meta).unwrap();
    f.write_all(&leaves).unwrap();
    for t in &all {
        f.write_all(&t.gz).unwrap();
    }
    f.write_all(&stop).unwrap();
    f.write_all(&srec).unwrap();
    f.flush().unwrap();
    let write_s = tw.elapsed().as_secs_f64();
    let total_bytes = tile_off + tile_bytes + stop.len() as u64 + srec.len() as u64;
    let mut sz: Vec<u32> = all.iter().map(|t| t.gz.len() as u32).collect();
    sz.sort();
    let mut rsz: Vec<u32> = all.iter().map(|t| t.raw).collect();
    rsz.sort();
    let dir_blocks: u64 = sz.iter().map(|&s| (s as u64 + 4095) / 4096 * 4096 + 256).sum();
    let j = format!(
        "{{\"graph\":\"{graph}\",\"layout\":\"{lay}\",\"n\":{n},\"m\":{m_und},\"threads\":{},\"zmax\":{zmax},\"tiles\":{},\"load_s\":{t_load:.3},\"build_s\":{build_s:.3},\"write_s\":{write_s:.3},\"total_s\":{:.3},\
\"file_bytes\":{total_bytes},\"tile_bytes_gz\":{tile_bytes},\"tile_bytes_raw\":{},\"root_dir_bytes\":{},\"leaf_dir_bytes\":{},\"metadata_bytes\":{},\"search_bytes\":{},\"search_top_bytes\":{},\
\"tile_gz_median\":{},\"tile_gz_p99\":{},\"tile_gz_max\":{},\"tile_raw_median\":{},\"tile_raw_max\":{},\"dir_of_files_bytes_4k_blocks\":{dir_blocks},\"peak_rss_mb\":{:.0},\"zoom\":[{}]}}",
        rayon::current_num_threads(),
        all.len(),
        t_all.elapsed().as_secs_f64(),
        rsz.iter().map(|&x| x as u64).sum::<u64>(),
        root.len(),
        leaves.len(),
        meta.len(),
        srec.len() + stop.len(),
        stop.len(),
        sz[sz.len() / 2],
        sz[sz.len() * 99 / 100],
        sz[sz.len() - 1],
        rsz[rsz.len() / 2],
        rsz[rsz.len() - 1],
        peak_rss_mb(),
        zstats.join(",")
    );
    if let Some(o) = a.get("json") {
        std::fs::write(o, &j).unwrap();
    }
    println!("{j}");
}
