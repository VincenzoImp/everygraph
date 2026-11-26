/* Invented sample graph for the style preview (data.html).
   Copied unchanged from the first viewer mockup (branch design-previews-archive, design/preview/viewer/index.html, part 1: utilities, the seeded graph generator,
   circle packing, cells, links, lookups, search, scripted path). Every name and number is invented; nothing is a measurement.
   Loaded as a classic script: its top-level names (root, expand, linksOf, nodeById, …) are shared with the page script. */
"use strict";
/* =====================================================================
   PART 1 — utilities and the invented graph
   Everything below is generated deterministically from SEED. No number is real.
   ===================================================================== */
const T_BOOT = performance.now();
const $ = (s, el) => (el || document).querySelector(s);
const SEED = 'everygraph-mock-3';
function h32(s) { let h = 2166136261; for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); } return h >>> 0; }
function mulberry(a) { return function () { a |= 0; a = a + 0x6D2B79F5 | 0; let t = Math.imul(a ^ a >>> 15, 1 | a); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; }; }
const R = key => mulberry(h32(SEED + '|' + key));
function gauss(r) { let u = r(); if (u < 1e-12) u = 1e-12; return Math.sqrt(-2 * Math.log(u)) * Math.cos(6.283185307179586 * r()); }
const fmt = n => Math.round(n).toLocaleString('en-US');
const pct = (a, b, d = 2) => { if (!(b > 0)) return '—'; const p = 100 * a / b; if (a < b && p >= 100 - Math.pow(10, -d) / 2) return '>' + (100 - Math.pow(10, -d)).toFixed(d) + '%'; if (a > 0 && p < Math.pow(10, -d)) return '<' + Math.pow(10, -d).toFixed(d) + '%'; return p.toFixed(d) + '%'; };
const clamp = (x, a, b) => x < a ? a : x > b ? b : x;
const sstep = (a, b, x) => { x = clamp((x - a) / (b - a), 0, 1); return x * x * (3 - 2 * x); };
const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

