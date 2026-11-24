// node js/browser_run.mjs '<json array of configs>'  -> one JSON line per config
// Launches headless Chromium (Playwright, private install), server on 127.0.0.1.
import { chromium } from 'playwright';
import { serve } from './serve.mjs';
const cfgs = JSON.parse(process.argv[2]);
const srv = await serve(0);
const base = `http://127.0.0.1:${srv.address().port}/`;
const browser = await chromium.launch({ headless: true, args: (process.env.CHROME_ARGS || '').split(' ').filter(Boolean) });
const ctx = await browser.newContext();
for (const c of cfgs) {
  const page = await ctx.newPage();
  page.on('console', (m) => { if (process.env.DEBUG) console.error('console:', m.text()); });
  await page.goto(base);
  await page.waitForFunction(() => window.ready);
  const info = await page.evaluate(() => window.info());
  let r;
  if (c.mode === 'file' || c.mode === 'file2') {
    await page.setInputFiles('#f', c.path);
    r = await page.evaluate((c) => window.runInWorker({ ...c, file: window.fileInput() }), c);
  } else {
    r = await page.evaluate((c) => window.runInWorker(c), c);
  }
  r.runtime = 'chromium-headless ' + (info.ua.match(/Chrome\/[\d.]+/) || [''])[0];
  r.crossOriginIsolated = info.isolated;
  console.log(JSON.stringify(r));
  await page.close();
}
await browser.close();
srv.close();
