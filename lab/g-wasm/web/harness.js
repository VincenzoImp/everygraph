// Shared benchmark logic for Node and browser (ES module, no deps).
// E = wasm exports, mem = WebAssembly.Memory, is64 = memory64 build.
const now = () => performance.now();

export function wrap(E, mem, is64) {
  const P = is64 ? (x) => BigInt(x) : (x) => x;
  const N = (x) => Number(x);
  const u8 = (ptr, len) => new Uint8Array(mem.buffer, ptr, len);
  return {
    E, mem, is64, P, N, u8,
    alloc(bytes) { const p = N(E.ga_alloc(P(bytes))); if (!p) throw new Error(`ga_alloc(${bytes}) failed`); return p; },
    allocTyped(count, width) { const p = N(E.ga_alloc_typed(P(count), width)); if (!p) throw new Error(`alloc_typed ${count}x${width} failed`); return p; },
    memBytes() { return E.ga_mem_bytes(); },
  };
}

async function analyze(W, r, iters = 20) {
  let t = now();
  r.pr_iters = iters;
  r.pr_max = W.E.ga_pagerank(iters);
  r.pr_s = (now() - t) / 1000;
  r.pr_sum = W.E.ga_pr_sum();
  t = now();
  r.cc_count = W.E.ga_cc();
  r.cc_s = (now() - t) / 1000;
  r.cc_largest = W.E.ga_cc_largest();
  r.entries = W.E.ga_entries();
  r.wasm_mem_bytes = W.memBytes();
  return r;
}

// CSR mode: offsets (Uint8Array of u64 LE) + targets (Uint8Array of u32 LE).
export async function runCsr(W, offBytes, tgtBytes) {
  const r = { mode: 'csr' };
  let t = now();
  const n1 = offBytes.byteLength / 8, m = tgtBytes.byteLength / 4;
  const po = W.allocTyped(n1, 8);
  W.u8(po, offBytes.byteLength).set(offBytes);
  const pt = W.allocTyped(m, 4);
  W.u8(pt, tgtBytes.byteLength).set(tgtBytes);
  W.E.ga_load_csr(W.P(po), W.P(n1), W.P(pt), W.P(m));
  r.copy_in_s = (now() - t) / 1000;
  r.n = n1 - 1;
  return analyze(W, r);
}

// Text mode, whole buffer: copy into wasm, parse, build CSR.
export async function runText(W, text) {
  const r = { mode: 'text', text_bytes: text.byteLength };
  let t = now();
  const p = W.alloc(text.byteLength);
  W.u8(p, text.byteLength).set(text);
  r.copy_in_s = (now() - t) / 1000;
  t = now();
  r.edges_in = W.E.ga_parse_owned(W.P(p), W.P(text.byteLength));
  r.parse_s = (now() - t) / 1000;
  t = now();
  r.n = W.E.ga_build_csr();
  r.csr_s = (now() - t) / 1000;
  return analyze(W, r);
}

// Text mode, streaming: `reader` is an async iterator of Uint8Array chunks
// (fetch body / File.stream()). Chunks are copied into a reusable staging buffer.
export async function runTextStream(W, chunks, sizeHint, stageBytes = 16 << 20) {
  const r = { mode: 'text-stream', text_bytes: 0 };
  let t = now();
  W.E.ga_stream_begin(Math.ceil(sizeHint / 13)); // ~13 B/line for SNAP lj, grows if wrong
  const stage = W.alloc(stageBytes);
  let fill = 0;
  let pushT = 0;
  const flush = () => { const t0 = now(); W.E.ga_stream_push(W.P(stage), W.P(fill)); pushT += now() - t0; fill = 0; };
  for await (let c of chunks) {
    r.text_bytes += c.byteLength;
    while (c.byteLength) {
      const k = Math.min(c.byteLength, stageBytes - fill);
      W.u8(stage + fill, k).set(c.subarray(0, k));
      fill += k; c = c.subarray(k);
      if (fill === stageBytes) flush();
    }
  }
  if (fill) flush();
  r.edges_in = W.E.ga_stream_end();
  r.ingest_s = (now() - t) / 1000;      // read + copy + parse, overlapped as the stream delivers
  r.parse_in_wasm_s = pushT / 1000;
  t = now();
  r.n = W.E.ga_build_csr();
  r.csr_s = (now() - t) / 1000;
  return analyze(W, r);
}

// Two-pass streaming CSR build: `open()` returns a fresh async iterator of chunks
// each time (File.stream() / fetch / fs stream). Input never resident in wasm.
export async function runTextTwoPass(W, open, stageBytes = 16 << 20) {
  const r = { mode: 'text-2pass', text_bytes: 0 };
  const t0 = now();
  W.E.ga_tp_begin();
  const stage = W.alloc(stageBytes);
  for (let pass = 1; pass <= 2; pass++) {
    let fill = 0;
    const flush = () => { W.E.ga_tp_push(W.P(stage), W.P(fill)); fill = 0; };
    for await (let c of open()) {
      if (pass === 1) r.text_bytes += c.byteLength;
      while (c.byteLength) {
        const k = Math.min(c.byteLength, stageBytes - fill);
        W.u8(stage + fill, k).set(c.subarray(0, k));
        fill += k; c = c.subarray(k);
        if (fill === stageBytes) flush();
      }
    }
    if (fill) flush();
    if (pass === 1) { r.edges_in = W.E.ga_tp_next_pass(); r.pass1_s = (now() - t0) / 1000; }
  }
  r.n = W.E.ga_tp_finish();
  r.ingest_two_pass_s = (now() - t0) / 1000;
  r.mem_after_csr = W.memBytes();
  return analyze(W, r);
}

export async function runGen2(W, n, m, iters = 1) {
  const r = { mode: 'gen2', n_ids: n, m };
  const t = now();
  r.n = W.E.ga_gen2(n, m, 7);
  r.gen_csr_s = (now() - t) / 1000;
  r.mem_after_csr = W.memBytes();
  return analyze(W, r, iters);
}

// Synthetic: m edges over n ids, generated inside wasm (memory-ceiling probe).
export async function runGen(W, n, m, iters = 1) {
  const r = { mode: 'gen', n_ids: n, m };
  let t = now();
  W.E.ga_gen(n, m, 7);
  r.gen_s = (now() - t) / 1000;
  r.mem_after_gen = W.memBytes();
  t = now();
  r.n = W.E.ga_build_csr();
  r.csr_s = (now() - t) / 1000;
  r.mem_after_csr = W.memBytes();
  return analyze(W, r, iters);
}
