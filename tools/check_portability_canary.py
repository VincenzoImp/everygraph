#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""The portability canary: checks that the portability checks still catch what they
are meant to catch.

The portability rule of everygraph-core (CONTRIBUTING.md, "The portability rule") is
enforced by three pieces: the lint lists in crates/everygraph-core/clippy.toml, the
script tools/check_core_portability.py, and the build for wasm32-unknown-unknown. Each
can rot without anyone noticing: a list entry deleted, `clippy.toml` moved so that
clippy no longer finds it, the `forbid` line weakened, an allow-list widened. A check
that has silently stopped checking still passes.

So this script breaks the rule on purpose. Each file under tools/canary/portability/
is a probe: a module that does something the core must not do. For each probe, on a
scratch copy of the workspace, the script adds the module to everygraph-core (and the
dependency the probe names, if any), runs the check the probe names, and requires
that check to FAIL with every message the probe lists. A probe that is accepted, or
that fails for another reason, fails the canary.

Before any probe, the same checks run on the unmodified copy and must pass: otherwise
a broken toolchain would make every probe "fail" and the canary would prove nothing.

A probe file declares, in `//` comments at its top:

    // check: clippy | script | wasm32
    // expect: <text that must appear in the check's output>      (one or more)
    // dependency: <a line for [dependencies] of everygraph-core>  (optional)

The checks are the commands of CONTRIBUTING.md, "Checks":

    clippy  cargo clippy -p everygraph-core --all-targets -- -D warnings
    script  python3 tools/check_core_portability.py
    wasm32  cargo build -p everygraph-core --target wasm32-unknown-unknown
                --no-default-features

Usage: python3 tools/check_portability_canary.py [PROBE ...]
       (from anywhere; PROBE is a file name under tools/canary/portability without
       `.rs`; all probes by default). Needs cargo with clippy and the
       wasm32-unknown-unknown target, and network access for the probe that adds a
       dependency (or a cargo registry cache that has it).
Exit status 0 if every probe is caught, 1 otherwise.

The scratch copy lives in target/portability-canary/ (ignored by git, reused between
runs so that the dependencies are not rebuilt) and the repository itself is never
modified.
"""

import os
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
PROBES = ROOT / "tools" / "canary" / "portability"
SCRATCH = ROOT / "target" / "portability-canary"
COPY = SCRATCH / "workspace"
# What the scratch workspace needs: the manifest, the lock file, the crates and the
# portability script. Build output under any `target` directory is not copied.
COPIED = ["Cargo.toml", "Cargo.lock", "crates", "tools/check_core_portability.py"]

CHECKS = {
    "clippy": [
        "cargo", "clippy", "--package", "everygraph-core", "--all-targets",
        "--", "-D", "warnings",
    ],
    "script": [sys.executable, "tools/check_core_portability.py"],
    "wasm32": [
        "cargo", "build", "--package", "everygraph-core",
        "--target", "wasm32-unknown-unknown", "--no-default-features",
    ],
}

PROBE_MODULE = "canary_probe"


def read_probe(path):
    """Returns (check, expected messages, dependency or None) declared by a probe."""
    check, expected, dependency = None, [], None
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.startswith("//"):
            break
        key, _, value = line.lstrip("/ ").partition(":")
        value = value.strip()
        if key == "check":
            check = value
        elif key == "expect":
            expected.append(value)
        elif key == "dependency":
            dependency = value
    if check not in CHECKS:
        raise SystemExit(f"{path.relative_to(ROOT)}: `// check:` must be one of {sorted(CHECKS)}")
    if not expected:
        raise SystemExit(f"{path.relative_to(ROOT)}: needs at least one `// expect:` line")
    return check, expected, dependency


def fresh_copy():
    """Makes COPY an unmodified copy of the workspace sources (keeping COPY/target)."""
    COPY.mkdir(parents=True, exist_ok=True)
    for entry in COPY.iterdir():
        if entry.name == "target":
            continue
        if entry.is_dir():
            shutil.rmtree(entry)
        else:
            entry.unlink()
    ignore = shutil.ignore_patterns("target")
    for name in COPIED:
        source, destination = ROOT / name, COPY / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        if source.is_dir():
            shutil.copytree(source, destination, ignore=ignore)
        else:
            shutil.copy2(source, destination)


def apply_probe(path, dependency):
    """Adds the probe to everygraph-core in COPY."""
    core = COPY / "crates" / "everygraph-core"
    shutil.copy2(path, core / "src" / f"{PROBE_MODULE}.rs")
    with open(core / "src" / "lib.rs", "a", encoding="utf-8") as lib:
        lib.write(f"\npub mod {PROBE_MODULE};\n")
    if dependency:
        manifest = core / "Cargo.toml"
        text = manifest.read_text(encoding="utf-8")
        if "\n[dependencies]\n" not in text:
            raise SystemExit("crates/everygraph-core/Cargo.toml has no [dependencies] table")
        text = text.replace("\n[dependencies]\n", f"\n[dependencies]\n{dependency}\n", 1)
        manifest.write_text(text, encoding="utf-8")


def run(check):
    """Runs a check in COPY; returns (exit status, combined output)."""
    result = subprocess.run(
        CHECKS[check], cwd=COPY, capture_output=True, text=True,
        # No colour: escape codes would split the messages the probes expect.
        env={**os.environ, "CARGO_TARGET_DIR": str(COPY / "target"), "CARGO_TERM_COLOR": "never"},
    )
    return result.returncode, result.stdout + result.stderr


def main(names):
    probes = sorted(PROBES.glob("*.rs"))
    if names:
        probes = [p for p in probes if p.stem in names]
        missing = set(names) - {p.stem for p in probes}
        if missing:
            raise SystemExit(f"no such probe: {', '.join(sorted(missing))}")
    declared = [(path, *read_probe(path)) for path in probes]

    failed = False
    fresh_copy()
    for check in sorted({check for _, check, _, _ in declared}):
        status, output = run(check)
        if status == 0:
            print(f"ok: baseline `{check}` passes on the unmodified core")
        else:
            failed = True
            print(f"FAILED: baseline `{check}` fails on the unmodified core (exit status {status})")
            print("  " + output.strip().replace("\n", "\n  "))
    if failed:
        print("The probes were not run: fix the baseline first.")
        return 1

    for path, check, expected, dependency in declared:
        fresh_copy()
        apply_probe(path, dependency)
        status, output = run(check)
        missing = [text for text in expected if text not in output]
        if status != 0 and not missing:
            print(f"ok: probe `{path.stem}` is rejected by `{check}`")
            continue
        failed = True
        if status == 0:
            print(f"FAILED: probe `{path.stem}` is ACCEPTED by `{check}` (exit status 0)")
        else:
            print(f"FAILED: probe `{path.stem}` fails `{check}`, but not for the expected reason")
        for text in missing:
            print(f"  missing from the output: {text}")
        print("  " + output.strip().replace("\n", "\n  "))
    fresh_copy()
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
