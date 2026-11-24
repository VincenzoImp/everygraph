// glab graph map viewer: plain WebGL2, no build step. Reads a PMTiles v3 container (custom
// "glab-map-tile-v0" payload) with HTTP range requests.
'use strict';
const P = new URLSearchParams(location.search);
const URL_ = P.get('file') || '../out/lj.pmtiles';
const stats = { bytes: 0, requests: 0, tiles: 0, frames: [] , recording: false};
async function range(off, len) {
  const r = await fetch(URL_, { headers: { Range: `bytes=${off}-${off + len - 1}` } });
  const b = await r.arrayBuffer(); stats.bytes += b.byteLength; stats.requests++; return b;
}
async function gunzip(buf) {
  return await new Response(new Blob([buf]).stream().pipeThrough(new DecompressionStream('gzip'))).arrayBuffer();
}
// ---------- PMTiles v3 reader ----------
function zxyToId(z, x, y) {
  let acc = (4 ** z - 1) / 3, n = 2 ** z, d = 0;
  for (let s = n / 2; s >= 1; s /= 2) {
    const rx = (x & s) > 0 ? 1 : 0, ry = (y & s) > 0 ? 1 : 0;
    d += s * s * ((3 * rx) ^ ry);
    if (ry === 0) { if (rx === 1) { x = n - 1 - x; y = n - 1 - y; } const t = x; x = y; y = t; }
  }
  return acc + d;
}
function parseDir(buf) {
  const u = new Uint8Array(buf); let p = 0;
  const vi = () => { let r = 0, m = 1, b; do { b = u[p++]; r += (b & 0x7f) * m; m *= 128; } while (b & 0x80); return r; };
  const n = vi(); const id = new Float64Array(n), run = new Uint32Array(n), len = new Uint32Array(n), off = new Float64Array(n);
  let last = 0; for (let i = 0; i < n; i++) { last += vi(); id[i] = last; }
  for (let i = 0; i < n; i++) run[i] = vi();
  for (let i = 0; i < n; i++) len[i] = vi();
  for (let i = 0; i < n; i++) { const v = vi(); off[i] = (v === 0 && i > 0) ? off[i - 1] + len[i - 1] : v - 1; }
  return { n, id, run, len, off };
}
const pm = { leafCache: new Map() };
async function pmOpen() {
  const b = await range(0, 16384); const dv = new DataView(b);
  const u64 = o => Number(dv.getBigUint64(o, true));
  if (new TextDecoder().decode(b.slice(0, 7)) !== 'PMTiles' || dv.getUint8(7) !== 3) throw new Error('not PMTiles v3');
  Object.assign(pm, { rootOff: u64(8), rootLen: u64(16), metaOff: u64(24), metaLen: u64(32), leafOff: u64(40), leafLen: u64(48), dataOff: u64(56), dataLen: u64(64), nTiles: u64(72), tileType: dv.getUint8(99), maxZoom: dv.getUint8(101) });
  pm.root = parseDir(await gunzip(b.slice(pm.rootOff, pm.rootOff + pm.rootLen)));
  const mb = (pm.metaOff + pm.metaLen <= 16384) ? b.slice(pm.metaOff, pm.metaOff + pm.metaLen) : await range(pm.metaOff, pm.metaLen);
  pm.meta = JSON.parse(new TextDecoder().decode(await gunzip(mb)));
}
function dirFind(d, id) {
  let lo = 0, hi = d.n - 1, r = -1;
  while (lo <= hi) { const m = (lo + hi) >> 1; if (d.id[m] <= id) { r = m; lo = m + 1; } else hi = m - 1; }
  return r;
}
async function pmGet(z, x, y) {
  const id = zxyToId(z, x, y); let d = pm.root;
  for (let depth = 0; depth < 4; depth++) {
    const i = dirFind(d, id); if (i < 0) return null;
    if (d.run[i] > 0) { if (id >= d.id[i] + d.run[i]) return null; return await gunzip(await range(pm.dataOff + d.off[i], d.len[i])); }
    const key = d.off[i];
    if (!pm.leafCache.has(key)) pm.leafCache.set(key, range(pm.leafOff + d.off[i], d.len[i]).then(gunzip).then(parseDir));
    d = await pm.leafCache.get(key);
  }
  return null;
}
// ---------- tile payload ----------
function parseTile(buf) {
  const dv = new DataView(buf); const u32 = o => dv.getUint32(o, true), f32 = o => dv.getFloat32(o, true), u64 = o => Number(dv.getBigUint64(o, true));
  const t = { z: dv.getUint8(4), x: u32(8), y: u32(12), nctx: u32(16), nbub: u32(20), nnode: u32(24), nedge: u32(28), omB: u32(32), omN: u32(36), omBN: u64(40), wTotal: u64(48), wShown: u64(56), wCtx: u64(64), bytes: buf.byteLength };
  let o = 72; t.bub = []; t.nodes = []; t.edges = [];
  for (let i = 0; i < t.nctx + t.nbub; i++, o += 44)
    t.bub.push({ id: u32(o), parent: u32(o + 4), x: f32(o + 8), y: f32(o + 12), r: f32(o + 16), nodes: u32(o + 20), internal: u64(o + 24), volume: u64(o + 32), level: dv.getInt8(o + 40), flags: dv.getUint8(o + 41), top: dv.getUint16(o + 42, true), ctx: i < t.nctx });
  for (let i = 0; i < t.nnode; i++, o += 36)
    t.nodes.push({ id: u32(o), x: f32(o + 4), y: f32(o + 8), r: f32(o + 12), deg: u32(o + 16), parent: u32(o + 20), top: dv.getUint16(o + 24, true), orig: u64(o + 28) });
  for (let i = 0; i < t.nedge; i++, o += 20) t.edges.push({ src: u32(o), dst: u32(o + 4), x: f32(o + 8), y: f32(o + 12), w: u32(o + 16) });
  return t;
}
function color(rank) {
  const h = ((rank * 0.618034) % 1) * 6, s = 0.55 + 0.15 * (rank % 3), c = s, x = c * (1 - Math.abs((h % 2) - 1)), m = 1 - c;
  const [r, g, b] = [[c, x, 0], [x, c, 0], [0, c, x], [0, x, c], [x, 0, c], [c, 0, x]][Math.floor(h) % 6];
  return [r + m, g + m, b + m];
}
// ---------- GL ----------
const cv = document.getElementById('gl'), ov = document.getElementById('ov'), ctx2 = ov.getContext('2d');
const gl = cv.getContext('webgl2', { antialias: true, alpha: false, powerPreference: 'high-performance' });
const dbg = gl.getExtension('WEBGL_debug_renderer_info');
const RENDERER = dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
function prog(vs, fs) {
  const p = gl.createProgram();
  for (const [t, s] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) { const sh = gl.createShader(t); gl.shaderSource(sh, s); gl.compileShader(sh); if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(sh)); gl.attachShader(p, sh); }
  gl.linkProgram(p); if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p)); return p;
}
const circP = prog(`#version 300 es
layout(location=0) in vec2 corner; layout(location=1) in vec3 inst; layout(location=2) in vec4 col;
uniform vec2 uCenter; uniform float uScale; uniform vec2 uView; out vec2 vUv; out vec4 vCol; out float vR;
void main(){ float rpx = inst.z*uScale; if (col.a > 0.9) rpx = max(rpx*0.55, 1.6); float q = rpx + 1.5;
 vec2 p = (inst.xy-uCenter)*uScale + corner*q; gl_Position = vec4(p/(uView*0.5)*vec2(1.,-1.),0.,1.); vUv = corner*q; vR = rpx; vCol = col; }`,
`#version 300 es
precision highp float; in vec2 vUv; in vec4 vCol; in float vR; out vec4 o;
void main(){ float d = length(vUv); float aa = clamp(vR-d+0.5,0.,1.); 
 if (vCol.a > 1.5) { float ring = clamp(1.8-abs(vR-d),0.,1.); if (ring<=0.) discard; o = vec4(1.,1.,1.,ring); return; }
 if (aa<=0.) discard;
 if (vCol.a > 0.9) { o = vec4(vCol.rgb*(0.75+0.25*clamp(1.-d/vR,0.,1.)), aa); }
 else { float ring = clamp(1.3-abs(vR-d-0.8),0.,1.); o = vec4(mix(vCol.rgb, vec3(1.), ring*0.35), max(ring*vCol.a*1.6, 0.03)*aa); } }`);
