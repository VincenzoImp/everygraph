// m1-06 (#20): screenshots of the preview on lj and orkut (JPEG, 1440 x 900).
//   node shots.mjs OVERVIEW_URL OUT_DIR
import { chromium } from 'playwright';

const [url, dir] = process.argv.slice(2);
const STATES = [
  ['lj-1-overview', ''], ['lj-2-matrix', 'matrix=1'], ['lj-3-strongest-12', 'links=12'], ['lj-4-selection', 'sel=15'],
  ['lj-5-inside-region-2', 'p=2&matrix=1'], ['lj-6-light', 'theme=light&links=12'],
  ['orkut-1-overview', 'g=orkut'], ['orkut-2-matrix', 'g=orkut&matrix=1'], ['orkut-3-strongest-12', 'g=orkut&links=12'], ['orkut-4-inside-region-1', 'g=orkut&p=10&matrix=1'],
];
const b = await chromium.launch({ args: ['--use-angle=gl-egl', '--ignore-gpu-blocklist'] });
const p = await b.newPage({ viewport: { width: 1440, height: 900 } });
const errors = []; p.on('pageerror', e => errors.push(String(e)));
for (const [name, hash] of STATES) {
  await p.goto('about:blank'); await p.goto(url + (hash ? '#' + hash : '')); await p.waitForFunction(() => window.__ov); await p.waitForTimeout(700);
  await p.screenshot({ path: `${dir}/${name}.jpg`, type: 'jpeg', quality: 80 });
}
await b.close();
if (errors.length) { console.error(errors.join('\n')); process.exit(1); }
console.log(STATES.length + ' screenshots');
