// m1-12 (#26): can headless Firefox on this host create a WebGL2 context at all? Tries the prefs and environment
// variables that select a software or EGL path, and prints the renderer or the creation error.
import { firefox } from 'playwright';

const variants = [
  [{}, {}],
  [{ 'webgl.out-of-process': false }, {}],
  [{ 'webgl.out-of-process': false, 'gfx.webrender.software': true }, { LIBGL_ALWAYS_SOFTWARE: '1' }],
  [{ 'webgl.out-of-process': false }, { LIBGL_ALWAYS_SOFTWARE: '1', MOZ_X11_EGL: '1', EGL_PLATFORM: 'surfaceless' }],
];
for (const [prefs, env] of variants) {
  const b = await firefox.launch({ firefoxUserPrefs: { 'webgl.force-enabled': true, ...prefs }, env: { ...process.env, ...env } });
  const p = await b.newPage();
  await p.setContent('<canvas id=c></canvas>');
  const r = await p.evaluate(() => {
    const c = document.getElementById('c'); let why = '';
    c.addEventListener('webglcontextcreationerror', e => { why = e.statusMessage; });
    const gl = c.getContext('webgl2'); if (!gl) return 'no context: ' + why;
    const d = gl.getExtension('WEBGL_debug_renderer_info');
    return d ? gl.getParameter(d.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
  });
  console.log(JSON.stringify({ prefs, env, result: r }));
  await b.close();
}
