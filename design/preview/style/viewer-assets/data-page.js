/* Page script for data.html (style preview). Draws every specimen from the tokens and the invented sample graph.
   Not product code. Colours are never written here: they are read from the CSS variables, so the measurements
   on the page are measurements of whatever tokens.css (and the page's curated override) currently say. */
(async function () {
'use strict';
const $$ = (s, el) => Array.from((el || document).querySelectorAll(s));
const byId = id => document.getElementById(id);
try { await Promise.all(['600 12px Inter', '400 12px Inter', '500 12px Inter', '400 11px "JetBrains Mono"', '500 11px "JetBrains Mono"'].map(f => document.fonts.load(f))); } catch (e) { /* fall back to system fonts */ }
const mctx = document.createElement('canvas').getContext('2d');
const tw = (t, f) => { mctx.font = f; return mctx.measureText(t).width; };
const FS = (w, s) => w + ' ' + s + 'px Inter', FM = (w, s) => w + ' ' + s + 'px "JetBrains Mono"';
const fsize = f => parseFloat(f.split(' ')[1]);
const cssFont = f => f.replace('Inter', 'var(--font-sans)').replace('"JetBrains Mono"', 'var(--font-mono)');

/* ---------- colour maths: OKLab, Machado 2009 (severity 1.0) in linear RGB, WCAG contrast ---------- */
const MACHADO = {
  protan: [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
  deutan: [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881]],
  tritan: [[1.255528, -0.076749, -0.178779], [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.303900]]
};
const s2lin = c => c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
const lin2s = c => { c = Math.max(0, Math.min(1, c)); return c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055; };
const lin = h => [1, 3, 5].map(i => s2lin(parseInt(h.slice(i, i + 2), 16) / 255));
const lin2hex = l => '#' + l.map(c => Math.round(lin2s(c) * 255).toString(16).padStart(2, '0')).join('');
function oklabL(l) {
  const a = Math.cbrt(0.4122214708 * l[0] + 0.5363325363 * l[1] + 0.0514459929 * l[2]), b = Math.cbrt(0.2119034982 * l[0] + 0.6806995451 * l[1] + 0.1073969566 * l[2]), c = Math.cbrt(0.0883024619 * l[0] + 0.2817188376 * l[1] + 0.6299787005 * l[2]);
  return [0.2104542553 * a + 0.7936177850 * b - 0.0040720468 * c, 1.9779984951 * a - 2.4285922050 * b + 0.4505937099 * c, 0.0259040371 * a + 0.7827717662 * b - 0.8086757660 * c];
}
const simLin = (l, k) => { const M = MACHADO[k], c = v => Math.max(0, Math.min(1, v)); return [0, 1, 2].map(i => c(M[i][0] * l[0] + M[i][1] * l[1] + M[i][2] * l[2])); };
const simHex = (h, k) => k ? lin2hex(simLin(lin(h), k)) : h;
const dE = (h1, h2, k) => { const a = oklabL(k ? simLin(lin(h1), k) : lin(h1)), b = oklabL(k ? simLin(lin(h2), k) : lin(h2)); return 100 * Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]); };
const okL = h => oklabL(lin(h))[0];
const relLum = h => { const l = lin(h); return 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2]; };
const contrast = (a, b) => { const x = relLum(a), y = relLum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
const VIS = [[null, 'Normal vision', 15], ['protan', 'Protanopia', 8], ['deutan', 'Deuteranopia', 8], ['tritan', 'Tritanopia', 8]];
const grade = (d, k) => d >= (k ? 8 : 15) ? 'ok' : k && d >= 6 ? 'warn' : 'err';   // 6 to 8 under colour blindness: legal only with a second cue
const gradeTag = g => '<span class="tag ' + g + '">' + (g === 'ok' ? '✓ passes' : g === 'warn' ? '! needs a second cue' : '✕ fails') + '</span>';
const gradeMark = g => g === 'ok' ? ' ✓' : g === 'warn' ? ' !' : ' ✕';

/* ---------- read the palettes actually in force (token = tokens.css untouched; cur = with this page's override) ---------- */
function readPal(theme, token) {
  const d = document.createElement('div'); d.dataset.theme = theme; if (token) d.dataset.pal = 'token'; d.style.display = 'none'; document.body.appendChild(d);
  const cs = getComputedStyle(d), g = n => cs.getPropertyValue(n).trim(), seq = (p, n) => Array.from({ length: n }, (_, i) => g('--' + p + (i + 1)));
  const o = { d: seq('d', 10), s: seq('s', 7), v: seq('v', 7), bg: g('--bg'), surface: g('--surface'), map: g('--map-bg'), ink: g('--map-ink'), accent: g('--accent') };
  d.remove(); return o;
}
const PAL = { light: { token: readPal('light', true), cur: readPal('light', false) }, dark: { token: readPal('dark', true), cur: readPal('dark', false) } };
const themeOf = el => el.closest('[data-theme]').dataset.theme;
function sepOf(p) {
  const out = {};
  for (const [k, , floor] of VIS) { let w = [1e9, 0, 0], below = 0; for (let i = 0; i < p.length; i++) for (let j = i + 1; j < p.length; j++) { const d = dE(p[i], p[j], k); if (d < w[0]) w = [d, i, j]; if (d < floor) below++; } out[k || 'normal'] = { worst: w[0], i: w[1], j: w[2], below }; }
  return out;
}

/* ---------- the sample map: adjacency of the top-level regions and the two colourings ---------- */
const TOPS = root.kids, NT = TOPS.length, ADJ = TOPS.map(() => []), EDGES = [];
for (let i = 0; i < NT; i++) for (let j = i + 1; j < NT; j++) { const gap = Math.hypot(TOPS[i].x - TOPS[j].x, TOPS[i].y - TOPS[j].y) - TOPS[i].r - TOPS[j].r; if (gap < 90) { ADJ[i].push(j); ADJ[j].push(i); EDGES.push([i, j]); } }
function dsatur(k) {                                    // proper colouring, most-constrained first, least-used colour first (balanced)
  const col = new Array(NT).fill(-1), used = new Array(k).fill(0);
  for (let step = 0; step < NT; step++) {
    let best = -1, bs = -1, bd = -1;
    for (let i = 0; i < NT; i++) { if (col[i] >= 0) continue; const sat = new Set(ADJ[i].map(j => col[j]).filter(c => c >= 0)).size; if (sat > bs || (sat === bs && ADJ[i].length > bd)) { best = i; bs = sat; bd = ADJ[i].length; } }
    const taken = new Set(ADJ[best].map(j => col[j])); let pick = -1;
    for (let c = 0; c < k; c++) if (!taken.has(c) && (pick < 0 || used[c] < used[pick])) pick = c;
    if (pick < 0) return null; col[best] = pick; used[pick]++;
  }
  return { col, used };
}
const MINK = [2, 3, 4, 5, 6].find(k => dsatur(k));       // fewest colours the greedy colouring manages with
const REUSE = dsatur(5).col;                            // colouring (b): the five-colour region set d1…d5
const bySize = TOPS.map((c, i) => i).sort((a, b) => TOPS[b].n - TOPS[a].n), HUE = new Array(NT);
bySize.forEach((i, rank) => { HUE[i] = rank < 9 ? rank + 1 : 10; });   // colouring (a): one hue each for the nine largest, d10 for the rest
function adjStats(theme, slotOf) {                      // weakest pair among regions that actually touch
  const P = PAL[theme].cur.d, o = {}; let same = 0;
  for (const [k] of VIS) { let w = 1e9, wi = null; for (const [i, j] of EDGES) { const a = slotOf(i), b = slotOf(j); if (a === b) { if (!k) same++; continue; } const d = dE(P[a], P[b], k); if (d < w) { w = d; wi = [i, j]; } } o[k || 'normal'] = { w, pair: wi }; }
  o.same = same; return o;
}

/* ---------- drawing a map specimen as SVG ---------- */
const num = n => Math.round(n).toLocaleString('en-US');
const f1 = x => (Math.round(x * 10) / 10).toFixed(1);
let UID = 0;
function wrap2(t) { const mid = t.length / 2; let best = -1; for (let i = 0; i < t.length; i++) if (t[i] === ' ' && (best < 0 || Math.abs(i - mid) < Math.abs(best - mid))) best = i; return best < 0 ? null : [t.slice(0, best), t.slice(best + 1)]; }
const lineW = line => line.reduce((s, sp, i) => s + tw(sp.t, sp.f) + (i ? 6 : 0), 0);
function fitBlock(lines, r) {
  const hs = lines.map(l => fsize(l[0].f) * 1.3), Ht = hs.reduce((a, b) => a + b, 0); let y = -Ht / 2;
  for (let i = 0; i < lines.length; i++) { const m = Math.max(Math.abs(y), Math.abs(y + hs[i])); if (m >= r) return false; if (lineW(lines[i]) > 2 * Math.sqrt(r * r - m * m) * .93) return false; y += hs[i]; }
  return true;
}
function truncate(t, f, maxW) { if (tw(t, f) <= maxW) return t; let s = t; while (s.length > 3 && tw(s + '…', f) > maxW) s = s.slice(0, -1); return s.replace(/[\s&·]+$/, '') + '…'; }

function drawMap(el, o) {
  o = o || {};
  const theme = themeOf(el), P = PAL[theme].cur, W = o.W || Math.round(el.clientWidth) || 520, H = o.H || 340, T = o.T || 84, uid = ++UID;
  const focus = o.at != null && o.at !== '' ? nodeById(o.at) : root, mode = o.color || 'reuse';
  const oldWin = WIN; if (o.win) WIN = o.win;
  let k, cx, cy, pd = null;
  if (focus === root) { k = Math.min(W / (2 * root.bw), H / (2 * root.bh)) * .94 * (o.zf || 1); cx = 0; cy = 0; } else { k = Math.min(W, H) / (2 * focus.r) * .8 * (o.zf || 1); cx = focus.x; cy = focus.y; }
  if (o.path) {
    pd = computePath(nodeById(o.path[0]), nodeById(o.path[1]), o.avoid ? 1 : 0); pd.hub = o.avoid ? null : pd.nodes[2];
    let x0 = 1e18, x1 = -1e18, y0 = 1e18, y1 = -1e18; for (const n of pd.nodes) { x0 = Math.min(x0, n.x); x1 = Math.max(x1, n.x); y0 = Math.min(y0, n.y); y1 = Math.max(y1, n.y); }
    k = Math.min((W - 130) / Math.max(1e-9, x1 - x0), (H - 110) / Math.max(1e-9, y1 - y0)); cx = (x0 + x1) / 2; cy = (y0 + y1) / 2;
  }
  cx -= (o.dx || 0) / k; cy -= (o.dy || 0) / k;
  const sx = x => (x - cx) * k + W / 2, sy = y => (y - cy) * k + H / 2;
  const items = [], dots = [], byDepth = []; let small = 0, addr = 0;
  const fade = o.fade || {};
  const visit = (m, a) => {
    const s = m.r * k, px = sx(m.x), py = sy(m.y); if (px + s < 0 || px - s > W || py + s < 0 || py - s > H) return;
    if (m.kind === 'address' || m.kind === 'more') { dots.push({ n: m, s, px, py, a }); if (m.kind === 'address') addr++; return; }
    if (s < 1.5) { small++; return; }
    let e = !m.rest && s > T && (o.maxDepth == null || m.depth <= o.maxDepth) ? 1 : 0; if (fade[m.id] != null) e = fade[m.id];
    items.push({ n: m, s, px, py, e, a }); if (e < .5) byDepth[m.depth] = (byDepth[m.depth] || 0) + 1;
    if (e > 0) for (const c of expand(m)) visit(c, a * e);
  };
  for (const c of root.kids) visit(c, 1);

  /* colour */
  const fillMode = mode === 'metric' || mode === 'div';
  const divIdx = n => { const v = n.intv; if (!v) return 4; let a = 0, b = 0; for (let m = 0; m < 6; m++) { a += v[m]; b += v[m + 6]; } const ch = a + b ? (b - a) / (a + b) : 0; return 4 + Math.max(-3, Math.min(3, Math.round(ch / (o.divStep || .03)))); };
  const fillIdx = n => mode === 'div' ? divIdx(n) : 2 + Math.min(4, Math.floor(n.u * 5));
  const fillVar = n => 'var(--' + (mode === 'div' ? 'v' : 's') + fillIdx(n) + ')', fillHex = n => (mode === 'div' ? P.v : P.s)[fillIdx(n) - 1];
  const col = n => mode === 'hue' ? 'var(--d' + HUE[n.top.idx] + ')' : mode === 'label' ? (n.cat === 4 ? 'var(--line-strong)' : 'var(--d' + [1, 2, 3, 10][n.cat] + ')') : mode === 'none' ? 'var(--line-strong)' : 'var(--d' + (REUSE[n.top.idx] + 1) + ')';
  const onFill = hex => relLum(hex) > .3 ? (theme === 'dark' ? 'var(--map-bg)' : 'var(--map-ink)') : (theme === 'dark' ? 'var(--map-ink)' : 'var(--map-bg)');

  /* what is selected, linked or on the path stays at full strength; the rest dims to 60% */
  const sel = o.select != null && o.select !== '' ? nodeById(o.select) : null, hotSet = new Set(); if (sel) hotSet.add(sel);
  let L = [], linkInfo = null; const linkEls = [], chips = [], rings = [];
  if (sel && o.links) {
    const all = linksOf(sel), tot = all.reduce((s, e) => s + e.w, 0); L = all.filter(l => !l.elsewhere).slice(0, o.linkCount || 12);
    const max = L.length ? L[0].w : 1, src = [sx(sel.x), sy(sel.y)], box = { x0: 8, y0: 8, x1: W - 8, y1: H - 8 }; let wsum = 0;
    for (let q = L.length - 1; q >= 0; q--) {
      const l = L[q]; wsum += l.w; hotSet.add(l.t); let ex = sx(l.t.x), ey = sy(l.t.y), off = false;
      if (ex < box.x0 || ex > box.x1 || ey < box.y0 || ey > box.y1) { let t = 1; const dx = ex - src[0], dy = ey - src[1]; if (dx > 0) t = Math.min(t, (box.x1 - src[0]) / dx); else if (dx < 0) t = Math.min(t, (box.x0 - src[0]) / dx); if (dy > 0) t = Math.min(t, (box.y1 - src[1]) / dy); else if (dy < 0) t = Math.min(t, (box.y0 - src[1]) / dy); ex = src[0] + dx * t; ey = src[1] + dy * t; off = true; }
      const dx = ex - src[0], dy = ey - src[1], bend = off ? .06 : .16, lw = Math.min(1 + 7 * Math.sqrt(l.w / max), Math.max(1.5, sel.r * k * .7));
      linkEls.push('<path d="M' + f1(src[0]) + ' ' + f1(src[1]) + 'Q' + f1((src[0] + ex) / 2 - dy * bend) + ' ' + f1((src[1] + ey) / 2 + dx * bend) + ' ' + f1(ex) + ' ' + f1(ey) + '" class="m-link" style="stroke-width:' + f1(lw) + '"/>');
      if (off && !o.noChips) chips.push({ x: ex, y: ey, t: shortTitle(l.t), w: num(l.w), c: fillMode ? fillVar(l.t) : col(l.t) }); else rings.push('<circle cx="' + f1(ex) + '" cy="' + f1(ey) + '" r="' + f1(Math.max(1.8, l.t.r * k) + 2) + '" class="m-hover"/>');
    }
    linkInfo = { shown: L.length, total: all.length, share: tot ? wsum / tot : 0, max, wsum, tot };
  }
  if (pd) for (const n of pd.nodes) hotSet.add(n);
  const dimOn = (sel || pd) && o.dim !== false;
  const isHot = n => { if (!dimOn) return true; for (let m = n; m; m = m.parent) if (hotSet.has(m)) return true; return false; };

  const defs = [], cold = [], hot = [], hatches = {};
  const hatch = c => { if (!hatches[c]) { hatches[c] = 'h' + uid + '-' + Object.keys(hatches).length; defs.push('<pattern id="' + hatches[c] + '" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><line x1="0" y1="0" x2="0" y2="6" style="stroke:' + c + ';stroke-width:1.2;stroke-opacity:.6"/></pattern>'); } return 'url(#' + hatches[c] + ')'; };
  const op = a => a < .995 ? ' opacity="' + a.toFixed(2) + '"' : '';
  const covers = it => it.e > .5 && [[0, 0], [W, 0], [0, H], [W, H]].every(q => Math.hypot(q[0] - it.px, q[1] - it.py) < it.s);
  const skel = o.skeleton || null;                      // ids of regions whose children have not arrived yet
  for (const it of items) {
    const n = it.n, A = isHot(n) ? hot : cold, c = col(n), g = 'cx="' + f1(it.px) + '" cy="' + f1(it.py) + '" r="' + f1(it.s) + '"';
    if (skel && n.depth >= 2 && skel.has(n.top.id)) { A.push('<circle ' + g + ' class="m-skel"/>'); continue; }
    let st;
    if (fillMode) st = it.e > .5 ? 'fill:none;stroke:var(--line-strong);stroke-width:1' : 'fill:' + fillVar(n) + ';stroke:var(--map-bg);stroke-width:1';
    else st = 'fill:' + c + ';fill-opacity:' + (mode === 'label' && n.cat === 4 || covers(it) ? 0 : n.kind === 'aggregate' ? .05 : (.12 - .07 * it.e).toFixed(3)) + ';stroke:' + c + ';stroke-width:' + (n.kind === 'aggregate' || it.s < 9 ? 1 : 1.5);
    A.push('<circle ' + g + ' style="' + st + '"' + (n.kind === 'subdivision' ? ' stroke-dasharray="4 3"' : '') + op(it.a) + '/>');
    if (n.kind === 'aggregate' && it.e < .5) A.push('<circle ' + g + ' fill="' + hatch(fillMode ? onFill(fillHex(n)) : c) + '" stroke="none"' + op(it.a) + '/>');
  }
  for (const d of dots) {
    const n = d.n, A = isHot(n) ? hot : cold, c = fillMode ? fillVar(n) : col(n);
    if (skel && skel.has(n.top.id)) continue;
    if (n.kind === 'more') { A.push('<circle cx="' + f1(d.px) + '" cy="' + f1(d.py) + '" r="' + f1(d.s) + '" fill="' + hatch(fillMode ? 'var(--ink-2)' : c) + '" style="stroke:' + (fillMode ? 'var(--ink-2)' : c) + ';stroke-width:1" stroke-dasharray="3 3"' + op(d.a) + '/>'); continue; }
    const r = d.ds = Math.max(1.8, d.s);
    A.push('<circle cx="' + f1(d.px) + '" cy="' + f1(d.py) + '" r="' + f1(r) + '" style="fill:' + c + (r > 3.2 ? ';stroke:var(--map-bg);stroke-width:1' : '') + '"' + op(d.a) + '/>');
  }
  const ring = (n, cls) => { const r = Math.max(1.8, n.r * k), g = 'cx="' + f1(sx(n.x)) + '" cy="' + f1(sy(n.y)) + '"'; return cls === 'sel' ? '<circle ' + g + ' r="' + f1(r + 3) + '" class="m-sel"/><circle ' + g + ' r="' + f1(r + 7) + '" class="m-sel"/>' : '<circle ' + g + ' r="' + f1(r + 2) + '" class="m-hover"/>'; };
  if (o.hover) rings.push(ring(nodeById(o.hover), 'hover'));
  if (sel && !pd) rings.push(ring(sel, 'sel'));

  /* path overlay */
  const overlay = [], labels = [], fixed = (o.avoidRects || []).slice();
  if (pd) {
    const mk = pd.nodes.map(n => [sx(n.x), sy(n.y)]);
    for (let it = 0; it < 40; it++) for (let i = 0; i < mk.length; i++) for (let j = i + 1; j < mk.length; j++) { let dx = mk[j][0] - mk[i][0], dy = mk[j][1] - mk[i][1], d = Math.hypot(dx, dy); if (d < 26) { if (d < .01) { dx = 1; dy = .3 * (j - i); d = Math.hypot(dx, dy); } const p = (26 - d) / d * .5; mk[i][0] -= dx * p; mk[i][1] -= dy * p; mk[j][0] += dx * p; mk[j][1] += dy * p; } }
    let dstr = ''; for (let i = 0; i + 1 < mk.length; i++) { const a = mk[i], b = mk[i + 1], dx = b[0] - a[0], dy = b[1] - a[1]; dstr += 'M' + f1(a[0]) + ' ' + f1(a[1]) + 'Q' + f1((a[0] + b[0]) / 2 - dy * .1) + ' ' + f1((a[1] + b[1]) / 2 + dx * .1) + ' ' + f1(b[0]) + ' ' + f1(b[1]); }
    overlay.push('<path d="' + dstr + '" class="m-path-case"/><path d="' + dstr + '" class="m-path"/>');
    mk.forEach((p, i) => {
      const end = i === 0 || i === mk.length - 1, n = pd.nodes[i], hub = pd.hub === n;
      overlay.push('<circle cx="' + f1(p[0]) + '" cy="' + f1(p[1]) + '" r="' + (end ? 10 : 8.5) + '" class="' + (end ? 'm-end' : 'm-hop') + '"/><text x="' + f1(p[0]) + '" y="' + f1(p[1] + 3.6) + '" text-anchor="middle" class="m-mk' + (end ? ' end' : '') + '">' + (i === 0 ? 'A' : i === mk.length - 1 ? 'B' : i) + '</text>');
      fixed.push([p[0] - 13, p[1] - 13, p[0] + 13, p[1] + 13]);
      labels.push({ x: p[0], y: p[1] + 21, lines: [[{ t: shortAddr(n), f: FM(500, 10.5), c: 'var(--map-ink)' }].concat(hub ? [{ t: 'exchange hub', f: FS(500, 10.5), c: 'var(--ink-2)' }] : [])], pri: 9e4 - i, alt: [-42], hot: true });
    });
  }

  /* labels: collect, place greedily by priority, draw */
  const showCount = o.labels !== 'names', onlyCount = o.labels === 'countsonly', isNamed = n => onlyCount ? false : o.namedFn ? o.namedFn(n) : (n.named || n.kind === 'subdivision');
  if (o.labels !== 'none') for (const it of items) {
    const n = it.n; if (it.a < .08 || (skel && n.depth >= 2 && skel.has(n.top.id))) continue;
    const named = isNamed(n), nm = shortTitle(n), cnt = num(n.n), isSel = n === sel, h = isHot(n);
    const ink = fillMode && it.e < .5 ? onFill(fillHex(n)) : 'var(--map-ink)', ink2 = fillMode && it.e < .5 ? ink : 'var(--ink-2)', halo = !(fillMode && it.e < .5);
    if (it.e > .02) {                                   // children are showing: the name steps aside to the top edge, with its count
      const y = it.py - it.s; if (y > 14 && y < H - 6) { const line = named ? [{ t: nm, f: FS(600, 12), c: 'var(--map-ink)' }] : []; if (!named || (showCount && it.s > 70)) line.push({ t: cnt, f: FM(400, 11), c: named ? 'var(--ink-2)' : 'var(--map-ink)' }); labels.push({ x: it.px, y, lines: [line], a: it.a * it.e, pri: 5000 + it.s + (isSel ? 1e4 : 0), plate: true, hot: h }); }
    }
    if (it.e < .98) {
      const lr = it.s, a = it.a * (1 - it.e); if (a < .06 || lr < 11) continue; if (!named && lr < 22 && !isSel) continue;
      const pri = (named ? 40 : 0) + lr + (n.depth === 1 ? 2000 : 0) + (isSel ? 1e4 : 0), base = lr > 130 ? 15 : lr > 64 ? 13 : lr > 36 ? 12 : 11; let v = null;
      for (const size of base > 11 ? [base, base - 1.5] : [base, 10.5]) {
        const f = FS(600, size), cf = FM(400, Math.max(10, size - 2)), w2 = named ? wrap2(nm) : null;
        const one = named ? [[{ t: nm, f, c: ink }]] : [[{ t: cnt, f: FM(400, Math.max(10, size - 1.5)), c: ink2 }]], two = w2 ? w2.map(t => [{ t, f, c: ink }]) : null, cl = named && showCount ? [[{ t: cnt, f: cf, c: ink2 }]] : null;
        for (const cand of [cl && one.concat(cl), cl && two && two.concat(cl), one, two]) if (cand && fitBlock(cand, lr)) { v = cand; break; }
        if (v) break;
      }
      if (!v && named && lr >= 26) { const f = FS(600, 11), t = truncate(nm, f, 2 * lr * .86); if (t.length >= 5) v = [[{ t, f, c: ink }]]; }
      if (v) labels.push({ x: it.px, y: it.py, lines: v, a, pri, halo, hot: h, alt: lr > 26 ? [-lr * .3, lr * .3] : null });
      else if (!o.noAbove && named && (n.depth === 1 || isSel) && it.s >= 6) labels.push({ x: it.px, y: it.py - it.s - 9, lines: [[{ t: nm, f: FS(600, 11), c: 'var(--map-ink)' }]], a, pri: pri - 30, halo: true, hot: h });
    }
  }
  const onPath = pd ? new Set(pd.nodes) : null, linkTop = new Set(L.slice(0, 6).map(l => l.t));
  if (o.labels !== 'none') for (const d of dots) {
    const n = d.n, isSel = n === sel; if (d.a < .3 || (onPath && onPath.has(n)) || (skel && skel.has(n.top.id))) continue;
    if (n.kind === 'more') { if (d.s > 15) { const t = '+' + num(n.n); if (tw(t, FM(500, 11)) < d.s * 1.8) labels.push({ x: d.px, y: d.py, lines: [[{ t, f: FM(500, 11), c: 'var(--map-ink)' }]], a: d.a, pri: 60, halo: true, hot: isHot(n) }); } continue; }
    if (d.ds < (o.addrMin || 7.5) && !isSel && n.id !== o.hover && !linkTop.has(n)) continue;
    const m = d.ds > 8 || isSel ? addrMeta(n) : null, line = [{ t: shortAddr(n), f: FM(isSel ? 500 : 400, 10.5), c: isSel || linkTop.has(n) ? 'var(--map-ink)' : 'var(--ink-2)' }];
    if (m && m.label && !onlyCount) line.push({ t: m.label, f: FS(400, 10.5), c: 'var(--muted)' });
    const gap = d.ds + (isSel ? 12 : 5);
    labels.push({ x: d.px + gap + lineW(line) / 2, y: d.py, own: d.n.id, altXY: w => [[-(gap + w / 2), gap + 5], [-(2 * gap + w), 0], [-(gap + w / 2), -(gap + 5)]], lines: [line], a: d.a, pri: d.ds + (isSel ? 1e4 : 0) + (n.id === o.hover ? 5e3 : 0) + (linkTop.has(n) ? 3e3 : 0), halo: true, hot: isHot(n) });
  }
  labels.sort((a, b) => b.pri - a.pri); const placed = fixed.slice(); let dropped = 0;
  for (const d of dots) if (d.ds >= 4 && d.a > .3) placed.push([d.px - d.ds, d.py - d.ds, d.px + d.ds, d.py + d.ds, d.n.id]);
  for (const l of labels) {
    let w = 0, h = 0; for (const line of l.lines) { w = Math.max(w, lineW(line)); h += fsize(line[0].f) * 1.3; } l.w = w; l.h = h; l.ok = false; const pad = l.plate ? 6 : 3;
    for (const c of [[0, 0]].concat((l.alt || []).map(dy => [0, dy]), l.altXY ? l.altXY(w) : [])) {
      const dx = c[0], dy = c[1], x0 = l.x + dx - w / 2 - pad, y0 = l.y + dy - h / 2 - 2, x1 = l.x + dx + w / 2 + pad, y1 = l.y + dy + h / 2 + 2; if (x1 < 4 || x0 > W - 4 || y1 < 0 || y0 > H) break;
      if (x0 < -w * .35 || x1 > W + w * .35) break;
      if (!placed.some(p => (p[4] === undefined || p[4] !== l.own) && x0 < p[2] && x1 > p[0] && y0 < p[3] && y1 > p[1])) { l.ok = true; l.x += dx; l.y += dy; placed.push([x0, y0, x1, y1]); break; }
    }
    if (!l.ok) dropped++;
  }
  const lab = [[], []];                                 // [dimmed, full]
  for (let q = labels.length - 1; q >= 0; q--) {
    const l = labels[q]; if (!l.ok) continue; const G = lab[!dimOn || l.hot ? 1 : 0]; let y = l.y - l.h / 2, s = '';
    if (l.plate) s += '<rect x="' + f1(l.x - l.w / 2 - 6) + '" y="' + f1(l.y - l.h / 2 - 1) + '" width="' + f1(l.w + 12) + '" height="' + f1(l.h + 2) + '" rx="3" class="m-plate"/>';
    for (const line of l.lines) { const lh = fsize(line[0].f) * 1.3; s += '<text x="' + f1(l.x) + '" y="' + f1(y + lh / 2 + fsize(line[0].f) * .35) + '" text-anchor="middle"' + (l.halo ? ' class="m-halo"' : '') + '>' + line.map((sp, i) => '<tspan' + (i ? ' dx="6"' : '') + ' style="font:' + cssFont(sp.f) + ';fill:' + sp.c + '">' + esc(sp.t) + '</tspan>').join('') + '</text>'; y += lh; }
    G.push(l.a != null && l.a < .995 ? '<g opacity="' + Math.min(1, l.a).toFixed(2) + '">' + s + '</g>' : s);
  }
  /* chips for link targets that are off screen */
  let chipS = ''; const chipRects = fixed.slice(); chips.slice(-6).reverse().forEach(c => { const f = FS(500, 11), fm = FM(500, 11), w = tw(c.t, f) + tw(c.w, fm) + 34, y0 = Math.max(6, Math.min(H - 26, c.y - 10)), horiz = c.y < 14 || c.y > H - 14; let x = Math.max(6, Math.min(W - w - 6, c.x - w / 2)), y = y0, ok = false; const x0 = x;
    for (const d of [0, 1, -1, 2, -2, 3, -3]) { x = horiz ? Math.max(6, Math.min(W - w - 6, x0 + d * (w * .6))) : x0; y = horiz ? y0 : Math.max(6, Math.min(H - 26, y0 + d * 24)); if (!chipRects.some(p => x < p[2] + 4 && x + w + 4 > p[0] && y < p[3] + 3 && y + 23 > p[1])) { ok = true; break; } } if (!ok) return; chipRects.push([x, y, x + w, y + 20]);
    chipS += '<rect x="' + f1(x) + '" y="' + f1(y) + '" width="' + f1(w) + '" height="20" rx="2" class="m-chip"/><circle cx="' + f1(x + 10) + '" cy="' + f1(y + 10) + '" r="3.5" style="fill:' + c.c + '"/><text x="' + f1(x + 18) + '" y="' + f1(y + 14) + '"><tspan style="font:' + cssFont(f) + ';fill:var(--ink-2)">' + esc(c.t) + '</tspan><tspan dx="6" style="font:' + cssFont(fm) + ';fill:var(--ink)">' + c.w + '</tspan></text>'; });

  el.innerHTML = '<svg viewBox="0 0 ' + W + ' ' + H + '" width="' + W + '" height="' + H + '" role="img" aria-label="' + esc(o.aria || 'Map specimen, invented data') + '"><defs>' + defs.join('') + '</defs>' +
    (cold.length ? '<g opacity=".6">' + cold.join('') + '</g>' : '') + '<g>' + linkEls.join('') + '</g><g>' + hot.join('') + '</g><g>' + rings.join('') + '</g>' + (lab[0].length ? '<g opacity=".6">' + lab[0].join('') + '</g>' : '') + '<g>' + overlay.join('') + '</g><g>' + lab[1].join('') + '</g><g>' + chipS + '</g>' + (o.extra ? o.extra({ W, H, sx, sy, k }) : '') + '</svg>';
  let lv = 1, mx = 0; byDepth.forEach((c, d) => { if (c > mx) { mx = c; lv = d; } });
  const info = { labels, W, H, k, sx, sy, items, dots, small, level: addr > mx ? 7 : lv, drawn: addr > mx ? addr : mx, link: linkInfo, pd, dropped, labelsShown: labels.length - dropped, focus,
    anchor: id => { const n = nodeById(id); return { x: sx(n.x), y: sy(n.y), r: Math.max(1.8, n.r * k) }; } };
  WIN = oldWin; return info;
}
function captionParts(info, o) {
  const lv = info.level, l6 = Math.min(lv, 6), parts = [];
  if (lv >= 7) parts.push('<b class="num">' + num(info.drawn) + '</b> of <span class="num">' + num(GRAPH.n) + '</span> addresses on screen');
  else parts.push('<b class="num">' + num(info.drawn) + '</b> of <span class="num">' + num(LT[l6]) + '</span> ' + LEVELNOUN[l6] + ' on screen' + (info.small ? ', <span class="num">' + num(info.small) + '</span> too small to draw' : ''));
  if (info.pd) parts.push('one path drawn, no other links');
  else if (info.link) parts.push('<b class="num">' + info.link.shown + '</b> of <span class="num">' + num(info.link.total) + '</span> links of the selection drawn (<span class="num">' + Math.round(info.link.share * 100) + '%</span> of its transfers)');
  else parts.push('no links drawn until you select something');
  if (o.win) parts.push('numbers are for <b>' + MONTHS[o.win[0]] + ' – ' + MONTHS[o.win[1]] + ' 2024</b>');
  if (o.color === 'label') parts.push('colour = most common label'); else if (o.color === 'metric') parts.push('colour = PageRank, fifths within a level'); else if (o.color === 'div') parts.push('colour = change in transfers, second half against first half of 2024');
  parts.push(lv >= 7 ? 'dot size = activity' : 'area = addresses');
  parts.push('position groups related communities, <b>distance is not a measurement</b>');
  return parts;
}
/* a framed specimen: canvas, optional breadcrumb + level indicator, caption strip */
function frame(el, o) {
  o = Object.assign({}, o); el.classList.add('mapframe'); el.innerHTML = '<div class="mf-canvas"></div>';
  const cvs = el.firstChild; let crumbW = 0;
  const focus = o.at != null && o.at !== '' ? nodeById(o.at) : root;
  if (o.chrome) {
    const ch = chain(o.crumbAt != null ? nodeById(o.crumbAt) : focus), c = document.createElement('div'); c.className = 'mf-crumbs';
    c.innerHTML = '<div class="crumbbar">' + ch.map((n, i) => (i ? '<span class="sep">›</span>' : '') + (i === ch.length - 1 ? '<b>' : '<span>') + esc(n.depth ? shortTitle(n) : 'All') + (i === ch.length - 1 ? '</b>' : '</span>')).join('') + '</div><div class="levelind"></div>';
    el.appendChild(c); crumbW = c.offsetWidth + 20;
    o.avoidRects = (o.avoidRects || []).concat([[0, 0, crumbW + 176, 44]]);
  }
  const info = drawMap(cvs, o);
  if (o.chrome) { const lv = info.level, li = el.querySelector('.levelind'); let t = '<span class="ticks">'; for (let i = 1; i <= 6; i++) t += '<i' + (i <= Math.min(lv, 6) ? ' class="on"' : '') + '></i>'; li.innerHTML = t + '<i class="leaf' + (lv >= 7 ? ' on' : '') + '"></i></span><span><b>' + (lv >= 7 ? 'Addresses' : 'Level <span class="num">' + Math.min(lv, 6) + '</span> of <span class="num">6</span>') + '</b>' + (lv >= 7 ? '' : ' · ' + (lv === 1 ? 'regions' : lv === 2 ? 'sub-regions' : 'communities')) + '</span>'; }
  if (o.caption !== false) { const cap = document.createElement('div'); cap.className = 'capstrip'; cap.innerHTML = (typeof o.caption === 'string' ? o.caption : captionParts(info, o).join(' <span class="sep">·</span> ')); el.appendChild(cap); }
  return info;
}
const html = (el, s) => { if (typeof el === 'string') el = byId(el); if (el) el.innerHTML = s; return el; };

/* =====================================================================
   1 · Categorical palette
   ===================================================================== */
const NAMES = ['Violet', 'Ochre', 'Olive', 'Indigo', 'Rose', 'Sky', 'Green', 'Petrol', 'Berry', 'Stone (other)'];
const TOKNAMES = ['Ink blue', 'Rust', 'Teal', 'Plum', 'Olive', 'Violet', 'Red', 'Blue', 'Green', 'Stone'];
$$('[data-swatches]').forEach(el => {
  const theme = themeOf(el), token = el.dataset.swatches === 'token', P = PAL[theme][token ? 'token' : 'cur'], names = token ? TOKNAMES : NAMES;
  let h = '<div class="sw-grid">';
  P.d.forEach((c, i) => {
    const onW = contrast(c, PAL.light.cur.surface), onK = contrast(c, PAL.light.cur.ink);
    h += '<div class="sw-cell"><div class="sw-fill" style="background:var(--d' + (i + 1) + ');color:' + (theme === 'dark' ? 'var(--bg)' : onW >= onK ? 'var(--surface)' : 'var(--ink)') + '"><span class="num">d' + (i + 1) + '</span></div>' +
      '<div class="sw-out" style="border-color:var(--d' + (i + 1) + ');background:color-mix(in srgb, var(--d' + (i + 1) + ') 12%, transparent)"></div>' +
      '<div class="sw-dots">' + [8, 5, 3].map(s => '<i style="width:' + s + 'px;height:' + s + 'px;background:var(--d' + (i + 1) + ')"></i>').join('') + '</div>' +
      '<div class="sw-name">' + names[i] + '</div><div class="num xs muted">' + c + '</div></div>';
  });
  h += '</div><div class="sim-rows">' + VIS.slice(1).map(v => '<div class="sim-row"><span class="xs muted">' + v[1] + '</span>' + P.d.map(c => '<i style="background:' + simHex(c, v[0]) + '"></i>').join('') + '</div>').join('') + '</div>';
  el.innerHTML = h;
});
function matrixHTML(P, k, floor) {
  let h = '<table class="mx"><tr><th></th>' + P.map((c, i) => '<th><i style="background:' + simHex(c, k) + '"></i></th>').join('') + '</tr>';
  P.forEach((a, i) => { h += '<tr><th><i style="background:' + simHex(a, k) + '"></i></th>' + P.map((b, j) => { if (i === j) return '<td class="dg"></td>'; const d = dE(a, b, k); const g = grade(d, k); return '<td class="' + (g === 'err' ? 'bad' : g === 'warn' ? 'near' : '') + '" title="d' + (i + 1) + ' against d' + (j + 1) + ': ' + d.toFixed(1) + '">' + Math.round(d) + '</td>'; }).join('') + '</tr>'; });
  return h + '</table>';
}
$$('[data-matrices]').forEach(el => {
  const [pal, theme] = el.dataset.matrices.split('-'), P = PAL[theme][pal === 'token' ? 'token' : 'cur'].d, S = sepOf(P);
  el.innerHTML = '<div class="mx-row" data-theme="' + theme + '"' + (pal === 'token' ? ' data-pal="token"' : '') + '>' + VIS.map(v => { const s = S[v[0] || 'normal']; return '<div class="mx-cell"><div class="mx-h"><b>' + v[1] + '</b> <span class="muted">floor <span class="num">' + v[2] + '</span></span></div>' + matrixHTML(P, v[0], v[2]) + '<div class="mx-f">' + gradeTag(grade(s.worst, v[0])) + ' worst <b class="num">' + s.worst.toFixed(1) + '</b> <span class="num muted">d' + (s.i + 1) + '–d' + (s.j + 1) + '</span><br><span class="num">' + s.below + '</span> of <span class="num">45</span> pairs under the floor</div></div>'; }).join('') + '</div>';
});
(function verdict() {
  const rows = [['Token palette', 'token'], ['Curated replacement', 'cur']].map(([name, key]) => ['light', 'dark'].map(th => { const S = sepOf(PAL[th][key].d), minC = Math.min(...PAL[th][key].d.map(c => contrast(c, PAL[th][key].map)));
    return '<tr><td>' + name + '</td><td>' + th + '</td>' + VIS.map(v => { const s = S[v[0] || 'normal']; const g = grade(s.worst, v[0]); return '<td class="num' + (g === 'err' ? ' fail' : '') + '">' + s.worst.toFixed(1) + gradeMark(g) + '<span class="muted"> d' + (s.i + 1) + '–d' + (s.j + 1) + '</span></td>'; }).join('') + '<td class="num">' + minC.toFixed(2) + ' : 1</td></tr>'; }).join('')).join('');
  html('verdict-table', '<table class="t"><thead><tr><th>Worst pair, ΔE</th><th>Theme</th><th class="num">Normal (floor 15)</th><th class="num">Protanopia (8)</th><th class="num">Deuteranopia (8)</th><th class="num">Tritanopia (8)</th><th class="num">Lowest contrast on map</th></tr></thead><tbody>' + rows + '</tbody></table>');
  const t = sepOf(PAL.light.token.d), td = sepOf(PAL.dark.token.d), c = sepOf(PAL.light.cur.d), cd = sepOf(PAL.dark.cur.d);
  html('verdict-text', 'The token palette fails. On light, d' + (t.protan.i + 1) + ' and d' + (t.protan.j + 1) + ' are <b class="num">' + t.protan.worst.toFixed(1) + '</b> apart under protanopia (the same colour), and <span class="num">' + t.normal.below + '</span> of <span class="num">45</span> pairs are under the normal-vision floor; on dark, d' + (td.protan.i + 1) + ' and d' + (td.protan.j + 1) + ' are <b class="num">' + td.protan.worst.toFixed(1) + '</b> apart. Its ten colours sit in one narrow lightness band, and lightness is most of what a red–green colour-blind reader has left. The curated set spreads lightness and keeps one hue per slot in both themes: worst pair <b class="num">' + Math.min(c.protan.worst, c.deutan.worst).toFixed(1) + '</b> light and <b class="num">' + Math.min(cd.protan.worst, cd.deutan.worst).toFixed(1) + '</b> dark under protanopia and deuteranopia, <b class="num">' + c.normal.worst.toFixed(1) + '</b> and <b class="num">' + cd.normal.worst.toFixed(1) + '</b> with normal vision.');
  const r5 = sepOf(PAL.light.cur.d.slice(0, 5)), r5d = sepOf(PAL.dark.cur.d.slice(0, 5)), r4 = sepOf(PAL.light.cur.d.slice(0, 4)), r4d = sepOf(PAL.dark.cur.d.slice(0, 4));
  const row = (n, a, b) => '<tr><td>' + n + '</td>' + [a, b].map(s => ['normal', 'protan', 'deutan', 'tritan'].map(k => '<td class="num">' + s[k].worst.toFixed(1) + '</td>').join('')).join('') + '</tr>';
  html('subset-table', '<table class="t"><thead><tr><th rowspan="2">Colours used together</th><th colspan="4">Light, worst pair (ΔE)</th><th colspan="4">Dark, worst pair (ΔE)</th></tr><tr>' + '<th class="num">Normal</th><th class="num">Protan</th><th class="num">Deutan</th><th class="num">Tritan</th>'.repeat(2) + '</tr></thead><tbody>' + row('d1–d4', r4, r4d) + row('d1–d5, the region set', r5, r5d) + row('d1–d10, all ten', c, cd) + '</tbody></table>');
})();
$$('[data-contrast]').forEach(el => {
  const key = el.dataset.contrast; let h = '<table class="t"><thead><tr><th>Slot</th><th>Light</th><th class="num">on page</th><th class="num">on surface</th><th class="num">on map</th><th>Dark</th><th class="num">on page</th><th class="num">on surface</th><th class="num">on map</th></tr></thead><tbody>';
  for (let i = 0; i < 10; i++) { h += '<tr><td><span class="num">d' + (i + 1) + '</span> ' + (key === 'cur' ? NAMES[i] : TOKNAMES[i]) + '</td>'; for (const th of ['light', 'dark']) { const P = PAL[th][key], c = P.d[i]; h += '<td><i class="chip-sw" style="background:' + c + '"></i><span class="num">' + c + '</span></td>' + [P.bg, P.surface, P.map].map(s => { const r = contrast(c, s); return '<td class="num' + (r < 3 ? ' fail' : '') + '">' + r.toFixed(2) + (r < 3 ? ' ✕' : '') + '</td>'; }).join(''); } h += '</tr>'; }
  el.innerHTML = h + '</tbody></table>';
});
(function changed() {
  let h = '<table class="t"><thead><tr><th>Slot</th><th>Token, light</th><th>Curated, light</th><th>Token, dark</th><th>Curated, dark</th><th>What changed</th></tr></thead><tbody>';
  const why = ['Was the accent itself. Now a violet that is ' + dE(PAL.light.cur.d[0], PAL.light.cur.accent).toFixed(1) + ' (light) and ' + dE(PAL.dark.cur.d[0], PAL.dark.cur.accent).toFixed(1) + ' (dark) away from it.', 'Rust became a dark ochre on light and an amber on dark: the warm slot is told apart by lightness.', 'Teal became olive, light on light paper and dark on the dark canvas.', 'Plum became a deep indigo on light and a pale lavender on dark.', 'Olive became rose.', 'Second violet removed. Sky takes its place.', 'Red removed: it was the error colour. Green takes its place.', 'Blue became a dark petrol.', 'Second green became berry.', 'Stone stays the neutral for “other”, slightly lighter.'];
  for (let i = 0; i < 10; i++) h += '<tr><td class="num">d' + (i + 1) + '</td>' + [['light', 'token'], ['light', 'cur'], ['dark', 'token'], ['dark', 'cur']].map(([th, k]) => '<td><span class="chip-sw on-' + th + '" style="background:' + PAL[th][k].d[i] + '"></span><span class="num">' + PAL[th][k].d[i] + '</span></td>').join('') + '<td class="small ink2">' + why[i] + '</td></tr>';
  html('changed-table', h + '</tbody></table>');
})();

/* =====================================================================
   2 · Region colouring
   ===================================================================== */
$$('[data-regionmap]').forEach(el => frame(el, { H: 372, T: 50, color: el.dataset.regionmap, caption: false, labels: 'names' }));
(function regionEvidence() {
  const el = byId('adjacency'); if (!el) return;
  frame(el, { H: 330, maxDepth: 0, color: 'reuse', caption: false, labels: 'names', extra: v => EDGES.map(([i, j]) => { const a = TOPS[i], b = TOPS[j], d = Math.hypot(b.x - a.x, b.y - a.y), ux = (b.x - a.x) / d, uy = (b.y - a.y) / d; return '<line x1="' + f1(v.sx(a.x + ux * a.r)) + '" y1="' + f1(v.sy(a.y + uy * a.r)) + '" x2="' + f1(v.sx(b.x - ux * b.r)) + '" y2="' + f1(v.sy(b.y - uy * b.r)) + '" class="m-adj"/>'; }).join('') });
  const used = dsatur(5).used, rows = [], pn = p => p ? TOPS[p[0]].name + ' / ' + TOPS[p[1]].name : '';
  const line = (name, th, ncol, st) => { const w = Math.min(st.protan.w, st.deutan.w), g = grade(w, 1); return '<tr><td>' + name + '</td><td>' + th + '</td><td class="num">' + ncol + '</td><td class="num' + (st.same ? ' fail' : '') + '">' + st.same + (st.same ? ' ✕' : ' ✓') + '</td><td class="num">' + st.normal.w.toFixed(1) + '</td><td class="num' + (g === 'err' ? ' fail' : '') + '">' + w.toFixed(1) + gradeMark(g) + '</td><td class="small">' + pn(st.protan.w < st.deutan.w ? st.protan.pair : st.deutan.pair) + '</td></tr>'; };
  for (const th of ['light', 'dark']) { rows.push(line('(a) one hue per region', th, 10, adjStats(th, i => HUE[i] - 1))); rows.push(line('(b) five colours, reused', th, used.filter(x => x).length, adjStats(th, i => REUSE[i]))); }
  html('region-table', '<table class="t"><thead><tr><th>Colouring</th><th>Theme</th><th class="num">Colours on screen</th><th class="num">Touching pairs with the same colour</th><th class="num">Weakest other touching pair, normal (ΔE)</th><th class="num">Same, protan or deutan (ΔE)</th><th>Which pair</th></tr></thead><tbody>' + rows.join('') + '</tbody></table>');
  html('region-facts', 'The sample map has <b class="num">' + NT + '</b> top-level regions and <b class="num">' + EDGES.length + '</b> pairs that touch. Greedy colouring (most constrained region first, least-used colour first) needs <b class="num">' + MINK + '</b> colours; with the five of the region set it uses them <span class="num">' + used.join(' · ') + '</span> times. With one hue per region, <span class="num">' + HUE.filter(x => x === 10).length + '</span> of the <span class="num">' + NT + '</span> regions already fall back to the neutral d10, because there are only nine hues.');
})();

/* =====================================================================
   3 · Sequential and diverging
   ===================================================================== */
$$('[data-ramp]').forEach(el => {
  const [kind, key] = el.dataset.ramp.split('-'), theme = themeOf(el), R = PAL[theme][key === 'token' ? 'token' : 'cur'][kind === 'seq' ? 's' : 'v'], Ls = R.map(okL), dL = Ls.slice(1).map((l, i) => l - Ls[i]);
  const abs = dL.map(Math.abs), W = 7 * 52, hh = 56, lo = Math.min(...Ls) - .03, hi = Math.max(...Ls) + .03, y = l => 4 + (hh - 22) * (1 - (l - lo) / (hi - lo));
  let verdict;
  if (kind === 'seq') { const spread = Math.max(...abs) - Math.min(...abs); verdict = '<span class="tag ' + (spread > .02 ? 'warn' : 'ok') + '">' + (spread > .02 ? '! uneven' : '✓ even') + '</span> steps <span class="num">' + Math.min(...abs).toFixed(3) + '</span> to <span class="num">' + Math.max(...abs).toFixed(3) + '</span>'; }
  else { const asym = Math.max(Math.abs(Ls[0] - Ls[6]), Math.abs(Ls[1] - Ls[5]), Math.abs(Ls[2] - Ls[4])), mid = Math.min(dE(R[2], R[3]), dE(R[4], R[3])); verdict = '<span class="tag ' + (asym > .02 || mid < 8 ? 'warn' : 'ok') + '">' + (asym > .02 || mid < 8 ? '! uneven' : '✓ balanced') + '</span> arms differ by up to <span class="num">' + asym.toFixed(3) + '</span> · weakest step next to the midpoint ΔE <span class="num">' + mid.toFixed(1) + '</span>'; }
  el.innerHTML = '<div class="ramp-h"><b>' + (kind === 'seq' ? 'Sequential' : 'Diverging') + '</b> <span class="muted">' + (key === 'token' ? 'tokens.css' : 'curated') + '</span></div><div class="ramp">' + R.map((c, i) => '<div><i style="background:' + c + '"></i><span class="num">' + (kind === 'seq' ? 's' : 'v') + (i + 1) + '</span><span class="num muted">' + Ls[i].toFixed(2) + '</span></div>').join('') + '</div>' +
    '<svg class="ramp-l" viewBox="0 0 ' + W + ' ' + hh + '" width="' + W + '" height="' + hh + '" role="img" aria-label="Lightness of each step"><polyline points="' + Ls.map((l, i) => (26 + i * 52) + ',' + f1(y(l))).join(' ') + '" class="c-line"/>' + Ls.map((l, i) => '<circle cx="' + (26 + i * 52) + '" cy="' + f1(y(l)) + '" r="2.5" class="c-dot"/>').join('') + dL.map((d, i) => '<text x="' + (52 + i * 52) + '" y="' + f1(Math.min(hh - 2, (y(Ls[i]) + y(Ls[i + 1])) / 2 + 17)) + '" text-anchor="middle" class="c-tick">' + (d > 0 ? '+' : '−') + Math.abs(d).toFixed(2).slice(1) + '</text>').join('') + '</svg><div class="ramp-v xs">' + verdict + '</div>';
});
$$('[data-metricmap]').forEach(el => {
  const kind = el.dataset.metricmap, info = frame(el, { H: 372, T: 50, color: kind, caption: false, labels: 'names', divStep: .03 });
  const lg = document.createElement('div'); lg.className = 'mf-legend strip';
  if (kind === 'metric') lg.innerHTML = '<div class="lg-t">PageRank, summed over members</div><div class="lg-scale"><div class="lg-steps">' + [2, 3, 4, 5, 6].map(i => '<i style="background:var(--s' + i + ')"></i>').join('') + '</div><div class="lg-ticks num"><span>0</span><span>20</span><span>40</span><span>60</span><span>80</span><span>100</span></div></div><div class="lg-n">percentile among regions of the same level</div>';
  else lg.innerHTML = '<div class="lg-t">Change in transfers, second half against first</div><div class="lg-scale"><div class="lg-steps seven">' + [1, 2, 3, 4, 5, 6, 7].map(i => '<i style="background:var(--v' + i + ')"></i>').join('') + '</div><div class="lg-ticks seven num"><span>−7.5%</span><span>−4.5%</span><span>−1.5%</span><span>+1.5%</span><span>+4.5%</span><span>+7.5%</span></div></div><div class="lg-n">middle class: no change, within ±<span class="num">1.5</span>%</div>';
  el.insertBefore(lg, el.children[1] || null);
});

/* =====================================================================
   4 · Anatomy
   ===================================================================== */
function callouts(el, info, list) {
  const svg = el.querySelector('svg'), ns = 'http://www.w3.org/2000/svg', g = document.createElementNS(ns, 'g'); let s = '';
  for (const c of list) { const a = c.at(info); if (!a) continue; const mx = a[0] + c.dx, my = a[1] + c.dy, d = Math.hypot(c.dx, c.dy), ux = c.dx / d, uy = c.dy / d;
    s += '<line x1="' + f1(a[0]) + '" y1="' + f1(a[1]) + '" x2="' + f1(mx - ux * 10) + '" y2="' + f1(my - uy * 10) + '" class="co-line"/><circle cx="' + f1(a[0]) + '" cy="' + f1(a[1]) + '" r="2" class="co-dot"/><circle cx="' + f1(mx) + '" cy="' + f1(my) + '" r="10" class="co-mk"/><text x="' + f1(mx) + '" y="' + f1(my + 3.8) + '" text-anchor="middle" class="co-n">' + c.n + '</text>'; }
  g.innerHTML = s; svg.appendChild(g);
}
const edge = (id, deg, grow) => info => { const a = info.anchor(id), t = deg * Math.PI / 180; return [a.x + (a.r + (grow || 0)) * Math.cos(t), a.y + (a.r + (grow || 0)) * Math.sin(t)]; };
const HOVA = '0.8';                                      // Cold storage, hovered
function anatomyA(el, withCallouts) {
  const agg = root.kids[0].kids.find(c => c.kind === 'aggregate'), o = { H: withCallouts ? 640 : 400, at: '0', zf: withCallouts ? 1.1 : 1.1, dx: withCallouts ? 70 : 40, dy: withCallouts ? 18 : 14, T: withCallouts ? 70 : 43, hover: HOVA, chrome: true, crumbAt: '0' };
  const info = frame(el, o); if (!withCallouts) return info;
  const cv = el.querySelector('.mf-canvas'), box = el.getBoundingClientRect(), rel = q => { const r = el.querySelector(q).getBoundingClientRect(); return { x0: r.left - box.left, x1: r.right - box.left, y1: r.bottom - box.top }; };
  const tip = document.createElement('div'); tip.className = 'pop mf-tip'; const h = info.anchor(HOVA), hn = nodeById(HOVA); tip.innerHTML = '<b>' + esc(hn.name) + '</b><br><span class="muted">Found community · </span><span class="num">' + num(hn.n) + '</span> addresses'; tip.style.left = (h.x + h.r * .7 + 6) + 'px'; tip.style.top = (h.y + h.r * .7 + 6) + 'px'; cv.appendChild(tip);
  const parts = expand(nodeById('0.0')).slice().sort((a, b) => a.x - b.x), cb = rel('.crumbbar'), lb = rel('.levelind');
  callouts(cv, info, [
    { n: 1, at: i => { const a = i.anchor('0'); return [a.x + a.r + 46, a.y + 150]; }, dx: 0, dy: 36 },
    { n: 2, at: edge('0', -128), dx: -30, dy: -26 },
    { n: 3, at: edge('0.3', -50), dx: 26, dy: -28 },
    { n: 4, at: edge(parts[0].id, 180), dx: -38, dy: 16 },
    { n: 5, at: edge(agg.id, 160, -6), dx: -44, dy: 4 },
    { n: 6, at: edge('0.0', 50), dx: 22, dy: 32 },
    { n: 7, at: i => { const a = i.anchor('0.3'); return [a.x + 30, a.y + 14]; }, dx: 50, dy: 44 },
    { n: 9, at: edge(HOVA, 190, 2), dx: -34, dy: 14 },
    { n: 12, at: i => [(cb.x0 + cb.x1) / 2, cb.y1 + 1], dx: -16, dy: 36 },
    { n: 13, at: i => [(lb.x0 + lb.x1) / 2, lb.y1 + 1], dx: 22, dy: 34 },
    { n: 14, at: i => [i.W * .5 + 120, i.H - 3], dx: 26, dy: -32 }
  ]);
  return info;
}
function anatomyB(el, withCallouts) {
  const leaf = nodeById('1.1.1.1.1.1'), K = expand(leaf), sel = K[0].id, more = K.find(c => c.kind === 'more'), hov = K[7].id;
  const info = frame(el, { H: withCallouts ? 580 : 400, at: leaf.id, zf: withCallouts ? 1.7 : 1.45, dy: withCallouts ? -52 : -40, T: 84, select: sel, links: true, linkCount: 9, hover: hov, addrMin: 10.5, chrome: true, crumbAt: leaf.id });
  if (!withCallouts) return info;
  const tgt = linksOf(nodeById(sel)).filter(l => !l.elsewhere && l.t.parent === leaf).map(l => l.t), skip = new Set(tgt.map(t => t.id).concat([sel, hov]));
  const inView = d => d.px > 120 && d.px < info.W - 160 && d.py > 90 && d.py < info.H - 90 && !skip.has(d.n.id) && d.n.kind === 'address';
  const mid = info.dots.filter(d => inView(d) && d.ds > 8.5 && d.px > info.W * .6).sort((a, b) => b.ds - a.ds)[0], small = info.dots.filter(d => inView(d) && d.ds < 7.5 && d.px < info.W * .4)[0];
  const chipEls = el.querySelectorAll('.mf-canvas svg .m-chip'), chip = chipEls[chipEls.length - 1];
  callouts(el.querySelector('.mf-canvas'), info, [
    mid && { n: 15, at: edge(mid.n.id, -50), dx: 20, dy: -30 },
    small && { n: 16, at: edge(small.n.id, 200), dx: -30, dy: 18 },
    { n: 8, at: edge(sel, 140, 7), dx: -26, dy: 30 },
    { n: 9, at: edge(hov, 200, 2), dx: -32, dy: -18 },
    { n: 10, at: i => { const a = i.anchor(sel); return [a.x - 230, a.y - 9]; }, dx: 0, dy: -34 },
    { n: 11, at: edge(more.id, -30), dx: 30, dy: -24 },
    { n: 17, at: i => { const a = i.anchor(tgt[1].id); return [a.x + a.r + 52, a.y + 7]; }, dx: 34, dy: 26 },
    chip && { n: 18, at: i => [+chip.getAttribute('x') + 40, +chip.getAttribute('y')], dx: -6, dy: -34 }
  ].filter(Boolean));
  return info;
}
$$('[data-anatomy]').forEach(el => { const [w, c] = el.dataset.anatomy.split('-'); (w === 'a' ? anatomyA : anatomyB)(el, c === 'full'); });
html('width-legend', (function () { const ws = [.01, .0625, .25, .5, 1].map(p => [1 + 7 * Math.sqrt(p), Math.round(p * 100) + '%']); return '<svg viewBox="0 0 520 64" width="520" height="64" role="img" aria-label="Link widths">' + ws.map((w, i) => '<path d="M' + (10 + i * 100) + ' 26Q' + (45 + i * 100) + ' 8 ' + (80 + i * 100) + ' 26" class="m-link" style="stroke-width:' + w[0].toFixed(2) + '"/><text x="' + (45 + i * 100) + '" y="46" text-anchor="middle" class="c-tick in">' + w[1] + '</text><text x="' + (45 + i * 100) + '" y="60" text-anchor="middle" class="c-tick">' + w[0].toFixed(1) + ' px</text>').join('') + '</svg>'; })());

/* =====================================================================
   5 · Zoom levels
   ===================================================================== */
(function zoom() {
  const el = byId('zoomrow'); if (!el) return;
  const steps = [
    { o: { maxDepth: 0, labels: 'names', noAbove: true }, t: 'Whole graph', a: '12 regions, each a plain disc. At this size only the largest can carry a name; the rest answer to hover.', d: 'Nothing is hidden yet.' },
    { o: { at: '1', zf: .9, T: 60 }, t: 'A region opens', a: 'Past the threshold a region shows its sub-regions; its own name steps to the top edge with its count.', d: 'The region’s centred label; its tint drops from 12% to 5%.' },
    { o: { at: '1.1', zf: 1.1, T: 60 }, t: 'A sub-region opens', a: 'Communities inside “Lending”, named where a label exists, counted where not.', d: 'Siblings of the region leave the screen; the breadcrumb keeps them one click away.' },
    { o: { at: '1.1.1', zf: 1.1, T: 60 }, t: 'Communities', a: 'Unnamed communities: outline, tint and exact count only. Aggregates stay hatched.', d: 'Names: none exist at this depth in this graph.' },
    { o: { at: '1.1.1.1.1.1', zf: 1.7, dy: -20, T: 60, addrMin: 8 }, t: 'Single addresses', a: 'Dots sized by activity, the busiest labelled with a truncated address; “+k more” stands for the rest.', d: 'Region outlines of the parents; you cannot zoom further.' }
  ];
  el.innerHTML = steps.map((s, i) => '<figure class="zl"><div class="zl-map" id="zl' + i + '"></div><figcaption><div class="zl-lv"></div><h4>' + s.t + '</h4><p class="small ink2"><b>Appears.</b> ' + s.a + '</p><p class="small ink2"><b>Disappears.</b> ' + s.d + '</p></figcaption></figure>').join('');
  steps.forEach((s, i) => { const f = byId('zl' + i), info = frame(f, Object.assign({ H: 250, caption: false }, s.o)), lv = info.level; let t = '<span class="levelind"><span class="ticks">'; for (let q = 1; q <= 6; q++) t += '<i' + (q <= Math.min(lv, 6) ? ' class="on"' : '') + '></i>'; f.parentNode.querySelector('.zl-lv').innerHTML = t + '<i class="leaf' + (lv >= 7 ? ' on' : '') + '"></i></span><b>' + (lv >= 7 ? 'Addresses' : 'Level <span class="num">' + Math.min(lv, 6) + '</span> of <span class="num">6</span>') + '</b></span>'; });
  const hv = byId('handover'); if (!hv) return;
  hv.innerHTML = [0, .5, 1].map((e, i) => '<figure class="zl"><div class="zl-map" id="ho' + i + '"></div><figcaption><h4>' + ['Just before', 'Half-way, 95 ms', 'After 190 ms'][i] + '</h4><p class="small ink2">' + ['Radius 80 px: one disc, name and count centred, tint 12%.', 'Children fade in while the centred label fades out; the top label fades in. Nothing moves.', 'Radius 88 px: children at full strength, parent tint 5%, name and count on the top edge.'][i] + '</p></figcaption></figure>').join('');
  [0, .5, 1].forEach((e, i) => frame(byId('ho' + i), { H: 230, at: '4', zf: .82 + i * .04, T: 60, fade: { '4': e }, caption: false, maxDepth: 1 }));
})();

/* =====================================================================
   6 · States
   ===================================================================== */
(function states() {
  const el = byId('states'); if (!el) return;
  const leaf = nodeById('1.1.1.1.1.1'), LK = expand(leaf), hub = firstAddr(kidByName(root.kids[0], 'Hot wallets')), A = descend(root.kids[2], 'stateA', 2), B = descend(root.kids[4], 'stateB', 2);
  const S = [
    { t: 'Nothing selected', n: 'The resting state. Every region at full strength, no links, no rings.', o: { T: 60 } },
    { t: 'Region selected', n: 'Double ink ring; everything else drops to 60%. The panel carries the numbers.', o: { T: 60, select: '2' } },
    { t: 'Node selected, with its links', n: 'Links appear only now: ink bands at 55%, width by weight. Targets get a single ring.', o: { at: leaf.id, T: 60, select: LK[0].id, links: true, linkCount: 8, addrMin: 9 } },
    { t: 'Path between two nodes', n: 'One ink line with a canvas-coloured casing, lettered ends, numbered hops. This one runs through an exchange hub, and says so.', o: { T: 60, path: [A.id, B.id] } },
    { t: 'Path, avoiding hubs', n: 'Same ends with hubs excluded: longer, and more informative. The caption names what was excluded.', o: { T: 60, path: [A.id, B.id], avoid: true }, cap: 'one path drawn, <span class="num">5</span> hops · <span class="num">1,204</span> hub addresses excluded · position groups related communities, <b>distance is not a measurement</b>' },
    { t: 'Time window active', n: 'The map does not move; counts and links are for the window, and the caption says which.', o: { T: 60, select: '4', links: true, linkCount: 6, win: [2, 4] }, hist: true },
    { t: 'Coloured by attribute', n: 'Three categories get a hue, the rest fold into “other”; unlabelled regions keep an outline and no tint.', o: { T: 60, color: 'label' }, legend: 'label' },
    { t: 'Coloured by metric', n: 'Fills carry the value, outlines go neutral, labels switch ink to stay readable.', o: { T: 60, color: 'metric' }, legend: 'metric' },
    { t: 'Loading tiles', n: 'What is known is drawn; regions still loading show neutral discs with no numbers. Nothing is guessed.', o: { T: 60, skeleton: new Set(['1', '3', '5', '7', '11']) }, cap: '<b class="num">7</b> of <span class="num">12</span> regions loaded · loading <span class="num">5</span> more · area = addresses' },
    { t: 'Too dense to draw links', n: 'The honest fallback: the heaviest links as bands, the remainder as a count on the ring. No hairball.', o: { at: hub.parent.id, T: 60, select: hub.id, links: true, linkCount: 6, addrMin: 99 }, dense: true },
    { t: 'Empty graph', n: 'Says what was read and what to do next.', empty: true },
    { t: 'Error loading', n: 'What happened, why, and the ways out as commands that can be copied.', error: true }
  ];
  el.innerHTML = S.map((s, i) => '<figure class="st"><h4><span class="num muted">' + (i + 1) + '</span> ' + s.t + '</h4><div class="st-map" id="st' + i + '"></div><figcaption class="small ink2">' + s.n + '</figcaption></figure>').join('');
  S.forEach((s, i) => {
    const f = byId('st' + i);
    if (s.empty) { f.classList.add('mapframe'); f.innerHTML = '<div class="mf-canvas mf-msg"><div><svg width="40" height="40" viewBox="0 0 32 32" aria-hidden="true"><circle cx="16" cy="16" r="13" class="m-ghost"/></svg><h4>This graph is empty</h4><p class="small ink2">The file was read: a header and <span class="num">0</span> rows. There is nothing to place on a map.</p><p><span class="btn sm">Open another file</span></p></div></div><div class="capstrip"><b class="num">0</b> of <span class="num">0</span> nodes on screen <span class="sep">·</span> nothing omitted</div>'; return; }
    if (s.error) { f.classList.add('mapframe'); f.innerHTML = '<div class="mf-canvas mf-msg"><div><span class="tag err">✕ Error</span><h4>The map could not be loaded</h4><p class="small ink2">The map tiles of <code>ethereum-2024.eg</code> do not match the graph: the graph was rebuilt on <span class="num">24 Nov 2026</span>, the tiles are older.</p><pre><code>everygraph map ethereum-2024.eg --rebuild</code></pre><p class="small ink2">About <span class="num">11</span> min. The graph itself is intact; queries still work.</p><p><span class="btn sm">Rebuild the map</span> <span class="btn sm quiet">Copy command</span></p></div></div><div class="capstrip">nothing on screen <span class="sep">·</span> the graph has <span class="num">212,438,917</span> addresses</div>'; return; }
    const o = Object.assign({ H: 262, caption: s.cap || undefined }, s.o), info = frame(f, o);
    if (s.dense) { const a = info.anchor(s.o.select), L = info.link, svg = f.querySelector('svg'), g = document.createElementNS('http://www.w3.org/2000/svg', 'g'), t = '+' + num(L.total - L.shown) + ' links not drawn', w = tw(t, FM(500, 10.5)) + 14;
      g.innerHTML = '<circle cx="' + f1(a.x) + '" cy="' + f1(a.y) + '" r="' + f1(a.r + 13) + '" class="m-fan"/><rect x="' + f1(a.x - w / 2) + '" y="' + f1(a.y + a.r + 20) + '" width="' + f1(w) + '" height="18" rx="2" class="m-chip"/><text x="' + f1(a.x) + '" y="' + f1(a.y + a.r + 32.5) + '" text-anchor="middle" style="font:' + cssFont(FM(500, 10.5)) + ';fill:var(--ink)">' + t + '</text>'; svg.appendChild(g);
      f.querySelector('.capstrip').innerHTML = '<b class="num">' + L.shown + '</b> of <span class="num">' + num(L.total) + '</span> links of the selection drawn (<span class="num">' + Math.round(L.share * 100) + '%</span> of its transfers) <span class="sep">·</span> the other <span class="num">' + num(L.total - L.shown) + '</span> are listed in the panel, not drawn <span class="sep">·</span> dot size = activity'; }
    if (s.hist) { const h = document.createElement('div'); h.className = 'mf-hist'; const v = root.intv, mx = Math.max(...v); h.innerHTML = v.map((x, m) => '<i class="' + (m >= 2 && m <= 4 ? 'in' : '') + '" style="height:' + (100 * x / mx).toFixed(0) + '%"></i>').join('') + '<span class="mf-win" style="left:' + (2 / 12 * 100) + '%;width:' + (3 / 12 * 100) + '%"></span>'; f.insertBefore(h, f.querySelector('.capstrip')); }
    if (s.legend) { const lg = document.createElement('div'); lg.className = 'mf-legend strip'; lg.innerHTML = s.legend === 'metric' ? '<div class="lg-t">PageRank</div><div class="lg-scale"><div class="lg-steps">' + [2, 3, 4, 5, 6].map(q => '<i style="background:var(--s' + q + ')"></i>').join('') + '</div><div class="lg-ticks num"><span>0</span><span>20</span><span>40</span><span>60</span><span>80</span><span>100</span></div></div><div class="lg-n">percentile</div>' : '<div class="lg-cats">' + [['Exchange', 'd1'], ['DeFi', 'd2'], ['Stablecoin', 'd3'], ['Other', 'd10']].map(c => '<span class="lg-r"><i style="border-color:var(--' + c[1] + ');background:color-mix(in srgb,var(--' + c[1] + ') 12%,transparent)"></i>' + c[0] + '</span>').join('') + '<span class="lg-r"><i style="border-color:var(--line-strong)"></i>None</span></div>'; f.insertBefore(lg, f.children[1]); }
  });
})();

/* =====================================================================
   7 · Labels
   ===================================================================== */
(function labelsSec() {
  const few = new Set(['0', '2', '0.0', '0.3', '2.0']);
  const set = [['lab-counts', { T: 46 }], ['lab-names', { T: 46, labels: 'names' }], ['lab-honest', { T: 46, namedFn: n => few.has(n.id) }], ['lab-honest-zoom', { at: '3', zf: 1.15, T: 60, namedFn: n => few.has(n.id) }]];
  for (const [id, o] of set) { const el = byId(id); if (!el) continue; const info = frame(el, Object.assign({ H: 330, caption: false }, o)); const c = byId(id + '-n'); if (c) c.innerHTML = '<span class="num">' + info.labelsShown + '</span> labels placed, <span class="num">' + info.dropped + '</span> dropped for collisions'; }
  const hz = byId('halo-demo'); if (hz) { const mk = (halo) => { const d = document.createElement('div'); hz.appendChild(d); const info = frame(d, { H: 230, at: '0', zf: 2.5, dx: 30, dy: -50, T: 150, select: '0.9', links: true, linkCount: 14, dim: false, caption: false, noChips: true }); if (!halo) d.querySelectorAll('.m-halo').forEach(t => t.classList.remove('m-halo')); const c = document.createElement('div'); c.className = 'xs muted cap2'; c.textContent = halo ? 'With the 3 px halo in the canvas colour' : 'Without the halo: links cut through the text'; d.appendChild(c); }; mk(true); mk(false); }
})();

/* =====================================================================
   8 · Small charts
   ===================================================================== */
(function charts() {
  const hs = byId('hist'); if (hs) {
    const v = root.intv, mx = Math.max(...v), Wd = hs.clientWidth || 700, Hh = 150, pl = 54, pb = 24, pt = 10, pw = Wd - pl - 8, bw = pw / 12; let win = [2, 4];
    const ticks = [0, 50e6, 100e6, 150e6, 200e6].filter(t => t <= mx * 1.05), y = x => pt + (Hh - pt - pb) * (1 - x / (Math.max(mx, ticks[ticks.length - 1]) * 1.04));
    const draw = () => {
      const sum = v.reduce((a, b) => a + b, 0), inw = v.slice(win[0], win[1] + 1).reduce((a, b) => a + b, 0);
      hs.innerHTML = '<svg viewBox="0 0 ' + Wd + ' ' + Hh + '" width="' + Wd + '" height="' + Hh + '" role="img" aria-label="Transfers per month, 2024">' + ticks.map(t => '<line x1="' + pl + '" x2="' + (Wd - 8) + '" y1="' + f1(y(t)) + '" y2="' + f1(y(t)) + '" class="' + (t ? 'c-grid' : 'c-axis') + '"/><text x="' + (pl - 8) + '" y="' + f1(y(t) + 3.5) + '" text-anchor="end" class="c-tick">' + (t ? (t / 1e6) + ' M' : '0') + '</text>').join('') +
        '<rect x="' + f1(pl + win[0] * bw) + '" y="' + (pt - 4) + '" width="' + f1((win[1] - win[0] + 1) * bw) + '" height="' + f1(y(0) - pt + 4) + '" class="c-win-fill"/>' +
        v.map((x, m) => '<rect x="' + f1(pl + m * bw + 2) + '" y="' + f1(y(x)) + '" width="' + f1(bw - 4) + '" height="' + f1(y(0) - y(x)) + '" class="c-bar' + (m >= win[0] && m <= win[1] ? ' in' : '') + '"><title>' + MONTHS[m] + ' 2024: ' + num(x) + ' transfers</title></rect><text x="' + f1(pl + m * bw + bw / 2) + '" y="' + (Hh - 7) + '" text-anchor="middle" class="c-tick' + (m >= win[0] && m <= win[1] ? ' in' : '') + '">' + MONTHS[m] + '</text>').join('') +
        '<rect x="' + f1(pl + win[0] * bw) + '" y="' + (pt - 4) + '" width="' + f1((win[1] - win[0] + 1) * bw) + '" height="' + f1(y(0) - pt + 4) + '" class="c-win" data-h="move"/>' + [['l', pl + win[0] * bw], ['r', pl + (win[1] + 1) * bw]].map(h => '<rect x="' + f1(h[1] - 4) + '" y="' + f1((pt + y(0)) / 2 - 14) + '" width="8" height="28" rx="2" class="c-handle" data-h="' + h[0] + '"/>').join('') + '</svg>';
      html('hist-read', '<b>' + MONTHS[win[0]] + (win[1] > win[0] ? ' – ' + MONTHS[win[1]] : '') + ' 2024</b> · <span class="num">' + num(inw) + '</span> transfers · <span class="num">' + (100 * inw / sum).toFixed(2) + '%</span> of the year');
    };
    draw(); let drag = null;
    hs.addEventListener('pointerdown', e => { const m = Math.max(0, Math.min(11, Math.floor((e.clientX - hs.getBoundingClientRect().left - pl) / bw))), k = e.target.dataset && e.target.dataset.h; drag = { k: k || 'new', m0: m, a: win[0], b: win[1] }; if (!k) { win = [m, m]; draw(); } hs.setPointerCapture(e.pointerId); e.preventDefault(); });
    hs.addEventListener('pointermove', e => { if (!drag) return; const m = Math.max(0, Math.min(11, Math.floor((e.clientX - hs.getBoundingClientRect().left - pl) / bw))); let a = drag.a, b = drag.b; if (drag.k === 'l') a = Math.min(m, b); else if (drag.k === 'r') b = Math.max(m, a); else if (drag.k === 'move') { const d = Math.max(-a, Math.min(11 - b, m - drag.m0)); a += d; b += d; } else { a = Math.min(drag.m0, m); b = Math.max(drag.m0, m); } if (a !== win[0] || b !== win[1]) { win = [a, b]; draw(); } });
    hs.addEventListener('pointerup', () => { drag = null; });
  }
  const rb = byId('ranked'); if (rb) { const L = linksOf(root.kids[1]).filter(l => !l.elsewhere).slice(0, 8), mx = L[0].w, tot = linksOf(root.kids[1]).reduce((s, l) => s + l.w, 0);
    rb.innerHTML = '<div class="rb-h"><span>Strongest connections of DeFi protocols</span><span class="muted">transfers, both directions</span></div>' + L.map((l, i) => '<div class="rb"><span class="num muted">' + (i + 1) + '</span><span class="rb-n">' + esc(shortTitle(l.t)) + '</span><span class="num rb-v">' + num(l.w) + '</span><span class="rb-b"><i style="width:' + (100 * l.w / mx).toFixed(1) + '%"></i></span></div>').join('') + '<div class="rb-f xs muted">Showing <span class="num">8</span> of <span class="num">' + linksOf(root.kids[1]).length + '</span> · <span class="num">' + (100 * L.reduce((s, l) => s + l.w, 0) / tot).toFixed(1) + '%</span> of the weight</div>'; }
  const sp = byId('sparks'); if (sp) { const spark = v => { const mx = Math.max(...v), mn = Math.min(...v), W2 = 96, H2 = 22, x = i => 2 + i * (W2 - 4) / 11, y = q => 3 + (H2 - 6) * (1 - (q - mn) / ((mx - mn) || 1)), pk = v.indexOf(mx); return '<svg viewBox="0 0 ' + W2 + ' ' + H2 + '" width="' + W2 + '" height="' + H2 + '" role="img" aria-label="Monthly transfers, peak in ' + MONTHS[pk] + '"><polyline points="' + v.map((q, i) => f1(x(i)) + ',' + f1(y(q))).join(' ') + '" class="c-line"/><circle cx="' + f1(x(pk)) + '" cy="' + f1(y(mx)) + '" r="2.2" class="c-dot"/></svg>'; };
    sp.innerHTML = '<table class="t"><thead><tr><th>Region</th><th class="num">Addresses</th><th class="num">Transfers inside</th><th>Per month, 2024</th><th>Peak</th></tr></thead><tbody>' + [0, 1, 4, 7, 10].map(i => { const n = root.kids[i], v = n.intv, mx = Math.max(...v); return '<tr><td>' + esc(n.name) + '</td><td class="num">' + num(n.n) + '</td><td class="num">' + num(v.reduce((a, b) => a + b, 0)) + '</td><td class="sp">' + spark(v) + '</td><td><span class="num">' + MONTHS[v.indexOf(mx)] + '</span></td></tr>'; }).join('') + '</tbody></table>'; }
  const dd = byId('degree'); if (dd) { const Wd = dd.clientWidth || 520, Hh = 260, pl = 58, pb = 40, pt = 12, pr = 14, r = R('degree'), pts = [];
    for (let d = 1; d <= 3e5; d = d < 100 ? d + 1 : Math.ceil(d * 1.18)) { const c = 9.2e7 * Math.pow(d, -2.15) * Math.exp((d < 100 ? .1 : .22) * gauss(r)); if (c >= 1) pts.push([d, Math.round(c)]); }
    const x = d => pl + (Wd - pl - pr) * Math.log10(d) / 6, y = c => pt + (Hh - pt - pb) * (1 - Math.log10(c) / 8), sup = n => String(n).split('').map(ch => '⁰¹²³⁴⁵⁶⁷⁸⁹'[+ch]).join('');
    let s = ''; for (let e = 0; e <= 8; e += 2) s += '<line x1="' + pl + '" x2="' + (Wd - pr) + '" y1="' + f1(y(Math.pow(10, e))) + '" y2="' + f1(y(Math.pow(10, e))) + '" class="' + (e ? 'c-grid' : 'c-axis') + '"/><text x="' + (pl - 8) + '" y="' + f1(y(Math.pow(10, e)) + 3.5) + '" text-anchor="end" class="c-tick">' + (e ? '10' + sup(e) : '1') + '</text>';
    for (let e = 0; e <= 6; e++) { s += '<line x1="' + f1(x(Math.pow(10, e))) + '" x2="' + f1(x(Math.pow(10, e))) + '" y1="' + f1(y(1)) + '" y2="' + f1(y(1) + 4) + '" class="c-axis"/><text x="' + f1(x(Math.pow(10, e))) + '" y="' + f1(y(1) + 16) + '" text-anchor="middle" class="c-tick">' + (e ? '10' + sup(e) : '1') + '</text>'; if (e < 6) for (let m = 2; m < 10; m++) s += '<line x1="' + f1(x(m * Math.pow(10, e))) + '" x2="' + f1(x(m * Math.pow(10, e))) + '" y1="' + f1(y(1)) + '" y2="' + f1(y(1) + 2.5) + '" class="c-axis"/>'; }
    s += '<line x1="' + pl + '" x2="' + pl + '" y1="' + pt + '" y2="' + f1(y(1)) + '" class="c-axis"/>' + pts.map(p => '<circle cx="' + f1(x(p[0])) + '" cy="' + f1(y(p[1])) + '" r="1.6" class="c-pt"><title>degree ' + num(p[0]) + ': ' + num(p[1]) + ' addresses</title></circle>').join('');
    s += '<text x="' + f1((pl + Wd - pr) / 2) + '" y="' + (Hh - 6) + '" text-anchor="middle" class="c-title">Degree (counterparts per address)</text><text transform="translate(13 ' + f1((pt + Hh - pb) / 2) + ') rotate(-90)" text-anchor="middle" class="c-title">Addresses</text>';
    dd.innerHTML = '<svg viewBox="0 0 ' + Wd + ' ' + Hh + '" width="' + Wd + '" height="' + Hh + '" role="img" aria-label="Degree distribution on logarithmic axes">' + s + '</svg>'; }
})();

/* =====================================================================
   9 · Tables
   ===================================================================== */
(function tables() {
  const el = byId('result-table'), el2 = byId('bar-table'); if (!el) return;
  const leafs = topMembers(root.kids[1], 5).concat(topMembers(root.kids[0], 5)).map(a => ({ a, w: cutOf(a), m: addrMeta(a), k: linksOf(a).length })).sort((x, y) => y.w - x.w), r = R('table'), tot = 4812;
  el.innerHTML = '<table class="t"><thead><tr><th>Address</th><th>Label</th><th>Community</th><th class="num">Counterparts</th><th class="num sorted" aria-sort="descending">Transfers <span aria-hidden="true">↓</span></th><th class="num">Value (ETH)</th><th class="num">Share of region (%)</th><th>Last seen</th></tr></thead><tbody>' +
    leafs.map((x, i) => '<tr' + (i === 2 ? ' class="is-selected"' : '') + '><td class="num">' + shortAddr(x.a) + '</td><td>' + (x.m.label ? esc(x.m.label) : '<span class="muted">none</span>') + '</td><td>' + esc(shortTitle(x.a.parent.parent.named ? x.a.parent.parent : x.a.top)) + '</td><td class="num">' + num(x.k * (40 + Math.floor(r() * 900))) + '</td><td class="num sorted">' + num(x.w) + '</td><td class="num">' + (x.w * (.4 + r()) / 30).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) + '</td><td class="num">' + (100 * x.w / leafs[0].w * .0412).toFixed(3) + '</td><td class="num">' + fmtDate(x.m.last) + '</td></tr>').join('') +
    '</tbody><tfoot><tr><td colspan="8"><span>Showing <b class="num">10</b> of <b class="num">' + num(tot) + '</b> rows · sorted by transfers, largest first · cut short at <span class="num">10</span></span><span><a href="#" onclick="return false">Show <span class="num">50</span> more</a> · <a href="#" onclick="return false">Export all <span class="num">' + num(tot) + '</span> as CSV</a></span></td></tr></tfoot></table>';
  if (el2) { const K = root.kids.slice().sort((a, b) => b.n - a.n).slice(0, 8), mx = K[0].n, tx = n => n.intv.reduce((a, b) => a + b, 0), mt = Math.max(...K.map(tx));
    el2.innerHTML = '<table class="t bars"><thead><tr><th>Region</th><th>Kind</th><th class="num sorted" aria-sort="descending">Addresses <span aria-hidden="true">↓</span></th><th class="num">Share (%)</th><th class="num">Transfers inside</th></tr></thead><tbody>' + K.map(n => '<tr><td>' + esc(n.name) + '</td><td>' + (n.kind === 'aggregate' ? '<span class="tag">aggregate</span>' : '<span class="muted">found community</span>') + '</td><td class="num sorted bc">' + num(n.n) + '<i style="width:' + (100 * n.n / mx).toFixed(1) + '%"></i></td><td class="num">' + (100 * n.n / GRAPH.n).toFixed(2) + '</td><td class="num bc">' + num(tx(n)) + '<i style="width:' + (100 * tx(n) / mt).toFixed(1) + '%"></i></td></tr>').join('') + '</tbody><tfoot><tr><td colspan="5"><span>Showing <b class="num">8</b> of <b class="num">12</b> regions · sorted by addresses, largest first</span><span><a href="#" onclick="return false">Show all <span class="num">12</span></a></span></td></tr></tfoot></table>'; }
})();

