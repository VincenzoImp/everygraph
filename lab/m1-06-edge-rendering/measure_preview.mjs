// m1-06 (#20): what the overview costs to draw, in the preview (design/preview/style/overview.html, Canvas 2D) on lj
// and orkut. Exploratory: the host is shared, so the states are measured interleaved in one page.
//   node measure_preview.mjs OVERVIEW_URL CONFIG OUT.json      (CONFIG from lab/m1-12-viewer-no-gpu/browsers.mjs)
import fs from 'node:fs';
import { open, frameStats } from './browsers.mjs';

const [base, config, out] = process.argv.slice(2);
const ROUNDS = +(process.env.ROUNDS || 3), PER = +(process.env.PER || 20);
const { browser, page, errors, version } = await open(config, { width: 1440, height: 900 });
const res = { config, browser: version, viewport: [1440, 900], rounds: ROUNDS, frames_per_sample: PER, note: 'exploratory: shared host; Canvas 2D; a sample = the mean of 10 frames, each draw() then getImageData(1 px), which waits for the raster', graphs: {} };
for (const g of ['lj', 'orkut']) {
  await page.goto(base + '#g=' + g); await page.waitForFunction(() => window.__ov); await page.waitForTimeout(500);
  const big = await page.evaluate(() => String(window.OVERVIEW_DATA[window.__ov.state.g].parents['0'].kids[0].id));
  const STATES = [['plain', { glyph: '0', links: '0', sel: '' }], ['glyph', { glyph: '1', links: '0', sel: '' }], ['bands 12', { glyph: '1', links: '12', sel: '' }],
    ['bands 50', { glyph: '1', links: '50', sel: '' }], ['selection, 20 links', { glyph: '1', links: '0', sel: big }]];
  const acc = STATES.map(() => []), info = {};
  for (let r = 0; r < ROUNDS; r++) for (let s = 0; s < STATES.length; s++) {
    await page.evaluate(p => window.__ov.set(p), STATES[s][1]);
    acc[s].push(...await page.evaluate(k => window.__ov.cost(k), PER));
    info[STATES[s][0]] = await page.evaluate(() => window.__ov.bands());
  }
  await page.evaluate(() => window.__ov.set({ glyph: '1', links: '0', sel: '' }));
  const canvas = await page.evaluate(() => { const c = document.getElementById('map'); return [c.width, c.height]; });
  const mat = await page.evaluate(k => window.__ov.matrixCost(k), 10);
  const withMatrix = await page.evaluate(k => window.__ov.cost(k), PER);
  await page.evaluate(() => window.__ov.set({ matrix: '0' }));
  res.graphs[g] = { canvas_px: canvas, frame_ms: Object.fromEntries(STATES.map((s, i) => [s[0], frameStats(acc[i])])), bands: { 'bands 12': info['bands 12'], 'bands 50': info['bands 50'] },
    open_matrix_ms: frameStats(mat), frame_with_matrix_open_ms: frameStats(withMatrix) };
}
res.page_errors = errors;
fs.writeFileSync(out, JSON.stringify(res, null, 1)); console.log(JSON.stringify(res, null, 1));
await browser.close();