/* exact integer partition: parts are >= 0 and always sum to total */
function split(total, w) {
  const n = w.length, out = new Array(n); let sum = 0;
  for (let i = 0; i < n; i++) sum += w[i];
  if (!(sum > 0)) { out.fill(0); return out; }
  let acc = 0, prev = 0;
  for (let i = 0; i < n; i++) { acc += w[i]; const cur = i === n - 1 ? total : Math.round(acc / sum * total); out[i] = cur - prev; prev = cur; }
  return out;
}
/* the same, month by month (every quantity that depends on time is a 12-vector) */
function splitV(tv, w, mods) {
  const k = w.length, out = []; for (let i = 0; i < k; i++) out.push(new Array(12));
  const ww = new Array(k);
  for (let m = 0; m < 12; m++) {
    for (let i = 0; i < k; i++) ww[i] = w[i] * (mods ? mods[i][m] : 1);
    const s = split(tv[m], ww); for (let i = 0; i < k; i++) out[i][m] = s[i];
  }
  return out;
}
const zeros12 = () => [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
function season(r, amp) { const ph = r(), a = amp * (0.5 + r()); const v = []; for (let m = 0; m < 12; m++) v.push(Math.exp(a * Math.sin(6.2832 * (m / 12 + ph)) + 0.12 * gauss(r))); return v; }

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
let WIN = [0, 11];                                   // active time window, months inclusive
const WS = v => { let s = 0; for (let m = WIN[0]; m <= WIN[1]; m++) s += v[m]; return s; };
const winKey = () => WIN[0] + '-' + WIN[1];
const winLabel = () => WIN[0] === 0 && WIN[1] === 11 ? '2024' : (WIN[0] === WIN[1] ? MONTHS[WIN[0]] : MONTHS[WIN[0]] + ' – ' + MONTHS[WIN[1]]) + ' 2024';
const winFull = () => WIN[0] === 0 && WIN[1] === 11;

const GRAPH = { name: 'ethereum-2024', file: 'ethereum-2024.eg', n: 212438917, tx: 1831204566 };
const CATS = ['Exchange', 'DeFi', 'Stablecoin', 'Other labels', 'Unlabelled'];
const CAT_SHARE = [21.4, 9.8, 8.1, 12.3, 48.4];
const up = [1.25, 1.2, 1.15, 1.1, 1.0, 0.95, 0.9, 0.9, 0.88, 0.86, 0.85, 0.84], dn = up.slice().reverse();
const SPEC = [
  { name: 'Centralized exchanges', n: 24.1, tx: 26, hue: 3, k: 34, cat: 0, lab: ['deposit', 'sweeper', 'hot wallet'], kids: [['Deposit addresses', { giant: 1 }], ['Retail withdrawals'], ['Hot wallets', { nMul: .03, act: 55, hub: 1 }], ['Market makers', { act: 6 }], ['Fiat on-ramps'], ['Broker sub-accounts'], ['OTC desks', { act: 3 }], ['Internal sweeps'], ['Cold storage', { act: .2 }], ['Custody']] },
  { name: 'DeFi protocols', n: 13.2, tx: 19, hue: 7, k: 27, cat: 1, lab: ['pool', 'router', 'vault', 'liquidator'], kids: [['DEX liquidity pools'], ['Lending', { sub: ['Lending pools', 'Leveraged loopers', 'Stable borrowers', 'Liquidators', 'Flash-loan users', 'Collateral managers'] }], ['DEX routers & aggregators', { act: 8, hub: 1 }], ['Liquid staking'], ['Yield vaults'], ['Perpetuals', { act: 3 }], ['Restaking'], ['Governance & treasuries', { act: .5 }], ['Launchpads'], ['Oracles', { nMul: .15, act: 12, hub: 1 }], ['Options']] },
  { name: 'Stablecoin issuers', n: 11.5, tx: 9, hue: 4, k: 13, cat: 2, lab: ['merchant', 'payroll', 'treasury'], kids: [['Merchant payments'], ['Payroll & remittance'], ['Exchange settlement', { act: 3 }], ['Savings wrappers'], ['Mint & burn', { nMul: .3, act: 6, hub: 1 }], ['Issuer treasuries', { nMul: .1, act: 9, hub: 1 }]] },
  { name: 'Unlabelled cluster A', n: 10.8, tx: 6, hue: -1, k: 38, cat: 4, lab: [] },
  { name: 'NFT markets', n: 7.9, tx: 4, hue: 0, k: 21, cat: 3, season: up, lab: ['mint', 'marketplace', 'collector'], kids: [['Collectors'], ['Mint contracts'], ['Marketplaces', { act: 6, hub: 1 }], ['Airdrop farmers'], ['Wash-trading rings', { act: 4 }], ['Royalty splitters']] },
  { name: 'Bridges', n: 6.4, tx: 5, hue: 2, k: 11, cat: 3, lab: ['bridge', 'relayer'], kids: [['Rollup bridges'], ['Liquidity bridges'], ['Cross-chain messaging'], ['Wrapped assets'], ['Relayers', { nMul: .2, act: 7, hub: 1 }]] },
  { name: 'Unlabelled cluster B', n: 6.0, tx: 3.5, hue: -1, k: 30, cat: 4, lab: [] },
  { name: 'Gaming', n: 4.7, tx: 2.5, hue: 5, k: 17, cat: 3, season: dn, lab: ['player', 'item market', 'guild'], kids: [['Player wallets'], ['Item markets', { act: 3 }], ['Reward distributors', { act: 4 }], ['Guilds'], ['Game treasuries', { nMul: .2 }]] },
  { name: 'Unlabelled cluster C', n: 3.3, tx: 2, hue: -1, k: 23, cat: 4, lab: [] },
  { name: 'MEV & arbitrage bots', n: 2.1, tx: 11, act: 5, hue: 6, k: 8, cat: 3, lab: ['searcher', 'builder', 'bot'], kids: [['Arbitrage routers'], ['Sandwich bots'], ['Searchers'], ['Liquidation bots'], ['Block builders', { nMul: .1, act: 10, hub: 1 }], ['Relays', { nMul: .1 }]] },
  { name: 'Mixers', n: 1.2, tx: .8, hue: 1, k: 7, cat: 3, season: [.8, .8, .9, .9, 1, 1.1, 1.3, 2.1, 1.5, 1, .9, .8], lab: ['pool', 'relayer'], kids: [['Pool deposits'], ['Pool withdrawals'], ['Fresh-wallet clusters'], ['Peel chains'], ['Relayers', { nMul: .2 }]] },
  { name: '1,843 small communities', n: 8.8, tx: 3, hue: -1, cat: 4, agg: 1843, lab: [] }
];

const INDEX = [];                                    // nodes with names, for search
const LT = [1, 0, 0, 0, 0, 0, 0];                     // how many regions exist per level
function mk(parent, idx, o) {
  const n = Object.assign({ id: parent ? (parent.id ? parent.id + '.' + idx : '' + idx) : '', idx, parent, depth: parent ? parent.depth + 1 : 0, kind: 'community', name: '', named: false, n: 0, intv: null, btw: null, x: 0, y: 0, r: 0, kids: null, act: 1, aw: 1, hue: parent ? parent.hue : -1, top: parent ? parent.top : null, cat: 4, catShare: .6, u: .5, spec: null, M: 0, giant: false, hub: parent ? parent.hub : false, lab: parent ? parent.lab : [] }, o);
  if (n.depth === 1) n.top = n;
  return n;
}
const root = mk(null, 0, { name: 'All', n: GRAPH.n });
root.intv = split(GRAPH.tx, [0.92, 0.86, 1.18, 1.06, 0.97, 0.86, 0.83, 0.90, 0.95, 1.04, 1.19, 1.24]);
const expandable = n => n.kind !== 'address' && n.kind !== 'more' && !n.rest;
const commNo = id => '#' + (100000 + h32(SEED + 'c' + id) % 900000);

function planK(n) {
  if (n._plan) return n._plan;
  const r = R('k' + n.id); let k, agg = 0, share = 0;
  if (n.spec && n.spec.k) k = n.spec.k; else if (n.giant) k = Math.round(16 + r() * 13); else k = Math.round(8 + r() * r() * 30);
  k = Math.max(5, Math.min(k, 40, Math.floor(n.n / 14)));
  if (!n.giant && n.depth >= 1 && r() < 0.62) { let M = Math.round(k * (2 + r() * 38)); share = 0.03 + r() * 0.06; M = Math.min(M, Math.floor(n.n * share / 4)); if (M >= 6) agg = M; }
  return n._plan = { k, agg, share };
}
const isLeafCommunity = n => n.kind !== 'aggregate' && (n.n <= 90 || n.depth >= 6);

function expand(P) {
  if (P.kids) return P.kids;
  const r = R('x' + P.id), defs = []; let intraF = 0.7, ns;
  if (P.depth === 0) {
    SPEC.forEach(s => defs.push({ kind: s.agg ? 'aggregate' : 'community', name: s.name, named: true, tw: s.tx, act: s.act || 1, spec: s, hue: s.hue, cat: s.cat, M: s.agg || 0, lab: s.lab, mod: s.season || null }));
    ns = split(P.n, SPEC.map(s => s.n)); intraF = 0.69;
  } else if (P.kind === 'aggregate') {
    const M = P.M, m = M <= 48 ? M : 44;               // the largest ones are drawn; the rest stays one hatched area that says how many it holds
    for (let i = 0; i < m; i++) defs.push({ kind: 'community', name: 'community ' + commNo(P.id + '.' + i), min: 3, nw: Math.pow(i + 1, -0.9) * Math.exp(.25 * gauss(r)) });
    if (M > m) { let rw = 0; for (let i = m + 1; i <= M; i++) rw += Math.pow(i, -0.9); defs.push({ kind: 'aggregate', rest: true, name: '+' + fmt(M - m) + ' smaller communities', named: true, M: M - m, min: 3 * (M - m), nw: rw }); }
    intraF = 0.93;
  } else if (isLeafCommunity(P)) {
    const more = P.n > 80, cnt = more ? 79 : P.n, a = P.hub ? 1.5 + 0.3 * r() : 0.7 + 0.5 * r();
    for (let i = 0; i < cnt; i++) defs.push({ kind: 'address', dw: Math.pow(i + 1, -a) * Math.exp(.35 * gauss(r)) });
    if (more) { defs.push({ kind: 'more', name: '+' + fmt(P.n - 79) + ' more addresses', named: true }); intraF = Math.min(.9, Math.pow((P.n - 79) / P.n, 1.3) * 0.8); } else intraF = 0;
    ns = defs.map((d, i) => d.kind === 'more' ? P.n - 79 : 1);
  } else {
    const pl = planK(P), a = 0.55 + 0.5 * r(), names = (P.spec && P.spec.kids) || (P.sub ? P.sub.map(s => [s]) : []);
    for (let i = 0; i < pl.k; i++) {
      const nm = names[i], o = (nm && nm[1]) || {};
      const d = { kind: P.giant ? 'subdivision' : 'community', min: 2, act: (P.hub ? Math.exp(1.2 * gauss(r)) * (i === 0 ? 7 : 1) : Math.exp(.4 * gauss(r))) * (o.act || 1), opts: o };
      d.nw = P.giant ? Math.exp(.16 * gauss(r)) : Math.pow(i + 1, -a) * Math.exp(.35 * gauss(r)) * (o.nMul || 1);
      if (P.giant) d.name = 'part ' + (i + 1); else if (nm) { d.name = nm[0]; d.named = true; } else d.name = 'community ' + commNo(P.id + '.' + i);
      defs.push(d);
    }
    if (pl.agg) defs.push({ kind: 'aggregate', name: fmt(pl.agg) + ' small communities', named: true, M: pl.agg, min: 3 * pl.agg, nw: 0, fixShare: pl.share });
    intraF = P.giant ? 0.36 : 0.55 + 0.25 * r();
  }
  if (!ns) {                                         // children counts sum exactly to the parent
    let mins = 0; defs.forEach(d => mins += d.min);
    const wsum = defs.reduce((s, d) => s + d.nw, 0);
    defs.forEach(d => { if (d.fixShare) d.nw = wsum * d.fixShare / (1 - d.fixShare); });
    ns = split(P.n - mins, defs.map(d => d.nw)).map((x, i) => x + defs[i].min);
  }
  const kids = defs.map((d, i) => {
    const c = mk(P, i, { kind: d.kind, name: d.name || '', named: !!d.named, n: ns[i], act: d.act || 1, spec: d.spec || null, M: d.M || 0, rest: !!d.rest });
    if (d.hue !== undefined) c.hue = d.hue;
    if (d.lab) c.lab = d.lab;
    if (d.opts && d.opts.sub) c.sub = d.opts.sub;
    if (d.opts && d.opts.hub) c.hub = true;
    c.aw = d.kind === 'address' ? d.dw : d.kind === 'more' ? defs.reduce((s, x) => s + (x.dw || 0), 0) * (0.8 + Math.min(2.2, Math.log10(c.n) * .45)) : Math.pow(c.n, 0.85) * c.act;
    c.giant = !!(d.opts && d.opts.giant) || (d.kind === 'community' && !d.named && c.depth >= 2 && c.n > 30000 && r() < 0.13);
    c.cat = d.cat !== undefined ? d.cat : (r() < 0.74 ? P.cat : [4, 3, 0, 4, 1, 2, 4, 3][Math.floor(r() * 8)]);
    c.catShare = 0.46 + 0.5 * r();
    let u = P.depth === 0 ? r() : P.u + 0.33 * gauss(r); u = Math.abs(u); if (u > 1) u = 2 - u; c.u = clamp(u, 0, 0.999);
    if (P.depth === 0) c.u = [.93, .86, .7, .3, .45, .62, .2, .35, .1, .97, .5, .05][i];
    return c;
  });
  const tw = kids.map((c, i) => defs[i].tw !== undefined ? defs[i].tw : (c.kind === 'address' ? 0 : Math.pow(c.n, 1.05) * c.act));
  const mods = kids.map((c, i) => defs[i].mod || season(r, c.kind === 'address' ? 0 : .22));
  const intra = P.intv.map(v => Math.round(v * intraF));
  const iv = tw.some(x => x > 0) ? splitV(intra, tw, mods) : kids.map(zeros12);
  P.btw = P.intv.slice();
  kids.forEach((c, i) => { c.intv = iv[i]; for (let m = 0; m < 12; m++) P.btw[m] -= iv[i][m]; });
  P.kids = kids;
  layoutKids(P, r);
  for (const c of kids) if (c.depth <= 2 || (c.named && c.kind === 'community')) INDEX.push(c);
  return kids;
}

/* ---------- layout: circle packing (shared by both map styles) ---------- */
function packCircles(rs, aspect) {
  const n = rs.length, xs = new Float64Array(n), ys = new Float64Array(n), a2 = aspect * aspect;
  if (n > 1) xs[1] = rs[0] + rs[1];
  for (let c = 2; c < n; c++) {
    let best = Infinity, bx = 0, by = 0; const rc = rs[c];
    for (let i = 0; i < c; i++) {
      const ri = rs[i] + rc;
      for (let j = i + 1; j < c; j++) {
        const rj = rs[j] + rc, dx = xs[j] - xs[i], dy = ys[j] - ys[i], d2 = dx * dx + dy * dy, d = Math.sqrt(d2);
        if (d > ri + rj || d < Math.abs(ri - rj) || d === 0) continue;
        const a = (ri * ri - rj * rj + d2) / (2 * d), h2 = ri * ri - a * a; if (h2 < 0) continue;
        const h = Math.sqrt(h2), mx = xs[i] + a * dx / d, my = ys[i] + a * dy / d;
        for (let s = -1; s <= 1; s += 2) {
          const px = mx + s * h * dy / d, py = my - s * h * dx / d, sc = px * px / a2 + py * py;
          if (sc >= best) continue;
          let ok = true;
          for (let q = 0; q < c; q++) { if (q === i || q === j) continue; const ex = px - xs[q], ey = py - ys[q], rr = (rs[q] + rc) * 0.999999; if (ex * ex + ey * ey < rr * rr) { ok = false; break; } }
          if (ok) { best = sc; bx = px; by = py; }
        }
      }
    }
    xs[c] = bx; ys[c] = by;
  }
  return [xs, ys];
}
function enclose(xs, ys, rs) {
  const n = rs.length; let cx = 0, cy = 0, sw = 0;
  for (let i = 0; i < n; i++) { const w = rs[i] * rs[i]; cx += xs[i] * w; cy += ys[i] * w; sw += w; }
  cx /= sw; cy /= sw; let best = null;
  for (let it = 0; it < 120; it++) {
    let f = 0, fd = -1; for (let i = 0; i < n; i++) { const d = Math.hypot(xs[i] - cx, ys[i] - cy) + rs[i]; if (d > fd) { fd = d; f = i; } }
    if (!best || fd < best[2]) best = [cx, cy, fd];
    const dx = xs[f] - cx, dy = ys[f] - cy, d = Math.hypot(dx, dy) || 1, st = (fd - rs[f] > 0 ? 1 : 0) * d / (it + 2);
    cx += dx / d * st; cy += dy / d * st;
  }
  return best;
}
function layoutKids(P, r) {
  const K = P.kids, k = K.length, leaf = K[0].kind === 'address';
  let rad, xs, ys;
  if (leaf) {                                        // addresses: jittered spiral, hubs in the middle, then relaxed apart
    rad = K.map(c => c.kind === 'more' ? 0 : Math.pow(c.aw, 0.3));
    const mx = Math.max.apply(null, rad); rad = rad.map((x, i) => K[i].kind === 'more' ? mx * 0.8 : Math.max(x, mx * 0.3));
    xs = new Float64Array(k); ys = new Float64Array(k); const sp = mx * 2.3;
    for (let i = 0; i < k; i++) { const a = i * 2.39996 + r() * .9, d = sp * Math.sqrt(i + .3) * (.8 + .4 * r()); xs[i] = d * Math.cos(a); ys[i] = d * Math.sin(a); }
    for (let it = 0; it < 70; it++) for (let i = 0; i < k; i++) {
      for (let j = i + 1; j < k; j++) { const dx = xs[j] - xs[i], dy = ys[j] - ys[i], d = Math.hypot(dx, dy) || 1e-9, min = (rad[i] + rad[j]) * 1.75; if (d < min) { const p = (min - d) / d * .5; xs[i] -= dx * p; ys[i] -= dy * p; xs[j] += dx * p; ys[j] += dy * p; } }
      xs[i] *= .985; ys[i] *= .985;
    }
  } else {
    rad = K.map(c => Math.sqrt(c.n));
    const mx = Math.max.apply(null, rad), key = K.map((c, i) => c.kind === 'aggregate' ? -1 : rad[i] * Math.exp(.2 * gauss(r)));
    const ord = K.map((c, i) => i).sort((a, b) => key[b] - key[a]);
    const pr = ord.map(i => rad[i] * 1.07 + mx * (P.depth === 0 ? 0.075 : 0.02)), res = packCircles(pr, P.depth === 0 ? 1.62 : 1);
    xs = new Float64Array(k); ys = new Float64Array(k); ord.forEach((i, q) => { xs[i] = res[0][q]; ys[i] = res[1][q]; });
  }
  if (P.depth === 0) {                               // the world: a wide arrangement, measured by its bounding box
    let x0 = 1e9, x1 = -1e9, y0 = 1e9, y1 = -1e9;
    for (let i = 0; i < k; i++) { x0 = Math.min(x0, xs[i] - rad[i]); x1 = Math.max(x1, xs[i] + rad[i]); y0 = Math.min(y0, ys[i] - rad[i]); y1 = Math.max(y1, ys[i] + rad[i]); }
    const s = 1200 / (y1 - y0), cx = (x0 + x1) / 2, cy = (y0 + y1) / 2;
    K.forEach((c, i) => { c.x = (xs[i] - cx) * s; c.y = (ys[i] - cy) * s; c.r = rad[i] * s; });
    P.bw = (x1 - x0) / 2 * s; P.bh = 600; P.r = Math.max(P.bw, P.bh);
    const pw = 2.6; let m = 0, a = P.bw, b = P.bh;   // outline for the "cells" style: a superellipse that contains every region
    for (const c of K) for (let t = 0; t < 24; t++) { const px = c.x + c.r * Math.cos(t * .2618), py = c.y + c.r * Math.sin(t * .2618); m = Math.max(m, Math.pow(Math.pow(Math.abs(px / a), pw) + Math.pow(Math.abs(py / b), pw), 1 / pw)); }
    a *= m * 1.025; b *= m * 1.025; P.poly = [];
    for (let t = 0; t < 120; t++) { const an = t / 120 * 6.283185307, c = Math.cos(an), sn = Math.sin(an); P.poly.push(a * Math.sign(c) * Math.pow(Math.abs(c), 2 / pw), b * Math.sign(sn) * Math.pow(Math.abs(sn), 2 / pw)); }
    P.pa = a; P.pb = b; P.X = 0; P.Y = 0; P.R = Math.sqrt(polyAC(P.poly)[0] / Math.PI); P.ir = Math.min(a, b); P.br = Math.max(a, b);
  } else {
    const e = enclose(xs, ys, rad), s = P.r * (leaf ? 0.84 : 0.93) / e[2];
    K.forEach((c, i) => { c.x = P.x + (xs[i] - e[0]) * s; c.y = P.y + (ys[i] - e[1]) * s; c.r = rad[i] * s * (c.kind === 'address' ? .62 : 1); });
  }
}

/* ---------- layout: space-filling cells (power diagram of the packed circles, clipped to the parent's cell) ---------- */
function clipHalf(poly, ax, ay, c) {
  const out = [], n = poly.length >> 1; if (!n) return out;
  let px = poly[2 * n - 2], py = poly[2 * n - 1], pd = ax * px + ay * py - c;
  for (let i = 0; i < n; i++) {
    const x = poly[2 * i], y = poly[2 * i + 1], d = ax * x + ay * y - c;
    if (d <= 0) { if (pd > 0) { const t = pd / (pd - d); out.push(px + (x - px) * t, py + (y - py) * t); } out.push(x, y); }
    else if (pd <= 0) { const t = pd / (pd - d); out.push(px + (x - px) * t, py + (y - py) * t); }
    px = x; py = y; pd = d;
  }
  return out;
}
function polyAC(p) {                                  // area and centroid of a convex polygon
  let A = 0, gx = 0, gy = 0; for (let q = 0, n = p.length; q < n; q += 2) { const x0 = p[q], y0 = p[q + 1], x1 = p[(q + 2) % n], y1 = p[(q + 3) % n], cr = x0 * y1 - x1 * y0; A += cr; gx += (x0 + x1) * cr; gy += (y0 + y1) * cr; }
  return A === 0 ? [0, p[0] || 0, p[1] || 0] : [Math.abs(A / 2), gx / (3 * A), gy / (3 * A)];
}
function powerCells(base, px, py, w) {
  const k = px.length, out = [];
  for (let i = 0; i < k; i++) {
    let poly = base;
    for (let j = 0; j < k && poly.length >= 6; j++) { if (j !== i) poly = clipHalf(poly, 2 * (px[j] - px[i]), 2 * (py[j] - py[i]), px[j] * px[j] + py[j] * py[j] - px[i] * px[i] - py[i] * py[i] - w[j] + w[i]); }
    out.push(poly);
  }
  return out;
}
/* A small Voronoi treemap: start from the packed positions, then move sites to cell centroids and adapt
   weights until each cell's area matches its share of addresses. Cells style only; computed lazily. */
function ensureCells(P) {
  if (P._cells) return; P._cells = true;
  if (P.parent) ensureCells(P.parent);
  const K = expand(P), k = K.length;
  if (K[0].kind === 'address') { const s = P.ir * .93 / P.r; for (const c of K) { c.X = P.X + (c.x - P.x) * s; c.Y = P.Y + (c.y - P.y) * s; c.R = c.ir = c.br = c.r * s; } return; }
  const inv = 1 / P.R, base = []; for (let i = 0; i < P.poly.length; i += 2) base.push((P.poly[i] - P.X) * inv, (P.poly[i + 1] - P.Y) * inv);
  const A = polyAC(base)[0], tot = K.reduce((s, c) => s + c.n, 0), tg = K.map(c => Math.max(c.n / tot, 2e-5) * A), s0 = P.ir * inv * .9 / P.r;
  const px = K.map(c => (c.x - P.x) * s0), py = K.map(c => (c.y - P.y) * s0), w = tg.map(t => t / Math.PI * .5);
  let cells = null;
  for (let it = 0; it < 70; it++) {
    for (let i = 0; i < k; i++) { let lim = Infinity; for (let j = 0; j < k; j++) if (j !== i) { const d2 = (px[i] - px[j]) * (px[i] - px[j]) + (py[i] - py[j]) * (py[i] - py[j]); if (d2 < lim) lim = d2; } if (w[i] > lim * .92) w[i] = lim * .92; }
    cells = powerCells(base, px, py, w); if (it === 69) break;
    for (let i = 0; i < k; i++) { const ac = polyAC(cells[i]); if (cells[i].length < 6 || !(ac[0] > 0)) { w[i] *= 1.4; continue; } px[i] += (ac[1] - px[i]) * .9; py[i] += (ac[2] - py[i]) * .9; w[i] = Math.max(1e-12, w[i] * clamp(tg[i] / ac[0], .7, 1.45)); }
  }
  for (let i = 0; i < k; i++) {
    const c = K[i], poly = cells[i];
    if (poly.length < 6) { c.poly = null; c.X = P.X + px[i] * P.R; c.Y = P.Y + py[i] * P.R; c.R = c.ir = c.br = P.R * 1e-4; continue; }
    const ac = polyAC(poly), wp = new Array(poly.length); let inr = Infinity, br = 0;
    for (let q = 0, n = poly.length; q < n; q += 2) { wp[q] = P.X + poly[q] * P.R; wp[q + 1] = P.Y + poly[q + 1] * P.R; const x0 = poly[q], y0 = poly[q + 1], x1 = poly[(q + 2) % n], y1 = poly[(q + 3) % n], ex = x1 - x0, ey = y1 - y0, L = Math.hypot(ex, ey) || 1e-12; inr = Math.min(inr, Math.abs((ac[1] - x0) * ey - (ac[2] - y0) * ex) / L); br = Math.max(br, Math.hypot(x0 - ac[1], y0 - ac[2])); }
    c.poly = wp; c.X = P.X + ac[1] * P.R; c.Y = P.Y + ac[2] * P.R; c.R = Math.sqrt(ac[0] / Math.PI) * P.R; c.ir = inr * P.R; c.br = br * P.R;
  }
}

/* ---------- links (all exact partitions, computed on demand) ---------- */
function pairsOf(P) {
  if (P._pairs) return P._pairs;
  const K = expand(P), k = K.length, r = R('p' + P.id), leaf = K[0].kind === 'address', I = [], J = [], w = [], mods = [];
  for (let i = 0; i < k; i++) for (let j = i + 1; j < k; j++) {
    if (leaf && !(i < 2 && r() < .55) && r() > 4.5 / k) continue;
    const gap = Math.max(0, Math.hypot(K[i].x - K[j].x, K[i].y - K[j].y) - K[i].r - K[j].r) / P.r;
    I.push(i); J.push(j); w.push(Math.sqrt(K[i].aw * K[j].aw) * Math.exp((leaf ? .9 : P.depth === 0 ? .5 : 1.15) * gauss(r)) / Math.pow(.22 + gap, 1.5));
    if (leaf) { const a = Math.floor(r() * 12), b = Math.min(11, a + Math.floor(r() * r() * 9)); const md = []; for (let m = 0; m < 12; m++) md.push(m >= a && m <= b ? 1 : .015); mods.push(md); }
    else mods.push(season(r, .3));
  }
  const vs = splitV(P.btw, w, mods), list = [], by = K.map(() => []), map = new Map();
  for (let p = 0; p < w.length; p++) {
    let s = 0; for (let m = 0; m < 12; m++) s += vs[p][m]; if (!s) continue;
    const e = { i: I[p], j: J[p], v: vs[p] }; list.push(e); by[e.i].push({ o: e.j, v: e.v }); by[e.j].push({ o: e.i, v: e.v }); map.set(e.i * 128 + e.j, e.v);
  }
  return P._pairs = { list, by, map };
}
const pairVec = (P, i, j) => pairsOf(P).map.get(i < j ? i * 128 + j : j * 128 + i);
function shareVec(P, key, vec, idx) {                 // child idx's exact share of one of P's outside links
  const K = P.kids, r = R('s' + P.id + '>' + key), w = K.map(c => c.aw * Math.exp(.95 * gauss(r)));
  return splitV(vec, w, null)[idx];
}
/* links that leave the parent: the parent's siblings one by one, top-level regions and sub-regions one by one,
   anything deeper and farther folded into "elsewhere in <own sub-region>" (declared, not drawn) */
function extOf(n) {
  if (n._ext) return n._ext;
  const out = [], P = n.parent;
  if (!P || !P.parent) return n._ext = out;
  const G = P.parent, pr = pairsOf(G);
  for (const e of pr.by[P.idx]) { const v = shareVec(P, G.kids[e.o].id, e.v, n.idx); if (v.some(x => x)) out.push({ t: G.kids[e.o], v, rel: 'uncle' }); }
  let else_ = null;
  for (const e of extOf(P)) {
    const v = shareVec(P, (e.elsewhere ? 'E' : '') + e.t.id, e.v, n.idx); if (!v.some(x => x)) continue;
    if (e.t.depth <= 2 && !e.elsewhere) out.push({ t: e.t, v, rel: 'region' });
    else { if (!else_) out.push(else_ = { t: chain(n)[2], v: zeros12(), rel: 'elsewhere', elsewhere: true }); for (let m = 0; m < 12; m++) else_.v[m] += v[m]; }
  }
  return n._ext = out;
}
function linksOf(n) {                                 // every link of n, heaviest first, for the active time window
  if (n._links && n._linksKey === winKey()) return n._links;
  const out = [], P = n.parent;
  if (P) { for (const e of pairsOf(P).by[n.idx]) out.push({ t: P.kids[e.o], v: e.v, rel: 'sibling' }); for (const e of extOf(n)) out.push(e); }
  const L = out.map(e => ({ t: e.t, rel: e.rel, elsewhere: !!e.elsewhere, w: WS(e.v) })).filter(e => e.w > 0).sort((a, b) => b.w - a.w);
  n._linksKey = winKey(); return n._links = L;
}
const cutOf = n => linksOf(n).reduce((s, e) => s + e.w, 0);
const outFrac = (a, b) => 0.22 + 0.56 * R('o' + a.id + '>' + b.id)();

/* ---------- lookups ---------- */
function nodeById(id) {
  if (id === '' || id == null) return root;
  let n = root;
  for (const p of String(id).split('.')) { if (!expandable(n) || !/^\d+$/.test(p)) return null; const K = expand(n), c = K[+p]; if (!c) return null; n = c; }
  return n;
}
function chain(n) { const a = []; for (; n; n = n.parent) a.unshift(n); return a; }   // root … n
const ALIAS = {};
function addrOf(n) { let s = ''; for (let i = 0; i < 5; i++) s += ('0000000' + h32(SEED + 'a' + i + n.id).toString(16)).slice(-8); const al = ALIAS[n.id]; if (al) s = al + s.slice(al.length); return s; }
const shortAddr = n => { const a = addrOf(n); return '0x' + a.slice(0, 4) + '…' + a.slice(-4); };
function addrMeta(n) {
  if (n._meta) return n._meta;
  const r = R('m' + n.id), hub = n.idx === 0 && n.parent.n > 30, lab = n.lab && n.lab.length && (hub || r() < .16) ? n.lab[Math.floor(r() * n.lab.length)] : '';
  const contract = r() < (n.top.spec.cat === 1 ? .3 : .08);
  const d0 = Math.floor(r() * r() * 330), first = new Date(Date.UTC(2024, 0, 1 + d0)), last = new Date(Date.UTC(2024, 0, 1 + d0 + Math.floor((365 - d0) * (0.4 + 0.6 * r()))));
  return n._meta = { label: lab, type: contract ? 'Contract' : 'Wallet (EOA)', first, last };
}
const fmtDate = d => d.getUTCDate() + ' ' + MONTHS[d.getUTCMonth()] + ' ' + d.getUTCFullYear();
function title(n) { return n.kind === 'address' ? shortAddr(n) : n.depth === 0 ? 'Whole graph' : n.kind === 'subdivision' ? n.parent.name.replace(/^community /, '') + ' · ' + n.name : n.name; }
function shortTitle(n) { return n.kind === 'address' ? shortAddr(n) : n.named || n.kind === 'subdivision' ? n.name : n.name.replace(/^community /, ''); }
const LEVELNOUN = ['graph', 'regions', 'sub-regions', 'communities', 'communities', 'communities', 'communities'];
function kindText(n) {
  if (n.kind === 'address') return 'Single address';
  if (n.kind === 'more') return 'Addresses not drawn individually';
  if (n.depth === 0) return 'The whole graph';
  if (n.rest) return fmt(n.M) + ' communities not drawn one by one';
  if (n.kind === 'aggregate') return 'Aggregate of ' + fmt(n.M) + ' small communities';
  if (n.kind === 'subdivision') return 'Subdivision · not a community';
  return 'Found community · level ' + n.depth + ' of 6';
}
function descend(n, key, pow) {
  const r = R('d' + key);
  while (n.kind !== 'address') { let K = expand(n).filter(c => c.kind !== 'more' && c.kind !== 'aggregate'); if (!K.length) K = expand(n).filter(c => c.kind !== 'more' && !c.rest); n = K[Math.floor(K.length * Math.pow(r(), pow || 2.2))]; }
  return n;
}
function topMembers(n, count) {                       // busiest addresses: follow the busiest branches, then rank what was found
  if (n._tm) return n._tm;
  const busy = m => expand(m).filter(c => c.kind !== 'more' && !c.rest).sort((a, b) => b.aw - a.aw), out = [];
  const walk = (m, width) => { const K = busy(m); if (K[0].kind === 'address') { for (const a of K.slice(0, width)) out.push(a); return; } K.slice(0, width).forEach((c, i) => walk(c, i === 0 ? Math.max(1, width - 1) : 1)); };
  walk(n, n.kids && n.kids[0].kind === 'address' ? 12 : 5);
  return n._tm = out.map(a => ({ a, w: cutOf(a) })).sort((x, y) => y.w - x.w).slice(0, count).map(x => x.a);
}
function firstAddr(n) { while (n.kind !== 'address') n = expand(n)[0]; return n; }
const kidByName = (n, name) => expand(n).find(c => c.name === name);

/* ---------- search (exact and prefix; address prefixes are honoured by aliasing a deterministic pick) ---------- */
function search(q) {
  q = q.trim(); const out = { regions: [], addrs: [], q };
  if (!q) return out;
  const lq = q.toLowerCase(), hex = lq.replace(/^0x/, '');
  if (/^[0-9a-f]{2,40}$/.test(hex) && (lq.startsWith('0x') || hex.length >= 3)) {
    const cnt = hex.length >= 40 ? 1 : hex.length >= 8 ? 2 : 6, r = R('q' + hex.slice(0, 6));
    for (let i = 0; i < cnt; i++) { const reg = root.kids[[0, 0, 1, 3, 4, 6, 2, 5, 9, 10, 7][Math.floor(r() * 11)]], a = descend(reg, 'q' + hex + i, 1.6); if (out.addrs.find(x => x.n === a)) continue; out.addrs.push({ n: a, alias: hex }); }
  }
  const m = [];
  for (const n of INDEX) { const nm = n.name.toLowerCase(), i = nm.indexOf(lq); if (i < 0) continue; m.push({ n, at: i, sc: (i === 0 ? 0 : /[\s#&]/.test(nm[i - 1]) ? 1 : 2) * 1e12 - n.n }); }
  m.sort((a, b) => a.sc - b.sc); out.total = m.length; out.regions = m.slice(0, 7);
  return out;
}

/* ---------- path between two addresses (scripted: one route through an exchange hub, one that avoids hubs) ---------- */
function repAddr(n) { return n.kind === 'address' ? n : n.kind === 'more' ? n.parent.kids[0] : topMembers(n, 5)[0]; }
function computePath(a, b, avoid) {
  a = repAddr(a); b = repAddr(b);
  const key = a.id + '~' + b.id, cex = root.kids[0]; let nodes;
  const sib = x => { const K = x.parent.kids.filter(c => c.kind === 'address' && c !== x && c !== a && c !== b); return K.length ? K[Math.min(K.length - 1, 1 + h32(x.id) % 3)] : descend(x.parent.parent, x.id + 'sib', 2); };
  if (!avoid) nodes = [a, descend(kidByName(cex, 'Deposit addresses'), key + 'dep', 1.5), firstAddr(kidByName(cex, 'Hot wallets')), b];
  else nodes = [a, sib(a), descend(root.kids[5], key + 'br', 1.8), descend(root.kids[6], key + 'ub', 1.8), sib(b), b];
  nodes = nodes.filter((n, i) => i === 0 || i === nodes.length - 1 || (n !== a && n !== b));
  const hops = [];
  for (let i = 0; i + 1 < nodes.length; i++) {
    const r = R('h' + nodes[i].id + '>' + nodes[i + 1].id + avoid), cnt = 1 + Math.floor(r() * r() * 9), eth = Math.exp(1.2 + 1.6 * gauss(r));
    const m = WIN[0] + Math.floor(r() * (WIN[1] - WIN[0] + 1)), day = 1 + Math.floor(r() * 28);
    hops.push({ count: cnt, eth: eth * cnt, date: new Date(Date.UTC(2024, m, day)) });
  }
  return { nodes, hops, a, b, excluded: 1204 };
}

/* build the first two levels now: they feed search and the "how many exist at this level" counts */
expand(root); root.kids.forEach(expand); root.kids.forEach(c => c.kids.forEach(d => { if (d.sub) expand(d); }));
LT[1] = root.kids.length; LT[2] = root.kids.reduce((s, c) => s + c.kids.length, 0);
LT[3] = root.kids.reduce((s, c) => s + c.kids.reduce((t, d) => t + (d.kind === 'aggregate' ? Math.min(d.M, 45) : isLeafCommunity(d) ? 0 : planK(d).k + (planK(d).agg ? 1 : 0)), 0), 0);
LT[4] = Math.round(LT[3] * 13.7); LT[5] = Math.round(LT[4] * 9.2); LT[6] = Math.round(LT[5] * 6.4);
