// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The binary is called `everygraph` (not `everygraph-cli`, not `eg`), and until it can
//! do something it must not look as if it had succeeded.

#![forbid(unsafe_code)]

use std::process::Command;

#[test]
fn the_everygraph_binary_exists_and_reports_that_it_does_nothing_yet() {
    let output = Command::new(env!("CARGO_BIN_EXE_everygraph"))
        .output()
        .expect("the everygraph binary runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("the message is UTF-8");
    assert!(
        stderr.starts_with("everygraph 0.0.0: no commands yet"),
        "{stderr}"
    );
}