const edgeP = prog(`#version 300 es
layout(location=0) in vec2 corner; layout(location=1) in vec4 ab; layout(location=2) in vec2 wa;
uniform vec2 uCenter; uniform float uScale; uniform vec2 uView; out float vA; out float vS;
void main(){ vec2 pa=(ab.xy-uCenter)*uScale, pb=(ab.zw-uCenter)*uScale; vec2 dir=normalize(pb-pa+vec2(1e-6)); vec2 nrm=vec2(-dir.y,dir.x);
 vec2 p = mix(pa,pb,corner.x) + nrm*corner.y*(wa.x*0.5+0.75); gl_Position = vec4(p/(uView*0.5)*vec2(1.,-1.),0.,1.); vA = wa.y; vS = corner.y*(wa.x*0.5+0.75); }`,
`#version 300 es
precision highp float; in float vA; in float vS; out vec4 o; uniform float uHalf;
void main(){ o = vec4(0.72,0.80,1.0, vA); }`);
const quad = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, quad); gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
const equad = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, equad); gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, -1, 1, -1, 0, 1, 1, 1]), gl.STATIC_DRAW);
function mkVao(q, stride, attrs) {
  const vao = gl.createVertexArray(), buf = gl.createBuffer(); gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, q); gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  for (const [loc, size, off] of attrs) { gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, size, gl.FLOAT, false, stride * 4, off * 4); gl.vertexAttribDivisor(loc, 1); }
  gl.bindVertexArray(null); return { vao, buf, n: 0 };
}
const bubV = mkVao(quad, 7, [[1, 3, 0], [2, 4, 3]]), nodeV = mkVao(quad, 7, [[1, 3, 0], [2, 4, 3]]), edgeV = mkVao(equad, 6, [[1, 4, 0], [2, 2, 4]]), hiV = mkVao(quad, 7, [[1, 3, 0], [2, 4, 3]]);
// ---------- view state ----------
const view = { cx: +(P.get('x') || 0.5), cy: +(P.get('y') || 0.5), zf: +(P.get('z') || 1.6), tz: null, anim: null, mark: null };
view.tz = view.zf;
const cache = new Map(); let drawn = [], dirty = true, pending = 0, hover = null, lastKey = '';
const scale = () => 256 * 2 ** view.zf;
function tileZoom() { return Math.max(0, Math.min(pm.maxZoom, Math.floor(view.zf + 0.3))); }
function wanted() {
  const z = tileZoom(), n = 2 ** z, s = scale(), hw = cv.clientWidth / 2 / s, hh = cv.clientHeight / 2 / s, m = 1.0 / n * 0.75;
  const x0 = Math.max(0, Math.floor((view.cx - hw - m) * n)), x1 = Math.min(n - 1, Math.floor((view.cx + hw + m) * n));
  const y0 = Math.max(0, Math.floor((view.cy - hh - m) * n)), y1 = Math.min(n - 1, Math.floor((view.cy + hh + m) * n));
  const out = []; for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) out.push([z, x, y]);
  return out;
}
function ensure(z, x, y) {
  const k = `${z}/${x}/${y}`; if (cache.has(k)) return cache.get(k);
  const e = { state: 0, tile: null }; cache.set(k, e); pending++;
  pmGet(z, x, y).then(b => { e.state = 1; if (b) { e.tile = parseTile(b); stats.tiles++; } pending--; dirty = true; }).catch(err => { console.error(err); e.state = 1; pending--; });
  return e;
}
function rebuild() {
  const w = wanted(); const seen = new Set(), tiles = [];
  for (const [z, x, y] of w) {
    let e = ensure(z, x, y), zz = z, xx = x, yy = y;
    while (e.state === 0 && zz > 0) { zz--; xx >>= 1; yy >>= 1; const k = `${zz}/${xx}/${yy}`; if (!cache.has(k)) { e = null; break; } e = cache.get(k); }
    if (e && e.tile) { const k = `${zz}/${xx}/${yy}`; if (!seen.has(k)) { seen.add(k); tiles.push(e.tile); } }
  }
  drawn = tiles;
  const bset = new Map(); const nb = [], nn = [], ne = [];
  for (const t of tiles) for (const b of t.bub) if (!bset.has(b.id)) bset.set(b.id, b);
  const bl = [...bset.values()].sort((a, b) => b.r - a.r);
  for (const b of bl) { const c = color(b.top); nb.push(b.x, b.y, b.r, c[0], c[1], c[2], (b.flags & 1) ? 0.22 : 0.5); }
  const agg = [];
  const tw = 1 / 2 ** tileZoom();
  for (const t of tiles) {
    for (const n of t.nodes) { const c = color(n.top); nn.push(n.x, n.y, n.r, c[0], c[1], c[2], 1.0); }
    for (const e of t.edges) {
      const l = e.src, isNode = l >= t.bub.length, s = isNode ? t.nodes[l - t.bub.length] : t.bub[l];
      if (isNode && e.w === 1) { const len = Math.hypot(s.x - e.x, s.y - e.y); ne.push(s.x, s.y, e.x, e.y, 0.3, len < tw * 0.6 ? 0.42 : 0.07); }
      else agg.push([e.w, s.x, s.y, e.x, e.y]);
    }
  }
  // aggregated edges: only the strongest ones in view are drawn (the rest stay available in the tile data / tooltips)
  agg.sort((a, b) => b[0] - a[0]); const keep = agg.slice(0, 220), wmax = keep.length ? keep[0][0] : 1, wmin = keep.length ? keep[keep.length - 1][0] : 1;
  for (const e of keep) { const rel = wmax > wmin ? Math.log(e[0] / wmin) / Math.log(wmax / wmin) : 1; ne.push(e[1], e[2], e[3], e[4], 0.2 + 3.2 * rel, 0.10 + 0.45 * rel); }
  view.aggShown = keep.length; view.aggTotal = agg.length;
  for (const [v, arr, per] of [[bubV, nb, 7], [nodeV, nn, 7], [edgeV, ne, 6]]) { gl.bindBuffer(gl.ARRAY_BUFFER, v.buf); gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(arr), gl.DYNAMIC_DRAW); v.n = arr.length / per; }
  view.bl = bl; dirty = false;
}
function fmt(n) { return n.toLocaleString('en-US'); }
let lastT = 0;
function frame(t) {
  if (stats.recording && lastT) stats.frames.push(t - lastT); lastT = t;
  const dpr = window.devicePixelRatio || 1, W = cv.clientWidth, H = cv.clientHeight;
  if (cv.width !== W * dpr || cv.height !== H * dpr) { cv.width = W * dpr; cv.height = H * dpr; ov.width = W * dpr; ov.height = H * dpr; }
  if (view.anim) {
    const a = view.anim, u = Math.min(1, (t - a.t0) / a.dur), e = u * u * (3 - 2 * u);
    view.cx = a.x0 + (a.x1 - a.x0) * e; view.cy = a.y0 + (a.y1 - a.y0) * e; view.zf = a.z0 + (a.z1 - a.z0) * e - a.dip * 4 * u * (1 - u); view.tz = view.zf;
    if (u >= 1) view.anim = null;
  } else if (Math.abs(view.tz - view.zf) > 1e-3) {
    // smooth zoom about the anchor point
    const s0 = scale(); const dz = (view.tz - view.zf) * 0.25; view.zf += dz; const s1 = scale();
    if (view.anchor) { const [ax, ay] = view.anchor; view.cx += ax * (1 / s0 - 1 / s1); view.cy += ay * (1 / s0 - 1 / s1); }
  }
  const key = wanted().map(a => a.join('/')).join(' ');
  if (key !== lastKey) { lastKey = key; dirty = true; }
  if (dirty) rebuild();
  gl.viewport(0, 0, cv.width, cv.height); gl.clearColor(0.035, 0.04, 0.063, 1); gl.clear(gl.COLOR_BUFFER_BIT);
  gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  const s = scale();
  for (const [p, v] of [[circP, bubV], [edgeP, edgeV], [circP, nodeV], [circP, hiV]]) {
    if (!v.n) continue; gl.useProgram(p);
    gl.uniform2f(gl.getUniformLocation(p, 'uCenter'), view.cx, view.cy); gl.uniform1f(gl.getUniformLocation(p, 'uScale'), s); gl.uniform2f(gl.getUniformLocation(p, 'uView'), W, H);
    gl.bindVertexArray(v.vao); gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, v.n);
  }
  // labels
  ctx2.setTransform(dpr, 0, 0, dpr, 0, 0); ctx2.clearRect(0, 0, W, H); ctx2.textAlign = 'center'; ctx2.font = '11px system-ui'; let nl = 0;
  const boxes = [];
  for (const b of (view.bl || [])) {
    const rp = b.r * s; if (rp < 34 || rp > 900 || nl > 60) continue;
    const x = (b.x - view.cx) * s + W / 2, y = (b.y - view.cy) * s + H / 2 - rp + 13; if (x < 0 || x > W || y < 0 || y > H) continue;
    if (boxes.some(q => Math.abs(q[0] - x) < 86 && Math.abs(q[1] - y) < 14)) continue; boxes.push([x, y]);
    const txt = fmt(b.nodes) + ' nodes'; ctx2.fillStyle = '#090a10b0'; ctx2.fillRect(x - txt.length * 3.1 - 3, y - 10, txt.length * 6.2 + 6, 13);
    ctx2.fillStyle = (b.flags & 1) ? '#9aa3b5' : '#ffffff'; ctx2.fillText(txt, x, y); nl++;
  }
  document.getElementById('stat').textContent = `zoom ${view.zf.toFixed(2)} (tile z${tileZoom()}/${pm.maxZoom}) · ${drawn.length} tiles drawn · ${bubV.n} bubbles ${nodeV.n} nodes ${edgeV.n} edges\n${stats.requests} requests · ${(stats.bytes / 1024).toFixed(0)} KiB transferred`;
  requestAnimationFrame(frame);
}
// ---------- interaction ----------
function setHi(ent) { const a = ent ? [ent.x, ent.y, ent.deg !== undefined ? Math.max(ent.r * 0.55, 3 / scale()) : ent.r, 1, 1, 1, 2.0] : []; gl.bindBuffer(gl.ARRAY_BUFFER, hiV.buf); gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(a), gl.DYNAMIC_DRAW); hiV.n = a.length / 7; }
function pick(mx, my) {
  const s = scale(), wx = view.cx + (mx - cv.clientWidth / 2) / s, wy = view.cy + (my - cv.clientHeight / 2) / s; let best = null, bt = null;
  for (const t of drawn) {
    for (const n of t.nodes) { const rr = Math.max(n.r * 0.55, 4 / s); if ((n.x - wx) ** 2 + (n.y - wy) ** 2 <= rr * rr) return [n, t]; }
    for (const b of t.bub) if ((b.x - wx) ** 2 + (b.y - wy) ** 2 <= b.r * b.r && (!best || b.r < best.r)) { best = b; bt = t; }
  }
  return best ? [best, bt] : null;
}
const tip = document.getElementById('tip');
cv.addEventListener('mousemove', ev => {
  if (drag) { const s = scale(); view.cx -= (ev.clientX - drag[0]) / s; view.cy -= (ev.clientY - drag[1]) / s; drag = [ev.clientX, ev.clientY]; tip.style.display = 'none'; return; }
  const h = pick(ev.clientX, ev.clientY); if (!h) { tip.style.display = 'none'; setHi(null); hover = null; return; }
  const [e, t] = h; hover = e; setHi(e); let s;
  if (e.deg !== undefined) s = `node ${e.orig}  (dense #${e.id})\ndegree          ${fmt(e.deg)}\ncommunity       #${e.parent}\ncontinent       #${e.top}`;
  else s = `${(e.flags & 1) ? 'derived region' : 'community'} #${e.id}${e.level >= 0 ? '  (hierarchy level ' + e.level + ')' : ''}\nnodes           ${fmt(e.nodes)}\ninternal edges  ${fmt(e.internal)}\nvolume (Σdeg)   ${fmt(e.volume)}\ncut edges       ${fmt(e.volume - 2 * e.internal)}\ncontinent       #${e.top}`;
  s += `\n── tile ${t.z}/${t.x}/${t.y} (${fmt(t.bytes)} B) ──\nedge weight shown ${fmt(t.wShown)} of ${fmt(t.wTotal)}\nomitted: ${fmt(t.omB)} regions (${fmt(t.omBN)} nodes), ${fmt(t.omN)} nodes`;
  tip.textContent = s; tip.style.display = 'block'; tip.style.left = Math.min(ev.clientX + 14, cv.clientWidth - 330) + 'px'; tip.style.top = (ev.clientY + 14) + 'px';
});
let drag = null;
cv.addEventListener('mousedown', ev => { drag = [ev.clientX, ev.clientY]; });
window.addEventListener('mouseup', () => { drag = null; });
cv.addEventListener('wheel', ev => { ev.preventDefault(); view.anim = null; view.tz = Math.max(0, Math.min(pm.maxZoom + 2.5, view.tz - ev.deltaY * 0.0025)); view.anchor = [ev.clientX - cv.clientWidth / 2, ev.clientY - cv.clientHeight / 2]; }, { passive: false });
function flyTo(x, y, z, dur = 2500) { const dist = Math.hypot(x - view.cx, y - view.cy); const dip = Math.max(0, Math.min(view.zf, z) - Math.max(0.5, -Math.log2(dist * 2 + 1e-9))) * 0.5; view.anim = { t0: performance.now(), dur, x0: view.cx, y0: view.cy, z0: view.zf, x1: x, y1: y, z1: z, dip }; }
// ---------- search (2 range requests cold, 1 warm) ----------
let stopIdx = null;
async function search(idStr) {
  const S = pm.meta.search, id = BigInt(idStr);
  const base = pm.dataOff + pm.dataLen;
  if (!stopIdx) stopIdx = new BigUint64Array(await range(base + S.top_rel, S.top_length));
  let lo = 0, hi = stopIdx.length - 1, blk = 0; while (lo <= hi) { const m = (lo + hi) >> 1; if (stopIdx[m] <= id) { blk = m; lo = m + 1; } else hi = m - 1; }
  const first = blk * S.block, cnt = Math.min(S.block, S.n - first), b = await range(base + S.rec_rel + first * 16, cnt * 16), dv = new DataView(b);
  lo = 0; hi = cnt - 1;
  while (lo <= hi) { const m = (lo + hi) >> 1, v = dv.getBigUint64(m * 16, true); if (v === id) return { x: dv.getFloat32(m * 16 + 8, true), y: dv.getFloat32(m * 16 + 12, true) }; if (v < id) lo = m + 1; else hi = m - 1; }
  return null;
}
async function go() {
  const q = document.getElementById('q').value.trim(); if (!q) return; const b0 = stats.bytes, r0 = stats.requests;
  const r = await search(q); const el = document.getElementById('sres');
  if (!r) { el.textContent = 'not found'; return null; }
  el.textContent = `found · ${stats.requests - r0} req, ${((stats.bytes - b0) / 1024).toFixed(1)} KiB`;
  setHi({ x: r.x, y: r.y, r: 0, deg: 0 }); flyTo(r.x, r.y, pm.maxZoom + 1.2); return r;
}
document.getElementById('go').onclick = go; document.getElementById('q').addEventListener('keydown', e => { if (e.key === 'Enter') go(); });
window.__gm = { stats, view, flyTo, go, search, renderer: RENDERER, idle: () => pending === 0 && !view.anim && Math.abs(view.tz - view.zf) < 2e-3 && !dirty, pm };
pmOpen().then(() => { document.getElementById('title').textContent = `${pm.meta.name.split('/').pop()} — ${fmt(pm.meta.n)} nodes, ${fmt(pm.meta.m)} edges`; document.getElementById('tile').textContent = `${fmt(pm.nTiles)} tiles · ${RENDERER.slice(0, 60)}`; requestAnimationFrame(frame); });
