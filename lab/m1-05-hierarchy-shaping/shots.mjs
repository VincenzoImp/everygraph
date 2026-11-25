// Experiment #19 (m1-05): screenshots of the lab viewer at the targets of targets.py.
// Usage (from a directory whose node_modules has playwright): node shots.mjs URL TARGETS_JSON OUT_PREFIX
// Headless Chromium through ANGLE on the GPU, as lab/m1-01-tracer/capture.mjs; 1600x1000.
import { chromium } from 'playwright'; import fs from 'node:fs';
const [base, targetsJson, outPrefix] = process.argv.slice(2);
const T = JSON.parse(fs.readFileSync(targetsJson, 'utf8')).targets;
const browser = await chromium.launch({ headless: true, channel: 'chromium', args: ['--use-gl=angle', '--use-angle=gl-egl', '--ignore-gpu-blocklist', '--enable-gpu'] });
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
const errors = [];
page.on('pageerror', e => errors.push(String(e)));
page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
await page.goto(base + '&z=1.6');
await page.waitForFunction(() => window.__gm && window.__gm.pm.root, null, { timeout: 60000 });
const idle = async () => { await page.waitForFunction(() => window.__gm.idle(), null, { timeout: 120000 }); await page.waitForTimeout(300); };
await idle();
const shots = [];
for (const t of T) {
  await page.evaluate(([x, y, z]) => window.__gm.flyTo(x, y, z, 900), [t.x, t.y, t.z]);
  await page.waitForTimeout(1000); await idle();
  const path = `${outPrefix}-${t.name}.png`;
  // no mouse move: a hover would draw the highlight ring
  await page.screenshot({ path });
  shots.push({ ...t, path, renderer: await page.evaluate(() => window.__gm.renderer) });
}
await browser.close();
// the page asks for /favicon.ico, which the lab server does not have
const fatal = errors.filter(e => !/status of 404/.test(e));
console.log(JSON.stringify({ shots, errors }));
if (fatal.length) process.exit(1);
