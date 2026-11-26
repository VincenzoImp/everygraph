// Colour maths for the round-2 identity mockups. No dependencies.
// CVD model: Machado, Oliveira & Fernandes (2009), severity 1.0, applied in linear RGB.
// Delta E everywhere: Euclidean distance in OKLab x 100.
export const MACHADO = {
  protan: [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
  deutan: [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881]],
  tritan: [[1.255528, -0.076749, -0.178779], [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.303900]],
};
export const hex2srgb = h => { h = h.replace('#', ''); return [0, 2, 4].map(i => parseInt(h.slice(i, i + 2), 16) / 255); };
export const s2lin = c => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
export const lin2s = c => { c = Math.max(0, Math.min(1, c)); return c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055; };
export const lin = h => hex2srgb(h).map(s2lin);
export const lin2hex = l => '#' + l.map(c => Math.round(lin2s(c) * 255).toString(16).padStart(2, '0')).join('');
export const relLum = h => { const [r, g, b] = lin(h); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
export const contrast = (a, b) => { const [hi, lo] = [relLum(a), relLum(b)].sort((x, y) => y - x); return (hi + 0.05) / (lo + 0.05); };
export function oklabFromLin([r, g, b]) {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
          1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
          0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s];
}
export function linFromOklab([L, a, b]) {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.2914855480 * b) ** 3;
  return [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
          -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
          -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s];
}
export const oklab = h => oklabFromLin(lin(h));
export const oklch = h => { const [L, a, b] = oklab(h); return [L, Math.hypot(a, b), ((Math.atan2(b, a) * 180 / Math.PI) % 360 + 360) % 360]; };
export const inGamut = l => l.every(c => c >= -1e-4 && c <= 1 + 1e-4);
// OKLCH -> hex; chroma is reduced until the colour fits sRGB.
export function lch2hex(L, C, H) {
  const hr = H * Math.PI / 180;
  let lo = 0, hi = C, l = linFromOklab([L, C * Math.cos(hr), C * Math.sin(hr)]);
  if (inGamut(l)) return lin2hex(l);
  for (let i = 0; i < 24; i++) { const mid = (lo + hi) / 2; l = linFromOklab([L, mid * Math.cos(hr), mid * Math.sin(hr)]); if (inGamut(l)) lo = mid; else hi = mid; }
  return lin2hex(linFromOklab([L, lo * Math.cos(hr), lo * Math.sin(hr)]));
}
export function simLin(l, kind) {
  const M = MACHADO[kind], c = v => Math.max(0, Math.min(1, v));
  return [c(M[0][0] * l[0] + M[0][1] * l[1] + M[0][2] * l[2]), c(M[1][0] * l[0] + M[1][1] * l[1] + M[1][2] * l[2]), c(M[2][0] * l[0] + M[2][1] * l[1] + M[2][2] * l[2])];
}
export const simHex = (h, kind) => lin2hex(simLin(lin(h), kind));
export function deltaE(h1, h2, kind) {
  const a = oklabFromLin(kind ? simLin(lin(h1), kind) : lin(h1)), b = oklabFromLin(kind ? simLin(lin(h2), kind) : lin(h2));
  return 100 * Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}
// worst pair over all pairs of a palette: {cvd: min over protan+deutan, normal, tritan}
export function separation(p) {
  const out = { cvd: [1e9], normal: [1e9], tritan: [1e9], protan: [1e9], deutan: [1e9] };
  for (let i = 0; i < p.length; i++) for (let j = i + 1; j < p.length; j++) {
    for (const k of ['protan', 'deutan', 'tritan']) { const d = deltaE(p[i], p[j], k); if (d < out[k][0]) out[k] = [d, p[i], p[j]]; if (k !== 'tritan' && d < out.cvd[0]) out.cvd = [d, p[i], p[j], k]; }
    const d = deltaE(p[i], p[j]); if (d < out.normal[0]) out.normal = [d, p[i], p[j]];
  }
  return out;
}
