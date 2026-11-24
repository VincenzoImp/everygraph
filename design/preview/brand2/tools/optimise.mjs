// Search for a 10-colour categorical palette that maximises the worst pairwise
// Delta E under protanopia and deuteranopia, inside a system's constraints.
import { lch2hex, lin, simLin, oklabFromLin, contrast, oklch, deltaE, separation } from './color.mjs';
function rng(seed) { let s = seed >>> 0; return () => { s |= 0; s = s + 0x6D2B79F5 | 0; let t = Math.imul(s ^ s >>> 15, 1 | s); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; }; }
export function optimise({ n = 10, surface, surface2, Lmin, Lmax, Cmin, Cmax, cmin, cmin2 = 0, normalFloor = 15, seed = 1, iters = 60000, restarts = 6, minDL = 0, wNormal = 0.5, tritanFloor = 6, avoid = [] }) {
  const R = rng(seed);
  const feas = (L, C, H) => {
    if (L < Lmin || L > Lmax || C < Cmin || C > Cmax) return null;
    const hex = lch2hex(L, C, H); const [l2, c2] = oklch(hex);
    if (c2 < Cmin - 0.004 || Math.abs(l2 - L) > 0.02) return null;
    if (contrast(hex, surface) < cmin) return null;
    if (surface2 && contrast(hex, surface2) < cmin2) return null;
    for (const [a, dmin] of avoid) if (deltaE(hex, a) < dmin) return null;
    return hex;
  };
  const labs = hex => { const l = lin(hex); return [oklabFromLin(simLin(l, 'protan')), oklabFromLin(simLin(l, 'deutan')), oklabFromLin(l), oklabFromLin(simLin(l, 'tritan'))]; };
  const d = (a, b) => 100 * Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
  const score = (P) => { // P: array of {lab:[p,d,n]}
    let m = 1e9, pen = 0; const ds = [];
    for (let i = 0; i < n; i++) for (let j = i + 1; j < n; j++) {
      const nv = d(P[i].lab[2], P[j].lab[2]); if (nv < normalFloor) pen += (normalFloor - nv);
      const v = Math.min(d(P[i].lab[0], P[j].lab[0]), d(P[i].lab[1], P[j].lab[1]), wNormal * nv); ds.push(v); if (v < m) m = v;
      const tv = d(P[i].lab[3], P[j].lab[3]); if (tv < tritanFloor) pen += 0.5 * (tritanFloor - tv);
      if (minDL) { const dl = Math.abs(P[i].lab[2][0] - P[j].lab[2][0]) * 100; if (dl < minDL) pen += (minDL - dl); }
    }
    ds.sort((a, b) => a - b);
    return m + 0.02 * (ds[1] + ds[2] + ds[3]) / 3 - pen;
  };
  let best = null;
  for (let r = 0; r < restarts; r++) {
    const P = [];
    while (P.length < n) { const L = Lmin + R() * (Lmax - Lmin), C = Cmin + R() * (Cmax - Cmin), H = R() * 360; const hex = feas(L, C, H); if (hex) P.push({ L, C, H, hex, lab: labs(hex) }); }
    let cur = score(P);
    for (let it = 0; it < iters; it++) {
      const T = 1.5 * (1 - it / iters) ** 2, k = Math.floor(R() * n), o = P[k], s = 0.02 + 0.25 * (1 - it / iters);
      const L = o.L + (R() - 0.5) * s * 0.6, C = o.C + (R() - 0.5) * s * 0.3, H = (o.H + (R() - 0.5) * s * 360 + 360) % 360;
      const hex = feas(L, C, H); if (!hex) continue;
      P[k] = { L, C, H, hex, lab: labs(hex) };
      const sc = score(P);
      if (sc >= cur || R() < Math.exp((sc - cur) / Math.max(T, 1e-3))) cur = sc; else P[k] = o;
    }
    if (!best || cur > best.score) best = { score: cur, pal: P.map(p => p.hex) };
  }
  return best.pal;
}
// Fixed order: start from the colour nearest a preferred hue, then always take the
// colour farthest (in the worst of protan/deutan) from those already chosen.
export function order(pal, firstHue) {
  const hd = (a, b) => Math.min(Math.abs(a - b), 360 - Math.abs(a - b));
  const rest = [...pal].sort((a, b) => hd(oklch(a)[2], firstHue) - hd(oklch(b)[2], firstHue));
  const out = [rest.shift()];
  while (rest.length) {
    let bi = 0, bv = -1;
    rest.forEach((c, i) => { const v = Math.min(...out.map(o => Math.min(deltaE(c, o, 'protan'), deltaE(c, o, 'deutan'), deltaE(c, o) * 0.6))); if (v > bv) { bv = v; bi = i; } });
    out.push(rest.splice(bi, 1)[0]);
  }
  return out;
}
