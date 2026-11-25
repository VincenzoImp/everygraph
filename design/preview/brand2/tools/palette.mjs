#!/usr/bin/env node
// Builds src/palettes.json from src/tokens.json and prints the checks.
//   node tools/palette.mjs          (re-runs the search; deterministic, about a minute)
// Every colour-blindness figure shown on systems.html comes from this file's output.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { lch2hex, oklch, contrast, deltaE, separation, simHex } from './color.mjs';
import { optimise, order } from './optimise.mjs';
const here = path.dirname(fileURLToPath(import.meta.url));
const T = JSON.parse(fs.readFileSync(path.join(here, '../src/tokens.json'), 'utf8'));

// --- categorical search settings per system and surface ---------------------
const R1 = { // round 1, direction B, kept unchanged for the baseline
  light: '#9e3101,#02beff,#7e218c,#eea10d,#0959c9,#6fcc7f,#a071fc,#578b1a,#f786c9,#e21880'.split(','),
  dark: '#a73f33,#d5d204,#3749e5,#0fefc0,#9926a2,#fd976b,#8585e6,#7c9624,#fc93ef,#f23798'.split(','),
};
const SEARCH = {
  2: { light: { Lmin: .36, Lmax: .86, Cmin: .07, Cmax: .16, cmin: 2.2 }, dark: { Lmin: .50, Lmax: .90, Cmin: .07, Cmax: .16, cmin: 3 } },
  3: { light: { Lmin: .36, Lmax: .80, Cmin: .09, Cmax: .20, cmin: 2.5 }, dark: { Lmin: .56, Lmax: .93, Cmin: .09, Cmax: .21, cmin: 3.5 } },
  4: { light: { Lmin: .38, Lmax: .82, Cmin: .06, Cmax: .125, cmin: 2.2 }, dark: { Lmin: .52, Lmax: .90, Cmin: .06, Cmax: .125, cmin: 3 } },
  5: { both: { Lmin: .30, Lmax: .92, Cmin: .12, Cmax: .25, cmin: 2.6, cmin2: 2.6 } },
};
const sepOf = (p, surface) => { const s = separation(p); return {
  cvd: +s.cvd[0].toFixed(1), cvdPair: [s.cvd[1], s.cvd[2]], cvdKind: s.cvd[3], protan: +s.protan[0].toFixed(1), deutan: +s.deutan[0].toFixed(1),
  normal: +s.normal[0].toFixed(1), tritan: +s.tritan[0].toFixed(1),
  minContrast: +Math.min(...p.map(c => contrast(c, surface))).toFixed(2), below3: p.filter(c => contrast(c, surface) < 3).length }; };

// --- ramps -------------------------------------------------------------------
const seq = (h, cmax = .15) => [0.95, 0.88, 0.79, 0.69, 0.58, 0.47, 0.36].map((L, i) => lch2hex(L, cmax * [0.25, 0.5, 0.8, 1, 1, 0.9, 0.75][i], h));
const div = (h1, h2, dark) => { const Ls = dark ? [0.84, 0.66, 0.48] : [0.42, 0.60, 0.79], Cs = dark ? [0.11, 0.13, 0.10] : [0.13, 0.13, 0.08];
  const arm = h => Ls.map((L, i) => lch2hex(L, Cs[i], h)); const mid = dark ? lch2hex(0.34, 0.004, 250) : lch2hex(0.93, 0.004, 90);
  return [...arm(h1), mid, ...arm(h2).reverse()]; };

// --- accent vs status ----------------------------------------------------------
const statusTable = t => Object.fromEntries(['ok', 'warn', 'err', 'info'].map(k => [k, {
  normal: +deltaE(t.accent, t[k]).toFixed(1), protan: +deltaE(t.accent, t[k], 'protan').toFixed(1), deutan: +deltaE(t.accent, t[k], 'deutan').toFixed(1) }]));
const textChecks = t => ({ ink: +contrast(t.ink, t.bg).toFixed(1), ink2: +contrast(t.ink2, t.bg).toFixed(1), muted: +contrast(t.muted, t.bg).toFixed(1),
  accentInk: +contrast(t.accentInk, t.bg).toFixed(1), onAccent: +contrast(t.onAccent, t.accent).toFixed(1),
  ok: +contrast(t.ok, t.bg).toFixed(1), warn: +contrast(t.warn, t.bg).toFixed(1), err: +contrast(t.err, t.bg).toFixed(1), info: +contrast(t.info, t.bg).toFixed(1) });

const out = {};
for (const id of Object.keys(T)) {
  const s = T[id], o = out[id] = { cat: {}, sep: {}, sim: {}, status: {}, text: {} };
  if (id === '1') { o.cat = R1; }
  else if (SEARCH[id].both) {
    const p = order(optimise({ surface: s.light.bg, surface2: s.dark.bg, seed: 11, restarts: 8, ...SEARCH[id].both }), s.firstHue);
    o.cat = { light: p, dark: p };
  } else for (const th of ['light', 'dark'])
    o.cat[th] = order(optimise({ surface: s[th].bg, seed: 11, restarts: 8, avoid: s[th].hl ? [[s[th].hl, 14]] : [], ...SEARCH[id][th] }), s.firstHue);
  for (const th of ['light', 'dark']) { o.sep[th] = sepOf(o.cat[th], s[th].bg); o.sim[th] = { protan: o.cat[th].map(c => simHex(c, 'protan')), deutan: o.cat[th].map(c => simHex(c, 'deutan')) }; o.status[th] = statusTable(s[th]); o.text[th] = textChecks(s[th]); }
  o.seq = id === '1' ? '#ffe9dc,#ffcdb0,#ffab80,#ff8550,#f55e1f,#c6440d,#8f2f06'.split(',') : seq(s.seqHue, id === '4' ? .09 : .15);
  o.div = { light: div(s.div[0], s.div[1], false), dark: div(s.div[0], s.div[1], true) };
  if (s.light.hl) o.hl = Object.fromEntries(['light', 'dark'].map(th => [th, { vsWarn: +deltaE(s[th].hl, s[th].warn).toFixed(1), vsWarnCvd: +Math.min(deltaE(s[th].hl, s[th].warn, 'protan'), deltaE(s[th].hl, s[th].warn, 'deutan')).toFixed(1) }]));
}
fs.writeFileSync(path.join(here, '../src/palettes.json'), JSON.stringify(out, null, 1));

for (const id of Object.keys(T)) { const o = out[id]; console.log(`\n== ${id} ${T[id].name}`);
  for (const th of ['light', 'dark']) { const p = o.sep[th];
    console.log(`  ${th.padEnd(5)} categorical: worst pair protan/deutan ${p.cvd} (${p.cvdKind} ${p.cvdPair.join(' ')}) · normal ${p.normal} · tritan ${p.tritan} · min contrast ${p.minContrast} (${p.below3} below 3:1)`);
    console.log(`        ${o.cat[th].join(',')}`);
    console.log(`        accent vs status (normal/protan/deutan): ` + Object.entries(o.status[th]).map(([k, v]) => `${k} ${v.normal}/${v.protan}/${v.deutan}`).join(' · '));
    console.log(`        text contrast: ` + Object.entries(o.text[th]).map(([k, v]) => `${k} ${v}`).join(' · ')); }
  if (o.hl) console.log('  highlight vs warning:', JSON.stringify(o.hl));
  console.log(`  seq ${o.seq.join(',')}\n  div ${o.div.light.join(',')} | dark ${o.div.dark.join(',')}`); }
