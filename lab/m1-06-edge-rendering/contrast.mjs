// m1-06 (#20): WCAG contrast of what the overview draws, computed from the tokens (deterministic).
//   node contrast.mjs > results/contrast.json
// 1.4.11 asks 3:1 for graphical objects needed to understand the content; 1.4.3 asks 4.5:1 for text.
import fs from 'node:fs';

const tokens = fs.readFileSync(new URL('../../design/system/tokens.css', import.meta.url), 'utf8');
function block(css, sel) { const i = css.indexOf(sel); return css.slice(css.indexOf('{', i) + 1, css.indexOf('}', i)); }
function vars(text) { const o = {}; for (const m of text.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})/gi)) o[m[1]] = m[2].toLowerCase(); return o; }
const theme = t => vars(block(tokens, t === 'light' ? ':root, [data-theme="light"]' : '[data-theme="dark"] {'));
const rgb = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16));
const lum = h => { const [r, g, b] = rgb(h).map(v => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
const cr = (a, b) => { const x = lum(a), y = lum(b); return +((Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05)).toFixed(2); };
const over = (fg, a, bg) => '#' + rgb(fg).map((v, i) => Math.round(v * a + rgb(bg)[i] * (1 - a)).toString(16).padStart(2, '0')).join('');

const out = {};
for (const t of ['dark', 'light']) {
  const v = theme(t), bg = v['map-bg'], ink = v['map-ink'], S = [1, 2, 3, 4, 5, 6, 7].map(i => v['s' + i]);
  const A3 = t === 'light' ? 0.5 : 0.37, tint5 = c => over(c, 0.05, bg), tint12 = c => over(c, 0.12, bg);
  out[t] = {
    'band fill (ink at A3) vs canvas': cr(over(ink, A3, bg), bg), A3,
    'selection link (ink 55%) vs canvas': cr(over(ink, 0.55, bg), bg),
    'glyph meter fill (ink) vs its track (ink at A3)': cr(ink, over(ink, A3, bg)),
    'glyph meter track vs canvas': cr(over(ink, A3, bg), bg),
    'glyph tick (ink) vs canvas': cr(ink, bg),
    'region outlines d1-d5 vs canvas': [1, 2, 3, 4, 5].map(i => cr(v['d' + i], bg)),
    'region outlines d1-d5 vs their 5% tint': [1, 2, 3, 4, 5].map(i => cr(v['d' + i], tint5(v['d' + i]))),
    'region outlines d1-d5 vs their 12% tint': [1, 2, 3, 4, 5].map(i => cr(v['d' + i], tint12(v['d' + i]))),
    'matrix steps s2-s7 vs the card (surface)': S.slice(1).map(c => cr(c, v.surface)),
    'matrix: adjacent steps s2-s7': S.slice(1, 6).map((c, i) => cr(c, S[i + 2])),
    'matrix empty cell outline (line) vs surface': cr(v.line, v.surface),
    'least ink opacity reaching 3:1 on the canvas': (() => { for (let a = 0.05; a <= 1; a += 0.01) if (cr(over(ink, a, bg), bg) >= 3) return +a.toFixed(2); })(),
  };
}
console.log(JSON.stringify(out, null, 1));
