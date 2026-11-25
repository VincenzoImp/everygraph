// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The in-process references reproduce the golden files of `oracle/golden/`. The vendored
//! graph (Zachary's karate club) is always checked; downloaded ones when they are in the
//! cache (`oracle-ref golden --fetch`).

#![forbid(unsafe_code)]

use std::path::Path;

use everygraph_oracle::golden::check_dir;

#[test]
fn references_reproduce_the_golden_files() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/golden");
    let results = check_dir(&dir).expect("golden directory");
    let karate = results
        .iter()
        .find(|r| r.name == "karate")
        .expect("karate golden file");
    assert!(
        matches!(karate.outcome, Ok(n) if n >= 10),
        "{:?}",
        karate.outcome
    );
    for r in &results {
        if let Err(e) = &r.outcome {
            assert!(e.starts_with("skipped"), "{}: {e}", r.name);
        }
    }
}
