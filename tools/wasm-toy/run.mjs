// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// The wasm32 side of the wasm toy check: loads the module built from tools/wasm-toy
// and prints the same lines as the native binary.
//
// Usage: node tools/wasm-toy/run.mjs MODULE.wasm NODES EDGES SEED

import { readFile } from "node:fs/promises";

const [path, nodes, edges, seed] = process.argv.slice(2);
if (seed === undefined) {
  console.error("usage: node run.mjs MODULE.wasm NODES EDGES SEED");
  process.exit(2);
}
// The module imports nothing: no WASI, no bindings generator.
const { instance } = await WebAssembly.instantiate(await readFile(path), {});
const hash = instance.exports.everygraph_toy_degree_hash(
  Number(nodes),
  Number(edges),
  BigInt(seed),
);
// The exports return an `i64`, which JavaScript receives as a signed BigInt.
const hex = (h) => BigInt.asUintN(64, h).toString(16).padStart(16, "0");
console.log(`degree-hash nodes=${nodes} edges=${edges} seed=${seed} hash=${hex(hash)}`);
const kernels = instance.exports.everygraph_toy_kernels_hash(
  Number(nodes),
  Number(edges),
  BigInt(seed),
);
console.log(`kernels-hash nodes=${nodes} edges=${edges} seed=${seed} hash=${hex(kernels)}`);
// The names and order of `golden::digests()` (crates/everygraph-core/tests/numerics_golden.rs).
const GOLDEN = ["sin", "cos", "exp", "ln", "log2", "sqrt", "mul_add", "exact_sum", "rng", "canonical"];
GOLDEN.forEach((name, index) => {
  console.log(`golden ${name}=${hex(instance.exports.everygraph_toy_golden_digest(index))}`);
});
