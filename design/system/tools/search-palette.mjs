// SPDX-License-Identifier: Apache-2.0 OR MIT
// Search for a categorical palette: one hue per slot, its own lightness and chroma per theme,
// the last slot a neutral ("other"). Simulated annealing on the worst-pair ΔE, normal and
// colour-blind (color.mjs), against the surfaces and accent of tokens.json.
//   node search-palette.mjs [seed] [iterations] [colours] [light L max] [dark C max]
// The current d1–d10 is the output of seed 12 with the defaults, reordered farthest-first so
// that d1–d5, the region set, is the best-separated five. Re-measure with measure.mjs.
import fs from 'node:fs';
import { lch2hex, lin, simLin, oklabFromLin, contrast, oklch, deltaE, separation } from './color.mjs';
const J = JSON.parse(fs.readFileSync(new URL('../tokens.json', import.meta.url), 'utf8'));
const TOK = Object.fromEntries(['light', 'dark'].map(t => [t, { map: J[t]['map-bg'], bg: J[t].bg, surface: J[t].surface, accent: J[t].accent }]));
function rng(seed) { let s = seed >>> 0; return () => { s |= 0; s = s + 0x6D2B79F5 | 0; let t = Math.imul(s ^ s >>> 15, 1 | s); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; }; }
const hd = (a, b) => Math.min(Math.abs(a - b), 360 - Math.abs(a - b));
const [seedS = '1', itS = '80000', nS = '10', LlMax = '.70', CdMax = '.13'] = process.argv.slice(2); const R = rng(+seedS), iters = +itS, n = +nS;
const TH = [{ t: TOK.light, L: [.40, +LlMax], C: [.07, .16], nL: [.48, .58], surf: [TOK.light.map, TOK.light.bg], cmin: 3, av: 10 }, { t: TOK.dark, L: [.55, .88], C: [.06, +CdMax], nL: [.66, .76], surf: [TOK.dark.surface], cmin: 4, av: 8 }];
const labs = hex => { const l = lin(hex); return [oklabFromLin(simLin(l, 'protan')), oklabFromLin(simLin(l, 'deutan')), oklabFromLin(l), oklabFromLin(simLin(l, 'tritan'))]; };
const d = (a, b) => 100 * Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
function mkc(k, th, L, C, H) { const T = TH[th], neu = k === n - 1;
  if (neu) { if (L < T.nL[0] || L > T.nL[1] || C < .012 || C > .028 || hd(H, 75) > 20) return null; } else if (L < T.L[0] || L > T.L[1] || C < T.C[0] || C > T.C[1]) return null;
  const hex = lch2hex(L, C, H), o = oklch(hex); if (o[1] < (neu ? .008 : T.C[0] - .004) || Math.abs(o[0] - L) > .02) return null;
  for (const s of T.surf) if (contrast(hex, s) < T.cmin) return null;
  if (!neu && deltaE(hex, T.t.accent) < T.av) return null;
  return { hex, lab: labs(hex) }; }
function sc(P, th) { let m = 1e9, pen = 0; const ds = [];
  for (let i = 0; i < n; i++) for (let j = i + 1; j < n; j++) { const a = P[i].c[th].lab, b = P[j].c[th].lab, nv = d(a[2], b[2]), tv = d(a[3], b[3]); if (nv < 15) pen += 15 - nv; if (tv < 6) pen += .5 * (6 - tv);
    const v = Math.min(d(a[0], b[0]), d(a[1], b[1]), .5 * nv, .9 * tv + 2); ds.push(v); if (v < m) m = v; }
  ds.sort((x, y) => x - y); return m + .03 * (ds[1] + ds[2] + ds[3]) / 3 - pen; }
const score = P => { const a = sc(P, 0), b = sc(P, 1); return Math.min(a, b) + .1 * (a + b) / 2; };
function make(k, v) { const c0 = mkc(k, 0, v[0], v[1], v[4]), c1 = c0 && mkc(k, 1, v[2], v[3], v[4]); return c1 ? { v, c: [c0, c1] } : null; }
let best = null;
for (let r = 0; r < 4; r++) {
  const P = []; for (let k = 0; k < n; k++) for (;;) { const neu = k === n - 1, v = neu ? [.48 + R() * .1, .012 + R() * .016, .66 + R() * .1, .012 + R() * .016, 60 + R() * 30] : [.40 + R() * .3, .07 + R() * .09, .55 + R() * .33, .06 + R() * .07, R() * 360]; const p = make(k, v); if (p) { P.push(p); break; } }
  let cur = score(P);
  for (let it = 0; it < iters; it++) { const T = 1.2 * (1 - it / iters) ** 2, k = Math.floor(R() * n), o = P[k], s = .02 + .25 * (1 - it / iters), v = o.v.slice(), w = Math.floor(R() * 3);
    if (w === 0) { v[0] += (R() - .5) * s * .6; v[1] += (R() - .5) * s * .3; } else if (w === 1) { v[2] += (R() - .5) * s * .6; v[3] += (R() - .5) * s * .3; } else v[4] = (v[4] + (R() - .5) * s * 360 + 360) % 360;
    const p = make(k, v); if (!p) continue; P[k] = p; const x = score(P); if (x >= cur || R() < Math.exp((x - cur) / Math.max(T, 1e-3))) cur = x; else P[k] = o; }
  if (!best || cur > best.cur) best = { cur, P: P.slice() };
}
const out = { seed: +seedS, score: +best.cur.toFixed(2), light: best.P.map(p => p.c[0].hex), dark: best.P.map(p => p.c[1].hex) };
for (const th of ['light', 'dark']) { const s = separation(out[th]); out[th + 'Sep'] = `normal ${s.normal[0].toFixed(1)} protan ${s.protan[0].toFixed(1)} deutan ${s.deutan[0].toFixed(1)} tritan ${s.tritan[0].toFixed(1)}`; }
console.log(JSON.stringify(out));
