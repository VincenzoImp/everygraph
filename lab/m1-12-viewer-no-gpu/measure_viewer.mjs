// m1-12 (#26): the lab viewer (lab/e-map/viewer) without a discrete GPU. Exploratory: the host is shared, so cap
// settings are compared interleaved in one page, and absolute times are labelled exploratory.
//   node measure_viewer.mjs VIEWER_URL GRAPH CONFIG TARGET.json OUT.json
// Writes: first-view bytes and requests; the cost of one frame (render + readPixels) at three zooms for a set of
// per-frame caps; rAF frame intervals of a scripted session with fixed caps and with adaptive caps.
import fs from 'node:fs';
import { open, frameStats } from './browsers.mjs';

const [viewerUrl, graph, config, targetJson, out] = process.argv.slice(2);
const target = JSON.parse(fs.readFileSync(targetJson, 'utf8'));
const ROUNDS = +(process.env.ROUNDS || 3), PER = +(process.env.PER || 20);
const CAPS = [[1e9, 1e9], [600, 1e9], [220, 1e9], [60, 1e9], [0, 1e9], [220, 1000], [60, 400], [20, 200]];
const VIEWS = [['overview', 0.5, 0.5, 1.9], ['region', target.x, target.y, 6.6], ['nodes', target.x, target.y, 10.6]];
const res = { graph, config, target, rounds: ROUNDS, frames_per_sample: PER, note: 'exploratory: shared host; compare rows within one table' };

const { browser, page, errors, version } = await open(config);
res.browser = version;
const url = (q) => `${viewerUrl}?file=../out/${graph}.pmtiles&z=1.9${q}`;
const idle = () => page.waitForFunction(() => window.__gm && window.__gm.idle && window.__gm.idle(), null, { timeout: 120000 });
const t0 = Date.now();
await page.goto(url('&adaptive=0'));
await page.waitForFunction(() => window.__gm && (window.__gm.noWebGL2 || window.__gm.pm?.root), null, { timeout: 60000 });
if (await page.evaluate(() => !!window.__gm.noWebGL2)) {
  res.no_webgl2 = true; res.message = await page.textContent('#hud');
  fs.writeFileSync(out, JSON.stringify(res, null, 1)); console.log(JSON.stringify(res, null, 1)); await browser.close(); process.exit(0);
}
await idle();
res.renderer = await page.evaluate(() => window.__gm.renderer);
res.first_view = { ...(await page.evaluate(() => ({ bytes: window.__gm.stats.bytes, requests: window.__gm.stats.requests, tiles: window.__gm.stats.tiles }))), ms_to_idle_exploratory: Date.now() - t0 };

// 1. the cost of one frame, per cap setting, interleaved
res.frame_cost = {};
for (const [vname, x, y, z] of VIEWS) {
  await page.evaluate(([x, y, z]) => { const v = window.__gm.view; v.anim = null; v.cx = x; v.cy = y; v.zf = v.tz = z; }, [x, y, z]);
  await page.evaluate(() => window.__gm.setCaps(1e9, 1e9)); await idle(); await page.waitForTimeout(300);
  const acc = CAPS.map(() => []), drawn = [];
  for (let r = 0; r < ROUNDS; r++) for (let c = 0; c < CAPS.length; c++) {
    await page.evaluate(([e, n]) => window.__gm.setCaps(e, n), CAPS[c]);
    await page.evaluate(() => window.__gm.renderCost(2));
    acc[c].push(...await page.evaluate((k) => window.__gm.renderCost(k), PER));
    drawn[c] = await page.evaluate(() => window.__gm.drawn());
  }
  res.frame_cost[vname] = CAPS.map((cp, c) => ({ edges_cap: cp[0] >= 1e9 ? 'all' : cp[0], nodes_cap: cp[1] >= 1e9 ? 'all' : cp[1], drawn: drawn[c], ms: frameStats(acc[c]) }));
}
await page.evaluate(() => window.__gm.setCaps(1e9, 1e9));

// 2. a scripted session (fly to the nodes in four steps, wheel back out), fixed caps against adaptive caps, alternating
async function session(adaptive) {
  await page.goto(url('&adaptive=' + (adaptive ? 1 : 0))); await page.waitForFunction(() => window.__gm && window.__gm.pm?.root); await idle();
  const phases = {};
  for (const z of [4.2, 6.6, 8.6, 10.6]) {
    await page.evaluate(() => { window.__gm.stats.frames.length = 0; window.__gm.stats.recording = true; });
    await page.evaluate(([x, y, z]) => window.__gm.flyTo(x, y, z, 1800), [target.x, target.y, z]);
    await page.waitForTimeout(1900);
    phases['fly to z' + z] = await page.evaluate(() => { window.__gm.stats.recording = false; return window.__gm.stats.frames.slice(); });
    await idle();
  }
  await page.evaluate(() => { window.__gm.stats.frames.length = 0; window.__gm.stats.recording = true; });
  await page.waitForTimeout(3000);
  phases['still at nodes, 3 s'] = await page.evaluate(() => { window.__gm.stats.recording = false; return window.__gm.stats.frames.slice(); });
  await page.mouse.move(800, 500);
  await page.evaluate(() => { window.__gm.stats.frames.length = 0; window.__gm.stats.recording = true; });
  for (let i = 0; i < 40; i++) { await page.mouse.wheel(0, 120); await page.waitForTimeout(40); }
  await page.waitForTimeout(800);
  phases['wheel out'] = await page.evaluate(() => { window.__gm.stats.recording = false; return window.__gm.stats.frames.slice(); });
  const all = Object.values(phases).flat();
  return { adaptive, all: frameStats(all), phases: Object.fromEntries(Object.entries(phases).map(([k, v]) => [k, frameStats(v)])), caps_log: await page.evaluate(() => window.__gm.caps.log), final_caps: await page.evaluate(() => ({ edges: window.__gm.caps.edges, nodes: window.__gm.caps.nodes })) };
}
res.sessions = [];
for (let r = 0; r < 2; r++) for (const a of [false, true]) res.sessions.push(await session(a));
res.page_errors = errors;
fs.writeFileSync(out, JSON.stringify(res, null, 1));
console.log(JSON.stringify({ config, renderer: res.renderer, first_view: res.first_view, nodes: res.frame_cost.nodes.map(r => [r.edges_cap, r.nodes_cap, r.drawn.edges, r.drawn.nodes, r.ms.p50_ms]), sessions: res.sessions.map(s => [s.adaptive, s.all.p50_ms, s.all.p95_ms, s.all.over_33ms, s.all.n, s.final_caps]) }, null, 1));
await browser.close();
