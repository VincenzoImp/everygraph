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
    let digests = everygraph_wasm_toy::golden::digests();
    for (name, digest) in digests {
        println!("golden {name}={digest:016x}");
    }
    if digests != everygraph_wasm_toy::golden::EXPECTED {
        eprintln!("the golden digests differ from the frozen ones in numerics_golden.rs");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
