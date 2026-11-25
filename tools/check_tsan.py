#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""ThreadSanitizer on the parallel code of everygraph-core, with a canary.

Two runs, both on the pinned nightly toolchain with `-Zsanitizer=thread` and a
standard library rebuilt with the sanitizer (`-Zbuild-std`; without it every
synchronization inside std is invisible to ThreadSanitizer and it reports false
races):

1. The tests of everygraph-core with default features, so that `everygraph_core::par`
   runs on rayon's thread pool. They must pass with no report.
2. The canary under tools/canary/tsan: a toy kernel on the same facade with a
   deliberate data race. It must FAIL, with "ThreadSanitizer: data race" in its
   output. If it passes, ThreadSanitizer is not actually watching (wrong flags, a
   toolchain change, one thread only) and run 1 proves nothing.

Kernels with parallel state join run 1 as they arrive: their tests run here unchanged.

Usage: python3 tools/check_tsan.py
       (from anywhere; Linux x86_64 only). The toolchain is the one named in
       tools/nightly-toolchain, or $EVERYGRAPH_NIGHTLY if set; it needs the
       `rust-src` component.
Exit status 0 if both runs behave as required, 1 otherwise.
"""

import os
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TARGET = "x86_64-unknown-linux-gnu"
TOOLCHAIN = os.environ.get("EVERYGRAPH_NIGHTLY") or (
    (ROOT / "tools" / "nightly-toolchain").read_text(encoding="utf-8").strip()
)
ENV = {
    **os.environ,
    "RUSTFLAGS": "-Zsanitizer=thread",
    "CARGO_TARGET_DIR": str(ROOT / "target" / "tsan"),
    # No colour: escape codes would split the messages searched for below.
    "CARGO_TERM_COLOR": "never",
    # More than one worker even on a one-core runner, so that chunks really overlap.
    "RAYON_NUM_THREADS": "4",
    # Report every race, not only the first; the exit status is 66 when there was one.
    "TSAN_OPTIONS": "halt_on_error=0",
}
COMMON = ["-Zbuild-std", "--target", TARGET, "--locked"]

TESTS = ["cargo", f"+{TOOLCHAIN}", "test", "--package", "everygraph-core", "--tests", *COMMON]
CANARY = [
    "cargo", f"+{TOOLCHAIN}", "run", "--manifest-path",
    str(ROOT / "tools" / "canary" / "tsan" / "Cargo.toml"), *COMMON,
]
RACE = "WARNING: ThreadSanitizer: data race"


def run(command):
    print(f"$ RUSTFLAGS={ENV['RUSTFLAGS']} RAYON_NUM_THREADS={ENV['RAYON_NUM_THREADS']} "
          + " ".join(command), flush=True)
    result = subprocess.run(command, cwd=ROOT, env=ENV, capture_output=True, text=True)
    return result.returncode, result.stdout + result.stderr


def main():
    failed = False

    status, output = run(TESTS)
    races = output.count(RACE)
    if status == 0 and races == 0:
        print("ok: the tests of everygraph-core pass under ThreadSanitizer, no race reported")
    else:
        failed = True
        print(f"FAILED: the tests of everygraph-core under ThreadSanitizer "
              f"(exit status {status}, {races} race reports)")
        print("  " + output.strip().replace("\n", "\n  "))

    status, output = run(CANARY)
    races = output.count(RACE)
    if status != 0 and races > 0:
        print(f"ok: the racy canary is caught (exit status {status}, {races} race reports)")
        first = output.index(RACE)
        report = output[first:].split("\n\n")[0].splitlines()
        print("  first report, first frames:")
        print("\n".join("    " + line.split(" (everygraph-tsan-canary+")[0] for line in report[:4]))
    else:
        failed = True
        print(f"FAILED: the racy canary was not caught (exit status {status}, "
              f"{races} race reports): ThreadSanitizer is not watching")
        print("  " + output.strip().replace("\n", "\n  "))

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
