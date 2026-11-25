// Prints the rendered height of a page state, so a screenshot can be sized to fit.
//   node tools/measure.mjs systems.html#only=2 [width=1440] [selector]
import path from 'node:path';
const { chromium } = await import('${EVERYGRAPH_SCRATCH:-/home/tux/graphlab/scratch}/pw/node_modules/playwright/index.mjs');
const [target, w = '1440', sel = ''] = process.argv.slice(2);
const [file, hash = ''] = target.split('#');
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: +w, height: 900 } });
const errors = []; page.on('pageerror', e => errors.push(String(e))); page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
await page.goto('file://' + path.resolve(file) + (hash ? '#' + hash : ''));
await page.waitForTimeout(400);
const out = await page.evaluate(sel => {
  const r = { height: document.documentElement.scrollHeight, overflowX: document.documentElement.scrollWidth - window.innerWidth };
  if (sel) r.sel = [...document.querySelectorAll(sel)].map(e => { const b = e.getBoundingClientRect(); return [Math.round(b.top + scrollY), Math.round(b.height)]; });
  // elements that stick out of their parent pane horizontally (layout bugs)
  r.spill = [...document.querySelectorAll('.eg *')].filter(e => e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflowX === 'visible' && e.clientWidth > 0 && !e.closest('svg')).slice(0, 12).map(e => e.className + ':' + e.textContent.trim().slice(0, 30));
  return r;
}, sel);
await browser.close();
console.log(JSON.stringify(out));
if (errors.length) { console.error('PAGE ERRORS:\n' + errors.join('\n')); process.exit(1); }
