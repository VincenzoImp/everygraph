// SPDX-License-Identifier: Apache-2.0 OR MIT
// Measure the colour tokens and fail if any falls below its floor.
//   node design/system/tools/measure.mjs        (reads ../tokens.json; run build.py first)
// Contrast: WCAG 2 relative luminance. Separation: ΔE = OKLab distance × 100, colour-blind
// vision simulated with Machado et al. (2009) at severity 1 (color.mjs).
import fs from 'node:fs';
import { contrast, deltaE, oklch } from './color.mjs';

const T = JSON.parse(fs.readFileSync(new URL('../tokens.json', import.meta.url), 'utf8'));
const THEMES = ['light', 'dark'];
const tok = (theme, name) => T[theme][name];
const series = (p, n) => Array.from({ length: n }, (_, i) => p + (i + 1));
const failures = [];
const check = (ok, what) => { if (!ok) failures.push(what); return ok ? 'ok' : 'FAIL'; };

// The pairs of foundations.html § 9 and where each is used. Text 4.5:1; large text,
// icons and control borders 3:1 (WCAG 2.2, 1.4.3 and 1.4.11).
const NEED = { text: 4.5, graphic: 3 };
const PAIRS = [
  ['ink', 'bg', 'text', 'body text on the page'], ['ink', 'surface', 'text', 'text in cards, panels, inputs'],
  ['ink', 'accent-soft', 'text', 'text in a selected row'],
  ['ink-2', 'bg', 'text', 'secondary text'], ['ink-2', 'surface-2', 'text', 'secondary text in wells'],
  ['muted', 'bg', 'text', 'captions'], ['muted', 'surface', 'text', 'placeholders in inputs'],
  ['muted', 'surface-2', 'text', 'comments in code blocks'], ['muted', 'accent-soft', 'text', 'caption in a selected row'],
  ['accent-ink', 'bg', 'text', 'links'], ['accent-ink', 'surface-2', 'text', 'keywords in code blocks'],
  ['accent-ink', 'accent-soft', 'text', 'accent tag; quiet button, hover'],
  ['on-accent', 'accent', 'text', 'label of the primary button'],
  ['ok', 'bg', 'text', 'status text'], ['ok', 'ok-soft', 'text', 'tag "done"'], ['ok', 'surface-2', 'text', 'strings in code blocks'],
  ['warn', 'bg', 'text', 'status text'], ['warn', 'warn-soft', 'text', 'tag "cut short"'], ['warn', 'surface-2', 'text', 'numbers in code blocks'],
  ['err', 'bg', 'text', 'status text'], ['err', 'surface', 'text', 'error under a field; destructive button'],
  ['err', 'err-soft', 'text', 'tag "failed"'],
  ['info', 'bg', 'text', 'status text'], ['info', 'info-soft', 'text', 'tag "estimate"'],
  ['syn-keyword', 'surface-2', 'text', 'keywords in code blocks'], ['syn-string', 'surface-2', 'text', 'strings in code blocks'],
  ['syn-number', 'surface-2', 'text', 'numbers in code blocks'],
  ['line-strong', 'surface', 'graphic', 'border of inputs and buttons'], ['line-strong', 'bg', 'graphic', 'border of controls on the page'],
  ['focus', 'bg', 'graphic', 'focus ring; selected tab'], ['focus', 'surface', 'graphic', 'focus ring inside a panel'],
  ['term-ink', 'term-bg', 'text', 'terminal output'], ['term-dim', 'term-bg', 'text', 'terminal secondary text'],
  ['term-ok', 'term-bg', 'text', 'terminal "done"'], ['term-warn', 'term-bg', 'text', 'terminal caution'],
  ['term-err', 'term-bg', 'text', 'terminal failure'], ['term-accent', 'term-bg', 'text', 'terminal next command'],
  ['map-ink', 'map-bg', 'text', 'labels and selection on the map'],
];
console.log('Contrast (needs | light | dark)');
for (const [fg, bg, kind, use] of PAIRS) {
  const r = THEMES.map(t => contrast(tok(t, fg), tok(t, bg)));
  const v = check(r.every(x => x >= NEED[kind]), `--${fg} on --${bg} (${use}): ${r.map(x => x.toFixed(2)).join(' / ')} < ${NEED[kind]}`);
  console.log(`  ${v.padEnd(4)} --${fg} on --${bg}`.padEnd(42) + `${NEED[kind]}:1 | ${r.map(x => x.toFixed(2)).join(' | ')}  ${use}`);
}
for (const bg of ['bg', 'map-bg']) {
  const r = THEMES.map(t => Math.min(...series('d', 10).map(d => contrast(tok(t, d), tok(t, bg)))));
  const v = check(r.every(x => x >= NEED.graphic), `worst --d1…10 on --${bg}: ${r.map(x => x.toFixed(2)).join(' / ')} < 3`);
  console.log(`  ${v.padEnd(4)} --d1…10 on --${bg} (worst)`.padEnd(42) + `3:1 | ${r.map(x => x.toFixed(2)).join(' | ')}  data marks`);
}

