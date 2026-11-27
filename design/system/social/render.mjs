// SPDX-License-Identifier: Apache-2.0 OR MIT
// Renders card.html to everygraph-social.png (1280 x 640) with headless Chromium. The map
// is drawn by egMap, the preview's map stand-in, taken from design/preview/style/web.html.
// Run from a directory whose node_modules has playwright:
//   node design/system/social/render.mjs
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const { chromium } = createRequire(path.join(process.cwd(), 'noop.js'))('playwright');
const here = path.dirname(fileURLToPath(import.meta.url));
const design = path.resolve(here, '../..');
const web = fs.readFileSync(path.join(design, 'preview/style/web.html'), 'utf8');
const egMap = web.slice(web.indexOf('/*EGMAP-BEGIN*/'), web.indexOf('/*EGMAP-END*/'));
if (!egMap) throw new Error('egMap not found in web.html');

const types = { '.html': 'text/html', '.css': 'text/css', '.woff2': 'font/woff2' };
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 640 }, deviceScaleFactor: 1 });
// Files are served from design/ under a made-up host, so that relative links and fonts load.
await page.route('http://design.local/**', (route) => {
  const file = path.join(design, decodeURIComponent(new URL(route.request().url()).pathname));
  if (!file.startsWith(design) || !fs.existsSync(file)) return route.fulfill({ status: 404 });
  route.fulfill({ body: fs.readFileSync(file), contentType: types[path.extname(file)] || 'application/octet-stream' });
});
await page.goto('http://design.local/system/social/card.html');
await page.addScriptTag({ content: egMap });
await page.evaluate(async () => {
  await document.fonts.ready;
  document.querySelectorAll('svg[data-map]').forEach((svg) => egMap(svg, JSON.parse(svg.dataset.map)));
});
await page.screenshot({ path: path.join(here, 'everygraph-social.png') });
await browser.close();
