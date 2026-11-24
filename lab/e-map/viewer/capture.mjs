// Playwright capture: screenshots at 5 zoom levels, hover, search fly-to, frame-time stats, bytes per interaction.
import { chromium } from 'playwright'; import fs from 'node:fs';
const [graph, outPrefix, resultJson, targetJson] = process.argv.slice(2);
const target = JSON.parse(fs.readFileSync(targetJson, 'utf8'));
const base = `http://127.0.0.1:8765/viewer/index.html?file=../out/${graph}.pmtiles`;
const configs = [
  // note: --use-angle=vulkan also reports the RTX 4090 but loses the WebGL context in headless mode (blank canvas) -> not used
  ['angle-gl-egl', ['--use-gl=angle', '--use-angle=gl-egl', '--ignore-gpu-blocklist', '--enable-gpu']],
  ['default', []],
];
let browser, page, renderer, cfgName; const tried = [];
if (process.env.FORCE_SOFTWARE) configs.splice(0, configs.length - 1);
for (const [name, args] of configs) {
  try {
    browser = await chromium.launch({ headless: true, channel: 'chromium', args });
    page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    page.on('console', m => { if (m.type() === 'error') console.log('console.error:', m.text()); });
    page.on('pageerror', e => console.log('pageerror:', e.message));
    await page.goto(base + '&z=1.9'); await page.waitForFunction(() => window.__gm && window.__gm.pm.root, null, { timeout: 15000 });
    renderer = await page.evaluate(() => window.__gm.renderer); tried.push([name, renderer]); cfgName = name;
    if (!/SwiftShader|llvmpipe/i.test(renderer) || process.env.FORCE_SOFTWARE) break;
    await browser.close(); browser = null;
  } catch (e) { tried.push([name, 'FAILED ' + String(e).slice(0, 120)]); if (browser) await browser.close(); browser = null; }
}
if (!browser) { // fall back to software
  browser = await chromium.launch({ headless: true, channel: 'chromium' }); page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  await page.goto(base + '&z=1.9'); await page.waitForFunction(() => window.__gm && window.__gm.pm.root); renderer = await page.evaluate(() => window.__gm.renderer); cfgName = 'default(software)';
}
console.log('renderer:', renderer, tried);
const idle = async () => { await page.waitForFunction(() => window.__gm.idle(), null, { timeout: 30000 }); await page.waitForTimeout(250); };
const st = async () => page.evaluate(() => ({ bytes: window.__gm.stats.bytes, requests: window.__gm.stats.requests, tiles: window.__gm.stats.tiles }));
const res = { graph, renderer, config: cfgName, tried, viewport: [1600, 1000], interactions: [], frames: {} };
let prev = { bytes: 0, requests: 0, tiles: 0 };
const mark = async (name) => { const s = await st(); res.interactions.push({ name, bytes: s.bytes - prev.bytes, requests: s.requests - prev.requests, tiles: s.tiles - prev.tiles }); prev = s; };
const frameStats = a => { if (!a.length) return null; const s = [...a].sort((x, y) => x - y); return { n: a.length, mean_ms: +(a.reduce((x, y) => x + y, 0) / a.length).toFixed(2), p50_ms: +s[s.length >> 1].toFixed(2), p95_ms: +s[Math.floor(s.length * 0.95)].toFixed(2), max_ms: +s[s.length - 1].toFixed(2), over_33ms: a.filter(x => x > 33.4).length }; };
await idle(); await mark('initial load (header+root dir+metadata+overview tiles)');
await page.screenshot({ path: `${outPrefix}-1-overview.png` });
const zooms = [[4.2, '2-continent'], [6.6, '3-region'], [8.6, '4-communities'], [10.6, '5-nodes-edges']];
for (const [z, name] of zooms) {
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
// pan by one screen width at node level
await page.mouse.move(1200, 500); await page.mouse.down(); await page.mouse.move(400, 500, { steps: 20 }); await page.mouse.up(); await idle(); await mark('pan 800 px at node level');
// wheel zoom out with frame timing
await page.evaluate(() => { window.__gm.stats.frames.length = 0; window.__gm.stats.recording = true; });
await page.mouse.move(800, 500);
for (let i = 0; i < 40; i++) { await page.mouse.wheel(0, 120); await page.waitForTimeout(40); }
await page.waitForTimeout(800);
res.frames['wheel zoom-out 12 levels'] = frameStats(await page.evaluate(() => { window.__gm.stats.recording = false; return window.__gm.stats.frames.slice(); }));
await idle(); await mark('wheel zoom-out to overview');
// search
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
fs.writeFileSync(resultJson, JSON.stringify(res, null, 1)); console.log(JSON.stringify(res, null, 1));
await browser.close();
