// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The native side of the wasm codec check: prints the same line as `run.mjs`.
//!
//! Usage: everygraph-wasm-codec SEED GRAPHS NODES

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args.as_slice() {
        [seed, graphs, nodes] => seed
            .parse()
            .ok()
            .zip(graphs.parse().ok())
            .zip(nodes.parse().ok()),
        _ => None,
    };
    let Some(((seed, graphs), nodes)) = parsed else {
        eprintln!("usage: everygraph-wasm-codec SEED GRAPHS NODES");
        return ExitCode::from(2);
    };
    let r = everygraph_wasm_codec::check(seed, graphs, nodes);
    println!(
        "codec seed={seed} graphs={graphs} nodes={nodes} decoders={} mismatches={} hash={:016x}",
        r.decoders, r.mismatches, r.hash
    );
    ExitCode::SUCCESS
}
