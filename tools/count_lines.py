#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Lines of a crate's `src/` per module, as the size targets of #200 count them.

Non-blank lines in four kinds: code; doc comments (`///`, `//!`); other comments; and
in-file tests (a `#[cfg(test)]` item, or a file declared by `#[cfg(test)] mod x;`).
The count is textual: braces inside strings are not special, which no module of the
product crates has around a test module.

    tools/count_lines.py crates/everygraph-ingest [crates/... ...]
"""
import re
import sys
from pathlib import Path


def test_files(src: Path) -> set[Path]:
    """Files that a `#[cfg(test)] mod x;` declares."""
    out = set()
    for path in src.rglob("*.rs"):
        lines = path.read_text().split("\n")
        for i, line in enumerate(lines):
            if not line.strip().startswith("#[cfg(test)]"):
                continue
            for next_line in lines[i + 1 : i + 4]:
                m = re.match(r"\s*(pub(\(crate\))?\s+)?mod\s+(\w+)\s*;", next_line)
                if m:
                    base = path.parent
                    if path.name not in ("mod.rs", "lib.rs", "main.rs"):
                        base = base / path.stem
                    out |= {base / f"{m.group(3)}.rs", base / m.group(3) / "mod.rs"}
                    break
                if not next_line.strip().startswith("#"):
                    break
    return out


def count(path: Path, all_test: bool) -> list[int]:
    """[code, doc, comment, test] of one file."""
    lines = [line.strip() for line in path.read_text().split("\n")]
    if all_test:
        return [0, 0, 0, sum(1 for s in lines if s)]
    n = [0, 0, 0, 0]
    depth = None
    started = False
    for s in lines:
        if depth is not None:
            n[3] += bool(s)
            depth += s.count("{") - s.count("}")
            started = started or "{" in s
            if (started and depth <= 0) or (not started and s.endswith(";")):
                depth = None
        elif s.startswith("#[cfg(test)]"):
            n[3] += 1
            depth, started = 0, False
        elif s.startswith(("///", "//!")):
            n[1] += 1
        elif s.startswith("//"):
            n[2] += 1
        elif s:
            n[0] += 1
    return n


def main() -> None:
    for crate in map(Path, sys.argv[1:]):
        src = crate / "src"
        tests = test_files(src)
        rows = sorted((p.relative_to(src), count(p, p in tests)) for p in src.rglob("*.rs"))
        print(f"{crate.name:32} {'code':>6} {'doc':>6} {'comment':>8} {'test':>6}")
        for name, n in rows:
            print(f"  {str(name):30} {n[0]:6} {n[1]:6} {n[2]:8} {n[3]:6}")
        total = [sum(n[k] for _, n in rows) for k in range(4)]
        print(f"  {'total':30} {total[0]:6} {total[1]:6} {total[2]:8} {total[3]:6}")


if __name__ == "__main__":
    main()
