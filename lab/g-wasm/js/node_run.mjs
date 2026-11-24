// node js/node_run.mjs <variant: st|st-simd|w64|w64-simd> <mode: csr|text|stream|gen> <arg...>
//   csr <dir>   text <file>   stream <file>   gen <n> <m>
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as H from '../web/harness.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const [variant, mode, a1, a2] = process.argv.slice(2);
const wasmPath = path.join(here, '..', 'web', `pkg-${variant}`, 'gwasm.wasm');
const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {});
const E = instance.exports;
const W = H.wrap(E, E.memory, variant.startsWith('w64'));
let r;
try {
  if (mode === 'csr') {
    const off = fs.readFileSync(path.join(a1, 'offsets.u64'));
    const tgt = fs.readFileSync(path.join(a1, 'targets.u32'));
    r = await H.runCsr(W, off, tgt);
  } else if (mode === 'text') {
    r = await H.runText(W, fs.readFileSync(a1));
  } else if (mode === 'stream') {
    const st = fs.createReadStream(a1, { highWaterMark: 4 << 20 });
    r = await H.runTextStream(W, st, fs.statSync(a1).size);
  } else if (mode === 'stream2') {
    r = await H.runTextTwoPass(W, () => fs.createReadStream(a1, { highWaterMark: 4 << 20 }));
  } else if (mode === 'gen2') {
    r = await H.runGen2(W, Number(a1), Number(a2));
  } else if (mode === 'gen') {
    r = await H.runGen(W, Number(a1), Number(a2));
  }
  r.ok = true;
} catch (e) {
  r = { mode, ok: false, error: String(e), wasm_mem_bytes: W.memBytes() };
}
Object.assign(r, { runtime: `node ${process.version}`, variant, threads: E.ga_threads(), input: a1 ?? null });
console.log(JSON.stringify(r));
