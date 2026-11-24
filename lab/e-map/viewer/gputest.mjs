import { chromium } from 'playwright';
const configs = {
  'angle-vulkan': ['--use-angle=vulkan', '--enable-features=Vulkan,VulkanFromANGLE,DefaultANGLEVulkan', '--ignore-gpu-blocklist', '--enable-gpu', '--disable-vulkan-surface'],
  'angle-vulkan2': ['--use-angle=vulkan', '--enable-features=Vulkan', '--ignore-gpu-blocklist', '--enable-gpu'],
  'angle-gl-egl': ['--use-gl=angle', '--use-angle=gl-egl', '--ignore-gpu-blocklist', '--enable-gpu'],
  'default': [],
};
for (const [name, args] of Object.entries(configs)) {
  try {
    const browser = await chromium.launch({ headless: true, channel: 'chromium', args });
    const page = await browser.newPage({ viewport: { width: 800, height: 500 } });
    await page.goto('http://127.0.0.1:8765/viewer/index.html?file=../out/lj.pmtiles&z=1.0');
    await page.waitForFunction(() => window.__gm && window.__gm.pm.root && window.__gm.idle(), null, { timeout: 20000 });
    await page.waitForTimeout(500);
    const r = await page.evaluate(() => window.__gm.renderer);
    const buf = await page.screenshot({ path: `/home/tux/graphlab/exp/e-map/tmp/gpu-${name}.png` });
    const px = await page.evaluate(() => { const c = document.getElementById('gl'); const g = c.getContext('webgl2'); const p = new Uint8Array(4); g.readPixels(5, 5, 1, 1, g.RGBA, g.UNSIGNED_BYTE, p); return [...p, g.getError()]; });
    console.log(name, '|', r, '| png bytes', buf.length, '| readPixels', px);
    await browser.close();
  } catch (e) { console.log(name, 'FAILED', String(e).slice(0, 200)); }
}
