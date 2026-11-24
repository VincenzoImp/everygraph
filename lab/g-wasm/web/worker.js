import * as H from './harness.js';

async function instantiate(variant, threads) {
  if (variant.startsWith('mt')) {
    const mod = await import(`./pkg-${variant}/gwasm.js`);
    const E = await mod.default();               // wasm-bindgen init; returns raw exports
    await mod.initThreadPool(threads);
    return H.wrap(E, E.memory, false);
  }
  const { instance } = await WebAssembly.instantiateStreaming(fetch(`./pkg-${variant}/gwasm.wasm`), {});
  const E = instance.exports;
  return H.wrap(E, E.memory, variant.startsWith('w64'));
}
async function bytes(url) { return new Uint8Array(await (await fetch(url)).arrayBuffer()); }
async function* streamChunks(rs) { const rd = rs.getReader(); for (;;) { const { done, value } = await rd.read(); if (done) return; yield value; } }

async function* streamChunksP(respP) { const resp = await respP; yield* streamChunks(resp.body); }

onmessage = async (ev) => {
  const m = ev.data;
  let r = {}, W;
  try {
    const t0 = performance.now();
    W = await instantiate(m.variant, m.threads || navigator.hardwareConcurrency);
    const tInit = (performance.now() - t0) / 1000;
    if (m.mode === 'csr') {
      let t = performance.now();
      const off = await bytes(`/data/csr/${m.ds}/offsets.u64`);
      const tgt = await bytes(`/data/csr/${m.ds}/targets.u32`);
      const fetch_s = (performance.now() - t) / 1000;
      r = await H.runCsr(W, off, tgt); r.fetch_s = fetch_s;
    } else if (m.mode === 'text') {
      let t = performance.now();
      const txt = await bytes(`/data/txt/${m.ds}`);
      const fetch_s = (performance.now() - t) / 1000;
      r = await H.runText(W, txt); r.fetch_s = fetch_s;
    } else if (m.mode === 'stream') {
      const resp = await fetch(`/data/txt/${m.ds}`);
      r = await H.runTextStream(W, streamChunks(resp.body), Number(resp.headers.get('Content-Length')));
    } else if (m.mode === 'file') {            // File object from <input type=file> (zero-install path)
      r = await H.runTextStream(W, streamChunks(m.file.stream()), m.file.size); r.source = 'File.stream()';
    } else if (m.mode === 'stream2') {
      r = await H.runTextTwoPass(W, () => streamChunksP(fetch(`/data/txt/${m.ds}`)));
    } else if (m.mode === 'file2') {           // File, read twice (two-pass CSR build)
      r = await H.runTextTwoPass(W, () => streamChunks(m.file.stream())); r.source = 'File.stream() x2';
    } else if (m.mode === 'gen2') {
      r = await H.runGen2(W, m.n, m.m);
    } else if (m.mode === 'gen') {
      r = await H.runGen(W, m.n, m.m);
    }
    r.init_s = tInit; r.ok = true;
  } catch (e) {
    r = { ok: false, error: String(e && e.stack || e), wasm_mem_bytes: W ? W.memBytes() : null };
  }
  Object.assign(r, { variant: m.variant, threads: W ? W.E.ga_threads() : null, ds: m.ds ?? null, hc: navigator.hardwareConcurrency });
  postMessage(r);
};
