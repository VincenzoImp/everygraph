// m1-12 (#26): what each configuration offers: WebGL2 renderer, workers, DecompressionStream, HTTP range requests,
// and the frame interval of an empty page (to check that the frame rate is uncapped).
//   node probe.mjs URL_OF_ANY_FILE_ON_THE_RANGE_SERVER OUT.json
import fs from 'node:fs';
import { CONFIGS, open, renderer, frameStats } from './browsers.mjs';

const [url, out] = process.argv.slice(2);
const res = {};
for (const name of Object.keys(CONFIGS)) {
  try {
    const { browser, page, version } = await open(name);
    await page.goto(url.replace(/[^/]*$/, '') + 'index.html').catch(() => {});
    const r = { version, renderer: await renderer(page) };
    Object.assign(r, await page.evaluate(async (u) => {
      const o = { worker: typeof Worker === 'function', decompression_stream: typeof DecompressionStream === 'function', offscreen_canvas: typeof OffscreenCanvas === 'function' };
      try { const x = await fetch(u, { headers: { Range: 'bytes=0-15' } }); o.range_status = x.status; o.range_bytes = (await x.arrayBuffer()).byteLength; } catch (e) { o.range_error = String(e); }
      try { const b = new Blob(['self.postMessage(42)'], { type: 'text/javascript' }); const w = new Worker(URL.createObjectURL(b)); o.worker_runs = await new Promise(ok => { w.onmessage = e => ok(e.data === 42); setTimeout(() => ok(false), 2000); }); } catch (e) { o.worker_runs = false; }
      const f = []; let last = 0; await new Promise(ok => { const step = t => { if (last) f.push(t - last); last = t; if (f.length < 120) requestAnimationFrame(step); else ok(); }; requestAnimationFrame(step); });
      o.empty_frames = f; return o;
    }, url));
    r.empty_frames = frameStats(r.empty_frames);
    res[name] = r; await browser.close();
  } catch (e) { res[name] = { error: String(e).slice(0, 400) }; }
}
fs.writeFileSync(out, JSON.stringify(res, null, 1)); console.log(JSON.stringify(res, null, 1));
