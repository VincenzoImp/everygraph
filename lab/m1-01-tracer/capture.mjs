// SPDX-License-Identifier: Apache-2.0 OR MIT
// Issue #15 (m1-01): lab/e-map/viewer/capture.mjs with the page URL as an argument and
// the four fly-to zoom levels spread up to the map's own deepest zoom (experiment E used
// 4.2, 6.6, 8.6, 10.6 for maps of zmax 9-10; here z = 1.9 + (zmax + 0.6 - 1.9) * f for
// f = 0.27, 0.54, 0.77, 1, which gives the same values at zmax 10). Otherwise the same
// session: overview, four fly-to steps towards a target community, a pan, a wheel zoom
// out, a search by original id; bytes and requests per interaction, frame times.
// Usage: node capture.mjs URL OUT_PREFIX RESULT_JSON TARGET_JSON
import { chromium } from 'playwright'; import fs from 'node:fs';
const [base, outPrefix, resultJson, targetJson] = process.argv.slice(2);
const target = JSON.parse(fs.readFileSync(targetJson, 'utf8'));
const configs = [
  // --use-angle=vulkan loses the WebGL context in headless mode (experiment E): not used.
  ['angle-gl-egl', ['--use-gl=angle', '--use-angle=gl-egl', '--ignore-gpu-blocklist', '--enable-gpu']],
  ['default', []],
];
let browser, page, renderer, cfgName; const tried = [];
for (const [name, args] of configs) {
  try {
    browser = await chromium.launch({ headless: true, channel: 'chromium', args });
    page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    page.on('console', m => { if (m.type() === 'error') console.log('console.error:', m.text()); });
    page.on('pageerror', e => console.log('pageerror:', e.message));
    await page.goto(base + '&z=1.9'); await page.waitForFunction(() => window.__gm && window.__gm.pm.root, null, { timeout: 60000 });
    renderer = await page.evaluate(() => window.__gm.renderer); tried.push([name, renderer]); cfgName = name;
    if (!/SwiftShader|llvmpipe/i.test(renderer)) break;
    if (name === configs[configs.length - 1][0]) break;
    await browser.close(); browser = null;
  } catch (e) { tried.push([name, 'FAILED ' + String(e).slice(0, 160)]); if (browser) await browser.close(); browser = null; }
}
if (!browser) { console.error('no browser configuration worked', tried); process.exit(1); }
console.log('renderer:', renderer, tried);
const zmax = await page.evaluate(() => window.__gm.pm.maxZoom);
const idle = async () => { await page.waitForFunction(() => window.__gm.idle(), null, { timeout: 120000 }); await page.waitForTimeout(250); };
const st = async () => page.evaluate(() => ({ bytes: window.__gm.stats.bytes, requests: window.__gm.stats.requests, tiles: window.__gm.stats.tiles }));
const res = { url: base, renderer, config: cfgName, tried, viewport: [1600, 1000], zmax, zooms: [1.9], interactions: [], frames: {} };
let prev = { bytes: 0, requests: 0, tiles: 0 };
const mark = async (name) => { const s = await st(); res.interactions.push({ name, bytes: s.bytes - prev.bytes, requests: s.requests - prev.requests, tiles: s.tiles - prev.tiles }); prev = s; };
const frameStats = a => { if (!a.length) return null; const s = [...a].sort((x, y) => x - y); return { n: a.length, mean_ms: +(a.reduce((x, y) => x + y, 0) / a.length).toFixed(2), p50_ms: +s[s.length >> 1].toFixed(2), p95_ms: +s[Math.floor(s.length * 0.95)].toFixed(2), max_ms: +s[s.length - 1].toFixed(2), over_33ms: a.filter(x => x > 33.4).length }; };
await idle(); await mark('initial load (header+root dir+metadata+overview tiles)');
await page.screenshot({ path: `${outPrefix}-1-overview.png` });
const span = zmax + 0.6 - 1.9;
const zooms = [[0.27, '2-continent'], [0.54, '3-region'], [0.77, '4-communities'], [1, '5-nodes-edges']].map(([f, n]) => [+(1.9 + span * f).toFixed(2), n]);
for (const [z, name] of zooms) {
  res.zooms.push(z);
  await page.evaluate(() => { window.__gm.stats.frames.length = 0; window.__gm.stats.recording = true; });
  await page.evaluate(([x, y, z]) => window.__gm.flyTo(x, y, z, 1800), [target.x, target.y, z]);
  await page.waitForTimeout(1900);
  res.frames['fly-to ' + name] = frameStats(await page.evaluate(() => { window.__gm.stats.recording = false; return window.__gm.stats.frames.slice(); }));
  await idle(); await mark('zoom to z=' + z + ' (' + name + ')');
  if (name === '5-nodes-edges') { await page.mouse.move(800, 500); await page.mouse.move(803, 502); await page.waitForTimeout(150); }
  if (name === '3-region') { await page.mouse.move(700, 430); await page.mouse.move(702, 431); await page.waitForTimeout(150); }
  await page.screenshot({ path: `${outPrefix}-${name}.png` });
  await page.mouse.move(5, 995);
}
await page.mouse.move(1200, 500); await page.mouse.down(); await page.mouse.move(400, 500, { steps: 20 }); await page.mouse.up(); await idle(); await mark('pan 800 px at node level');
await page.evaluate(() => { window.__gm.stats.frames.length = 0; window.__gm.stats.recording = true; });
await page.mouse.move(800, 500);
for (let i = 0; i < 40; i++) { await page.mouse.wheel(0, 120); await page.waitForTimeout(40); }
await page.waitForTimeout(800);
res.frames['wheel zoom-out'] = frameStats(await page.evaluate(() => { window.__gm.stats.recording = false; return window.__gm.stats.frames.slice(); }));
await idle(); await mark('wheel zoom-out to overview');
const b0 = await st();
await page.fill('#q', String(target.orig));
await page.evaluate(() => { window.__gm.stats.frames.length = 0; window.__gm.stats.recording = true; });
await page.click('#go'); await page.waitForTimeout(2700);
res.frames['search fly-to'] = frameStats(await page.evaluate(() => { window.__gm.stats.recording = false; return window.__gm.stats.frames.slice(); }));
await idle(); const b1 = await st();
res.search = { id: target.orig, text: await page.textContent('#sres'), total_bytes_incl_tiles: b1.bytes - b0.bytes, requests: b1.requests - b0.requests };
await mark('search + fly-to node (cold index)');
await page.screenshot({ path: `${outPrefix}-6-search.png` });
res.total = await st();
res.context_lost = await page.evaluate(() => document.getElementById('gl').getContext('webgl2').isContextLost());
fs.writeFileSync(resultJson, JSON.stringify(res, null, 1));
await browser.close();
