// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The native side of the wasm toy check: prints the same lines as `run.mjs`.
//!
//! Usage: everygraph-wasm-toy NODES EDGES SEED

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args.as_slice() {
        [nodes, edges, seed] => nodes
            .parse()
            .ok()
            .zip(edges.parse().ok())
            .zip(seed.parse().ok()),
        _ => None,
    };
    let Some(((nodes, edges), seed)) = parsed else {
        eprintln!("usage: everygraph-wasm-toy NODES EDGES SEED");
        return ExitCode::from(2);
    };
    let hash = everygraph_wasm_toy::degree_hash(nodes, edges, seed);
    println!("degree-hash nodes={nodes} edges={edges} seed={seed} hash={hash:016x}");
    let hash = everygraph_wasm_toy::kernels_hash(nodes, edges, seed);
    println!("kernels-hash nodes={nodes} edges={edges} seed={seed} hash={hash:016x}");
    ExitCode::SUCCESS
}
