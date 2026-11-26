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

The allocation rule of the kernel crates (issue #31: no buffer sized outside the
memory budget, through the `clippy.toml` of each kernel crate) is checked the same
way, by the probes under tools/canary/allocation/, which name the crates they go in.

Before any probe, the same checks run on the unmodified copy and must pass: otherwise
a broken toolchain would make every probe "fail" and the canary would prove nothing.

A probe file declares, in `//` comments at its top:

    // check: clippy | script | wasm32
    // expect: <text that must appear in the check's output>      (one or more)
    // dependency: <a line for [dependencies] of each crate probed>  (optional)
    // crates: <package> ...      (optional; default everygraph-core; clippy only)

The checks are the commands of CONTRIBUTING.md, "Checks":

    clippy  cargo clippy -p everygraph-core --all-targets -- -D warnings
            (-p each crate the probe names)
    script  python3 tools/check_core_portability.py
    wasm32  cargo build -p everygraph-core --target wasm32-unknown-unknown
                --no-default-features

Usage: python3 tools/check_portability_canary.py [PROBE ...]
       (from anywhere; PROBE is a file name under tools/canary/portability or
       tools/canary/allocation without `.rs`; all probes by default). Needs cargo with clippy and the
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
PROBE_DIRS = [ROOT / "tools" / "canary" / "portability", ROOT / "tools" / "canary" / "allocation"]
CORE = "everygraph-core"
SCRATCH = ROOT / "target" / "portability-canary"
COPY = SCRATCH / "workspace"
# What the scratch workspace needs: the manifest, the lock file, the lint lists of the
# crates without their own, the crates, the portability script, and the oracle's
# tolerances (compiled into everygraph-oracle, a dev-dependency of several crates).
# Build output under any `target` directory is not copied.
COPIED = [
    "Cargo.toml", "Cargo.lock", "clippy.toml", "crates", "tools/check_core_portability.py",
    "oracle/tolerances.json",
]

CHECKS = {
    "clippy": [
        "cargo", "clippy", "--package", "{crate}", "--all-targets",
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
    """Returns (check, expected messages, dependency or None, crates) declared by a probe."""
    check, expected, dependency, crates = None, [], None, [CORE]
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
        elif key == "crates":
            crates = value.split()
    if check not in CHECKS:
        raise SystemExit(f"{path.relative_to(ROOT)}: `// check:` must be one of {sorted(CHECKS)}")
    if not expected:
        raise SystemExit(f"{path.relative_to(ROOT)}: needs at least one `// expect:` line")
    if crates != [CORE] and check != "clippy":
        raise SystemExit(f"{path.relative_to(ROOT)}: `// crates:` goes with `check: clippy` only")
    return check, expected, dependency, crates


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


def apply_probe(path, dependency, crate):
    """Adds the probe to `crate` in COPY."""
    core = COPY / "crates" / crate
    shutil.copy2(path, core / "src" / f"{PROBE_MODULE}.rs")
    # A crate with only a binary (everygraph-cli) takes the probe in its main.rs.
    root = core / "src" / "lib.rs"
    if not root.exists():
        root = core / "src" / "main.rs"
    with open(root, "a", encoding="utf-8") as lib:
        lib.write(f"\npub mod {PROBE_MODULE};\n")
    if dependency:
        manifest = core / "Cargo.toml"
        text = manifest.read_text(encoding="utf-8")
        if "\n[dependencies]\n" not in text:
            text += "\n[dependencies]\n"
        name = dependency.split("=")[0].strip()
        # A crate that already depends on it (the core on libm) keeps its own line.
        if f"\n{name} = " not in text:
            text = text.replace("\n[dependencies]\n", f"\n[dependencies]\n{dependency}\n", 1)
        manifest.write_text(text, encoding="utf-8")


def run(check, crate=CORE):
    """Runs a check (on `crate`, for clippy) in COPY; returns (exit status, output)."""
    command = [part.replace("{crate}", crate) for part in CHECKS[check]]
    result = subprocess.run(
        command, cwd=COPY, capture_output=True, text=True,
        # No colour: escape codes would split the messages the probes expect.
        env={**os.environ, "CARGO_TARGET_DIR": str(COPY / "target"), "CARGO_TERM_COLOR": "never"},
    )
    return result.returncode, result.stdout + result.stderr


def main(names):
    probes = sorted(p for directory in PROBE_DIRS for p in directory.glob("*.rs"))
    if names:
        probes = [p for p in probes if p.stem in names]
        missing = set(names) - {p.stem for p in probes}
        if missing:
            raise SystemExit(f"no such probe: {', '.join(sorted(missing))}")
    declared = [(path, *read_probe(path)) for path in probes]

    failed = False
    fresh_copy()
    baselines = sorted({(check, crate) for _, check, _, _, crates in declared for crate in crates})
    for check, crate in baselines:
        status, output = run(check, crate)
        if status == 0:
            print(f"ok: baseline `{check}` passes on the unmodified {crate}")
        else:
            failed = True
            print(f"FAILED: baseline `{check}` fails on the unmodified {crate} (exit status {status})")
            print("  " + output.strip().replace("\n", "\n  "))
    if failed:
        print("The probes were not run: fix the baseline first.")
        return 1

    runs = [
        (path, check, expected, dependency, crate)
        for path, check, expected, dependency, crates in declared
        for crate in crates
    ]
    for path, check, expected, dependency, crate in runs:
        fresh_copy()
        apply_probe(path, dependency, crate)
        status, output = run(check, crate)
        missing = [text for text in expected if text not in output]
        where = "" if crate == CORE else f" in {crate}"
        if status != 0 and not missing:
            print(f"ok: probe `{path.stem}`{where} is rejected by `{check}`")
            continue
        failed = True
        if status == 0:
            print(f"FAILED: probe `{path.stem}`{where} is ACCEPTED by `{check}` (exit status 0)")
        else:
            print(f"FAILED: probe `{path.stem}`{where} fails `{check}`, but not for the expected reason")
        for text in missing:
            print(f"  missing from the output: {text}")
        print("  " + output.strip().replace("\n", "\n  "))
    fresh_copy()
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
