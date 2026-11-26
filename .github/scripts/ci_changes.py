#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Which CI jobs a run needs, from what the pull request changed.

Run by the `lint` job of .github/workflows/ci.yml, which exposes the flags to the
other jobs (CONTRIBUTING.md, "CI"). On a pull request the checkout is GitHub's merge
commit, so `git diff HEAD^1 HEAD` is exactly what merging would change on top of the
current base, with no third-party action. Everything runs (`full`) on a push to main,
by hand, when the run changes CI itself, and when the pull request has the label
`full-ci`.

Inputs (environment): EVENT_NAME (github.event_name), LABELS (a JSON list of the pull
request's label names), ALREADY_CHECKED=1 for a scheduled run of a commit whose push run
already passed (only lint runs then). Writes `name=true|false` lines to $GITHUB_OUTPUT
(standard output when that is unset) and prints the reason for each flag.

Usage, to see what a branch would run: python3 .github/scripts/ci_changes.py --base main
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Prefixes (directories end with "/") that switch a conditional job on.
CORE = [
    "crates/everygraph-core/",
    "tools/nightly-toolchain",
    "tools/check_tsan.py",
    "tools/canary/tsan/",
]
ORACLE = [
    "crates/everygraph-oracle/",
    "oracle/",
    # Kernel crates: what the oracle exists to check.
    "crates/everygraph-kernels/",
    "crates/everygraph-community/",
    "crates/everygraph-map/",
    # oracle/run-ci.sh generates registry datasets through the fetcher, and the golden
    # files are pinned to their bytes.
    "datasets/",
    "crates/everygraph-bench/src/datasets/",
]
# The crates of the fuzz targets (fuzz.yml's 5-minute campaigns), with what they build on.
FUZZ_FORMAT = [
    "crates/everygraph-format/",
    "crates/everygraph-community/",
    "crates/everygraph-kernels/",
    "crates/everygraph-map/",
    "crates/everygraph-core/",
    "tools/fuzz.sh",
]
FUZZ_INGEST = ["crates/everygraph-ingest/", "crates/everygraph-core/", "tools/fuzz.sh"]
FLAGS = ("core", "oracle", "msrv", "fuzz_format", "fuzz_ingest", "full")
# Any change here re-runs everything: a change to CI is checked by a full run.
EVERYTHING = [".github/"]


def manifest(path):
    """Cargo.toml (any crate: rust-version, dependencies, profiles), Cargo.lock, toolchains."""
    name = path.rsplit("/", 1)[-1]
    return name in ("Cargo.toml", "Cargo.lock") or name.startswith("rust-toolchain")


def unsafe_modules():
    """Paths in the Module column of CONTRIBUTING.md's `unsafe` table: a repository path
    (`crates/everygraph-io/tests/x.rs`), or a crate name followed by a path inside it
    (`everygraph-io`, `src/sys.rs`)."""
    text = (ROOT / "CONTRIBUTING.md").read_text(encoding="utf-8")
    section = text.split("## `unsafe`", 1)[-1].split("\n## ", 1)[0]
    paths = []
    for line in section.splitlines():
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if not line.lstrip().startswith("|") or len(cells) < 2:
            continue
        crate = None
        for token in re.findall(r"`([^`]+)`", cells[0]):
            if re.fullmatch(r"everygraph-[a-z0-9-]+", token):
                crate = token
                continue
            if "/" in token:
                if crate and not token.startswith("crates/"):
                    token = f"crates/{crate}/{token}"
                paths.append(token.rstrip("/") + ("/" if not token.endswith(".rs") else ""))
    return paths


def changed_files(base):
    if base:
        merge_base = subprocess.check_output(
            ["git", "merge-base", base, "HEAD"], cwd=ROOT, text=True).strip()
        rev = [merge_base, "HEAD"]
    else:
        # GitHub's merge commit for a pull request: first parent is the base branch.
        rev = ["HEAD^1", "HEAD"]
    out = subprocess.check_output(
        ["git", "diff", "--name-only", *rev], cwd=ROOT, text=True)
    return [line for line in out.splitlines() if line]


def touches(files, prefixes):
    return sorted(f for f in files if any(f == p or f.startswith(p) for p in prefixes))


def main():
    base = None
    if len(sys.argv) == 3 and sys.argv[1] == "--base":
        base = sys.argv[2]
    elif len(sys.argv) != 1:
        sys.exit(__doc__)
    event = os.environ.get("EVENT_NAME", "pull_request" if base else "")
    labels = json.loads(os.environ.get("LABELS") or "[]") or []
    flags = {}
    why = {}
    if event == "schedule" and os.environ.get("ALREADY_CHECKED") == "1":
        for k in FLAGS:
            flags[k], why[k] = False, "scheduled run of a commit whose push run passed"
    elif event != "pull_request":
        for k in FLAGS:
            flags[k], why[k] = True, f"event {event or 'unknown'}: everything runs"
    elif "full-ci" in labels:
        for k in FLAGS:
            flags[k], why[k] = True, "label full-ci"
    else:
        reason = None
        try:
            files = changed_files(base)
            ci = touches(files, EVERYTHING)
            if ci:
                reason = f"CI itself changed: {', '.join(ci[:3])}"
        except subprocess.CalledProcessError as e:
            reason = f"cannot compute the diff ({e}): everything runs"
        if reason:
            for k in FLAGS:
                flags[k], why[k] = True, reason
        else:
            print(f"{len(files)} files changed")
            core = CORE + unsafe_modules()
            groups = {
                "core": touches(files, core),
                "oracle": touches(files, ORACLE),
                "fuzz_format": touches(files, FUZZ_FORMAT),
                "fuzz_ingest": touches(files, FUZZ_INGEST),
                "msrv": sorted(f for f in files if manifest(f)),
            }
            for k, hits in groups.items():
                flags[k] = bool(hits)
                more = f" (+{len(hits) - 3})" if len(hits) > 3 else ""
                why[k] = ", ".join(hits[:3]) + more if hits else "nothing it checks changed"
            flags["full"], why["full"] = False, "pull request"
    lines = [f"{k}={'true' if v else 'false'}" for k, v in flags.items()]
    for k in flags:
        print(f"{k:12} {'yes' if flags[k] else 'no ':3}  {why[k]}")
    target = os.environ.get("GITHUB_OUTPUT")
    if target:
        with open(target, "a", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
    else:
        print("\n".join(lines))


if __name__ == "__main__":
    main()