/* "other" rule: a legend for more than ten categories */
(function otherRule() {
  const el = byId('other-legend'); if (!el) return; const r = R('other'), cats = ['USDT', 'USDC', 'WETH', 'DAI', 'WBTC', 'stETH', 'LINK', 'UNI', 'PEPE']; let rest = 1e8, h = '';
  const vals = cats.map((c, i) => { const v = Math.round(rest * (.26 + .1 * r())); rest -= v; return v; });
  cats.forEach((c, i) => { h += '<div class="lg-row"><i style="background:var(--d' + (i + 1) + ')"></i><span>' + c + '</span><span class="num">' + num(vals[i]) + '</span></div>'; });
  el.innerHTML = '<div class="lg-t">Token of the transfer</div>' + h + '<div class="lg-row other"><i style="background:var(--d10)"></i><span>Other <span class="muted">· <span class="num">1,195</span> tokens</span></span><span class="num">' + num(rest) + '</span></div><div class="lg-n">Nine hues by rank, fixed for the session. Everything else is “other”, with its count and how many it folds.</div>';
})();
/* section links scroll without touching the hash (the hash carries the theme); #go=s4 jumps on load, for screenshots */
$$('.toc a').forEach(a => a.addEventListener('click', e => { e.preventDefault(); byId(a.getAttribute('href').slice(1)).scrollIntoView(); }));
if (window.EGStyle && EGStyle.opts.go && byId(EGStyle.opts.go)) byId(EGStyle.opts.go).scrollIntoView();
window.EGData = { drawMap, frame, PAL, REUSE, HUE };   // debugging handle only
document.documentElement.dataset.ready = '1';
})();
