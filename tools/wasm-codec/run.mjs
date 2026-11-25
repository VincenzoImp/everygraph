// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// The wasm32 side of the wasm codec check: loads the module built from tools/wasm-codec
// (with simd128) and prints the same line as the native binary.
//
// Usage: node tools/wasm-codec/run.mjs MODULE.wasm SEED GRAPHS NODES

import { readFile } from "node:fs/promises";

const [path, seed, graphs, nodes] = process.argv.slice(2);
if (nodes === undefined) {
  console.error("usage: node run.mjs MODULE.wasm SEED GRAPHS NODES");
  process.exit(2);
}
// The module imports nothing: no WASI, no bindings generator.
const { instance } = await WebAssembly.instantiate(await readFile(path), {});
const args = [BigInt(seed), Number(graphs), Number(nodes)];
const hash = BigInt.asUintN(64, instance.exports.everygraph_codec_hash(...args));
const counts = BigInt.asUintN(64, instance.exports.everygraph_codec_counts(...args));
const decoders = counts >> 32n;
const mismatches = counts & 0xffffffffn;
console.log(
  `codec seed=${seed} graphs=${graphs} nodes=${nodes} decoders=${decoders} mismatches=${mismatches} hash=${hash.toString(16).padStart(16, "0")}`,
);
