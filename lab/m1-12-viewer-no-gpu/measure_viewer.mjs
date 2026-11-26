// m1-12 (#26): the lab viewer (lab/e-map/viewer) without a discrete GPU. Exploratory: the host is shared, so cap
// settings are compared interleaved in one page, and absolute times are labelled exploratory.
//   node measure_viewer.mjs VIEWER_URL GRAPH CONFIG TARGET.json OUT.json
// Writes: first-view bytes and requests; the cost of one frame (render + readPixels) at three zooms for a set of
// per-frame caps and canvas scales; frames drawn per second in a scripted session, per drawing mode.
import fs from 'node:fs';
import { open, frameStats } from './browsers.mjs';

const [viewerUrl, graph, config, targetJson, out] = process.argv.slice(2);
const target = JSON.parse(fs.readFileSync(targetJson, 'utf8'));
const ROUNDS = +(process.env.ROUNDS || 3), PER = +(process.env.PER || 20);
// [edges, nodes, bubbles, canvas scale]
// entities against pixels: caps at all, 600, 60 and 0 edges and nodes, canvas scales 1, 0.5 and 0.25, and a frame
// with no instances at all (clear and labels only)
const CAPS = [[1e9, 1e9, 1e9, 1], [600, 600, 1e9, 1], [60, 60, 1e9, 1], [0, 0, 1e9, 1], [1e9, 1e9, 1e9, 0.5], [1e9, 1e9, 1e9, 0.25], [600, 600, 1e9, 0.5], [600, 600, 1e9, 0.25], [0, 0, 0, 1]];
const VIEWS = [['overview', 0.5, 0.5, 1.9], ['region', target.x, target.y, 6.6], ['nodes', target.x, target.y, 10.6]];
const res = { graph, config, target, rounds: ROUNDS, frames_per_sample: PER, note: 'exploratory: shared host; compare rows within one table' };

const { browser, page, errors, version } = await open(config);
res.browser = version;
// a busy software renderer delays navigation and load well past Playwright's 30 s default
page.setDefaultTimeout(300000);
const save = () => fs.writeFileSync(out, JSON.stringify(res, null, 1));
const url = (q) => `${viewerUrl}?file=../out/${graph}.pmtiles&z=1.9${q}`;
const idle = () => page.waitForFunction(() => window.__gm && window.__gm.idle && window.__gm.idle(), null, { timeout: 120000 });
const t0 = Date.now();
await page.goto(url('&adaptive=0&motion=0&onchange=0&edges=1000000000&nodes=1000000000'));
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
    await page.evaluate(([e, n, b, sc]) => window.__gm.setCaps(e, n, b, sc), CAPS[c]);
    await page.evaluate(() => window.__gm.renderCost(2));
    acc[c].push(...await page.evaluate((k) => window.__gm.renderCost(k), PER));
    drawn[c] = await page.evaluate(() => window.__gm.drawn());
  }
  const all = x => x >= 1e9 ? 'all' : x, base = frameStats(acc[0]).p50_ms;
  res.frame_cost[vname] = CAPS.map((cp, c) => ({ edges_cap: all(cp[0]), nodes_cap: all(cp[1]), bubbles_cap: all(cp[2]), scale: cp[3], drawn: drawn[c], ms: frameStats(acc[c]), p50_vs_uncapped: +(frameStats(acc[c]).p50_ms / base).toFixed(2) }));
}
await page.evaluate(() => window.__gm.setCaps(1e9, 1e9));
save();

// 2. a scripted session (fly to the nodes in four steps, stay still, wheel back out) in each drawing mode, the modes
// alternating. A phase reports frames drawn per second of wall time and the 95th percentile of the frame interval.
const ALL = '&edges=1000000000&nodes=1000000000';
const MODES = {
  fixed: '&adaptive=0&motion=0&onchange=0' + ALL,
  adaptive: '&adaptive=1&motion=0&onchange=0' + ALL,
  'scale 0.5 while moving, draw on change': '&adaptive=0&motion=0.5&onchange=1' + ALL,
  'scale 0.25 while moving, draw on change': '&adaptive=0&motion=0.25&onchange=1' + ALL,
  'scale 0.5 while moving, edges 600, draw on change': '&adaptive=0&motion=0.5&onchange=1&edges=600&nodes=600',
};
async function session(mode) {
  await page.goto('about:blank');
  await page.goto(url(MODES[mode])); await page.waitForFunction(() => window.__gm && window.__gm.pm?.root); await idle();
  const phases = {};
  const phase = async (name, act) => {
    await page.evaluate(() => { const s = window.__gm.stats; s.frames.length = 0; s.recording = true; s.r0 = s.rendered || 0; s.t0 = performance.now(); });
    await act();
    phases[name] = await page.evaluate(() => { const s = window.__gm.stats; s.recording = false; const sec = (performance.now() - s.t0) / 1000;
      return { frames_drawn: (s.rendered || 0) - s.r0, seconds: +sec.toFixed(2), drawn_per_s: +(((s.rendered || 0) - s.r0) / sec).toFixed(1), intervals: s.frames.slice() }; });
    phases[name].intervals = frameStats(phases[name].intervals);
  };
  for (const z of [4.2, 6.6, 8.6, 10.6]) {
    await phase('fly to z' + z, async () => { await page.evaluate(([x, y, z]) => window.__gm.flyTo(x, y, z, 1800), [target.x, target.y, z]); await page.waitForTimeout(1900); });
    await idle();
  }
  await phase('still at nodes, 3 s', () => page.waitForTimeout(3000));
  await page.mouse.move(800, 500);
  await phase('wheel out', async () => { for (let i = 0; i < 40; i++) { await page.mouse.wheel(0, 120); await page.waitForTimeout(40); } await page.waitForTimeout(800); });
  return { mode, phases, caps_log: await page.evaluate(() => window.__gm.caps.log), final_caps: await page.evaluate(() => { const c = window.__gm.caps; return { edges: c.edges, nodes: c.nodes, bubbles: c.bubbles, scale: c.scale }; }) };
}
res.sessions = [];
for (let r = 0; r < +(process.env.SESSION_ROUNDS || 2); r++) for (const m of Object.keys(MODES)) { res.sessions.push(await session(m)); save(); }
res.page_errors = errors;
save();
console.log(JSON.stringify({ config, renderer: res.renderer, first_view: res.first_view, nodes: res.frame_cost.nodes.map(r => [r.edges_cap, r.nodes_cap, r.bubbles_cap, r.scale, r.p50_vs_uncapped]), sessions: res.sessions.map(s => [s.mode, Object.fromEntries(Object.entries(s.phases).map(([k, v]) => [k, v.drawn_per_s])), s.final_caps]) }, null, 1));
await browser.close();
