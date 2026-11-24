// node js/probe_run.mjs <file-for-File-test>  -> JSON: WebGPU, OPFS, File read throughput
import { chromium } from 'playwright';
import { serve } from './serve.mjs';
const srv = await serve(0);
const base = `http://127.0.0.1:${srv.address().port}/`;
const out = { webgpu: [] };
const gpuProbe = async (page) => page.evaluate(async () => {
  if (!navigator.gpu) return { navigator_gpu: false };
  const res = { navigator_gpu: true };
  try {
    const a = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
    if (!a) return { ...res, adapter: null };
    const i = a.info || {};
    res.adapter = { vendor: i.vendor, architecture: i.architecture, device: i.device, description: i.description, isFallback: a.isFallbackAdapter };
    res.limits = { maxBufferSize: a.limits.maxBufferSize, maxStorageBufferBindingSize: a.limits.maxStorageBufferBindingSize, maxComputeWorkgroupsPerDimension: a.limits.maxComputeWorkgroupsPerDimension };
    res.features = [...a.features];
    const d = await a.requestDevice({ requiredLimits: { maxBufferSize: a.limits.maxBufferSize, maxStorageBufferBindingSize: a.limits.maxStorageBufferBindingSize } });
    // tiny compute sanity check: y[i] = x[i]*2 over 16M floats
    const N = 1 << 24;
    const mod = d.createShaderModule({ code: `@group(0) @binding(0) var<storage, read_write> x: array<f32>;
      @compute @workgroup_size(256) fn main(@builtin(global_invocation_id) g: vec3u) { let i = g.x + g.y * 65536u; if (i < arrayLength(&x)) { x[i] = x[i] * 2.0 + 1.0; } }` });
    const pipe = d.createComputePipeline({ layout: 'auto', compute: { module: mod, entryPoint: 'main' } });
    const buf = d.createBuffer({ size: N * 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST });
    const rb = d.createBuffer({ size: 16, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST });
    const bg = d.createBindGroup({ layout: pipe.getBindGroupLayout(0), entries: [{ binding: 0, resource: { buffer: buf } }] });
    const run = async (reps) => { const e = d.createCommandEncoder(); const p = e.beginComputePass(); p.setPipeline(pipe); p.setBindGroup(0, bg);
      for (let k = 0; k < reps; k++) p.dispatchWorkgroups(256, N / 65536, 1); p.end(); e.copyBufferToBuffer(buf, 0, rb, 0, 16); d.queue.submit([e.finish()]); await rb.mapAsync(GPUMapMode.READ); const v = new Float32Array(rb.getMappedRange().slice(0)); rb.unmap(); return v; };
    await run(1);
    const t = performance.now(); const v = await run(100);
    const dt = (performance.now() - t) / 1000;
    res.compute_GBps = (100 * N * 8) / dt / 1e9; res.check_x0_after_101_passes = v[0]; res.check_ok = v[0] === 2 ** 101 - 1 || v[0] > 1e30;
  } catch (e) { res.error = String(e); }
  return res;
});
for (const [label, opts] of [
  ['headless-shell default', { headless: true }],
  ['headless-shell --enable-unsafe-webgpu --enable-features=Vulkan', { headless: true, args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan,VulkanFromANGLE', '--use-angle=vulkan'] }],
  ['chromium new-headless --enable-unsafe-webgpu --enable-features=Vulkan', { headless: true, channel: 'chromium', args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan,VulkanFromANGLE', '--use-angle=vulkan'] }],
]) {
  let r;
  try {
    const b = await chromium.launch(opts);
    const p = await b.newPage(); await p.goto(base); await p.waitForFunction(() => window.ready);
    r = await Promise.race([gpuProbe(p), new Promise((ok) => setTimeout(() => ok({ timeout: true }), 60000))]);
    r.ua = await p.evaluate(() => navigator.userAgent);
    await b.close();
  } catch (e) { r = { error: String(e) }; }
  out.webgpu.push({ label, ...r });
}
const opfsRun = (p, totalMB) => p.evaluate((totalMB) => new Promise((ok) => { const w = new Worker('./opfs_worker.js'); w.onmessage = (e) => ok(e.data); w.postMessage({ totalMB, chunkMB: 16 }); }), totalMB);
// (a) default Playwright context = ephemeral/incognito-like profile (storage kept in memory)
{ const b0 = await chromium.launch({ headless: true }); const p0 = await b0.newPage(); await p0.goto(base); await p0.waitForFunction(() => window.ready);
  out.opfs_ephemeral_256MB = await opfsRun(p0, 256); out.opfs_ephemeral_2GB = await opfsRun(p0, 2048); await b0.close(); }
// (b) persistent on-disk profile (what a real user has)
import fs from 'node:fs';
const prof = new URL('../tools/chrome-profile-probe', import.meta.url).pathname;
fs.rmSync(prof, { recursive: true, force: true });
const ctx = await chromium.launchPersistentContext(prof, { headless: true });
const p = await ctx.newPage(); await p.goto(base); await p.waitForFunction(() => window.ready);
out.info = await p.evaluate(() => window.info());
out.opfs_persistent_2GB = await opfsRun(p, 2048);
const b = ctx;
if (process.argv[2]) {
  await p.setInputFiles('#f', process.argv[2]);
  out.file = await p.evaluate(() => new Promise((ok) => { const w = new Worker('./file_worker.js'); w.onmessage = (e) => ok(e.data); w.postMessage({ file: window.fileInput() }); }));
}
await b.close(); srv.close(); fs.rmSync(prof, { recursive: true, force: true });
console.log(JSON.stringify(out, null, 1));
