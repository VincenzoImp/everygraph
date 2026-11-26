// m1-12 (#26): the browser configurations measured, shared by the measurement scripts.
// Frame rate is left at the display's rate: without it, headless Chromium runs requestAnimationFrame ahead of the
// GPU and a software renderer's frame intervals look short (seen in the first run). The cost of one frame is timed
// in the page instead (lab/e-map/viewer renderCost: draw, then readPixels).
import { chromium, firefox } from 'playwright';

export const CONFIGS = {
  // the RTX 4090 through ANGLE on EGL (lab/e-map/REPORT.md: the only headless path that reached the GPU)
  'chromium-gpu': () => chromium.launch({ args: ['--use-gl=angle', '--use-angle=gl-egl', '--ignore-gpu-blocklist', '--enable-gpu'] }),
  // SwiftShader: WebGL2 rasterized on the CPU, the fallback Chromium uses when no GPU is usable
  'chromium-swiftshader': () => chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] }),
  // Firefox headless: on this host it cannot create WebGL at all (results/firefox_webgl.jsonl)
  'firefox': () => firefox.launch({ firefoxUserPrefs: { 'webgl.force-enabled': true } }),
};

export async function open(name, viewport = { width: 1600, height: 1000 }) {
  const browser = await CONFIGS[name]();
  const page = await browser.newPage({ viewport });
  const errors = [];
  page.on('pageerror', e => errors.push(String(e)));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  return { browser, page, errors, version: browser.version() };
}

export function frameStats(a) {
  if (!a.length) return null;
  const s = [...a].sort((x, y) => x - y), q = p => +s[Math.min(s.length - 1, Math.floor(s.length * p))].toFixed(2);
  return { n: a.length, mean_ms: +(a.reduce((x, y) => x + y, 0) / a.length).toFixed(2), p50_ms: q(0.5), p95_ms: q(0.95), max_ms: +s[s.length - 1].toFixed(2), over_33ms: a.filter(x => x > 33.4).length };
}

export async function renderer(page) {
  return page.evaluate(() => {
    const gl = document.createElement('canvas').getContext('webgl2');
    if (!gl) return 'no WebGL2';
    const d = gl.getExtension('WEBGL_debug_renderer_info');
    return (d ? gl.getParameter(d.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER)) + ' | ' + gl.getParameter(gl.VERSION);
  });
}