// Categorical separation (design/preview/style/SPEC.md, "The data palette is measured").
// The five region colours are used without labels; all ten only with labels.
const VISION = [null, 'protan', 'deutan'];
const worst = (p, k) => { let w = 1e9; for (let i = 0; i < p.length; i++) for (let j = i + 1; j < p.length; j++) w = Math.min(w, deltaE(p[i], p[j], k)); return w; };
const FLOORS = [['d1–d5, regions', 5, [15, 8, 8]], ['d1–d10, with labels', 10, [15, 6, 6]]];
console.log('\nCategorical worst pair, ΔE (normal / protan / deutan; floor in brackets)');
for (const t of THEMES) for (const [name, n, floor] of FLOORS) {
  const p = series('d', n).map(d => tok(t, d)), w = VISION.map(k => worst(p, k));
  const v = check(w.every((x, i) => x >= floor[i]), `${t} ${name}: ${w.map(x => x.toFixed(1)).join(' / ')} below ${floor.join(' / ')}`);
  console.log(`  ${v.padEnd(4)} ${t.padEnd(5)} ${name.padEnd(20)} ${w.map(x => x.toFixed(1)).join(' / ')}  [${floor.join(' / ')}]`);
}

// No data colour equals the accent or a status colour (SPEC.md, "Settled while building").
// Accent floor: the ΔE the palette search keeps (search-palette.mjs). Status colours always
// come with an icon or a word, so their floor is only "not equal": ΔE 2, about one
// just-noticeable difference in OKLab.
const FLOOR = { light: { accent: 10, ok: 2, warn: 2, err: 2, info: 2 }, dark: { accent: 8, ok: 2, warn: 2, err: 2, info: 2 } };
console.log('\nNearest data colour to each chrome colour, ΔE normal vision');
for (const t of THEMES) for (const c of Object.keys(FLOOR[t])) {
  const [x, d] = series('d', 10).map(d => [deltaE(tok(t, d), tok(t, c)), d]).sort((a, b) => a[0] - b[0])[0];
  const v = check(x >= FLOOR[t][c], `${t}: --${d} is ΔE ${x.toFixed(1)} from --${c}, floor ${FLOOR[t][c]}`);
  console.log(`  ${v.padEnd(4)} ${t.padEnd(5)} --${c.padEnd(6)} nearest --${d}: ${x.toFixed(1)}  [${FLOOR[t][c]}]`);
}

// Ramps: sequential lightness moves one way; the diverging arms are symmetric in lightness.
console.log('\nRamps, OKLab lightness');
for (const t of THEMES) {
  const L = p => series(p, 7).map(s => oklch(tok(t, s))[0]);
  const s = L('s'), dir = Math.sign(s[6] - s[0]), mono = s.slice(1).every((x, i) => Math.sign(x - s[i]) === dir);
  console.log(`  ${check(mono, `${t} sequential lightness is not monotonic`).padEnd(4)} ${t.padEnd(5)} sequential ${s.map(x => x.toFixed(2)).join(' ')}`);
  const v = L('v'), asym = Math.max(...[0, 1, 2].map(i => Math.abs(v[i] - v[6 - i])));
  console.log(`  ${check(asym <= 0.05, `${t} diverging arms differ in lightness by ${asym.toFixed(3)}`).padEnd(4)} ${t.padEnd(5)} diverging  ${v.map(x => x.toFixed(2)).join(' ')}`);
}

if (failures.length) { console.error(`\n${failures.length} failed:\n  ` + failures.join('\n  ')); process.exit(1); }
console.log('\nAll measurements pass.');
