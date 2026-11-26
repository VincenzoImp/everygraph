#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Portability checks on everygraph-core that clippy and the wasm32 build cannot make.

The lint lists in crates/everygraph-core/clippy.toml only see code that clippy
compiles, and only code of this crate. Four things get past them, and each is
checked here:

1. Dependencies. A crate that opens files or starts threads on its own (walkdir,
   tokio) builds for wasm32 and is invisible to the lints. So the dependencies of
   everygraph-core, direct and transitive, must be in ALLOWED_DEPENDENCIES.
2. Conditional compilation. Code under `#[cfg(not(clippy))]`, or under a target that
   the lint run does not build (`#[cfg(windows)]`), is never linted. So every `cfg`
   in the crate must use only the predicates in ALLOWED_CFG, and CONTRIBUTING.md
   lists a lint run for each configuration those predicates can select.
3. The `forbid` line. The lints cannot be switched off with an inner `#[allow]`
   because the crate root forbids them. So every crate root of everygraph-core
   (lib.rs, and any file under tests/, benches/, examples/ or src/bin/) must
   carry that line.
4. The lists themselves. Clippy reports an entry of clippy.toml that names nothing
   (a misspelt path, an item `std` has renamed) as a warning that `-D warnings`
   does not turn into an error, and then ignores the entry. So any diagnostic that
   points into clippy.toml fails this check.

Changing either list below is a portability decision: it goes in its own pull
request, with the reason. See CONTRIBUTING.md, "The portability rule".

Usage: python3 tools/check_core_portability.py
       (from anywhere; needs cargo with the clippy component)
Exit status 0 if all checks pass, 1 otherwise.

The checks are textual and strict on purpose. They are aimed at mistakes and
shortcuts, not at someone determined to evade them: code pulled in from outside
the crate directory with `include!` or `#[path]` is not scanned.
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CORE = ROOT / "crates" / "everygraph-core"

# Every package everygraph-core may link, with any feature, on any target (normal and
# build dependencies; dev-dependencies are for tests only and are not listed).
# All of these are pure Rust: the core takes no C or C++ dependency (DESIGN D1).
ALLOWED_DEPENDENCIES = {
    # The `parallel` feature. Only `everygraph_core::par` uses it.
    "rayon",
    "rayon-core",
    "crossbeam-deque",
    "crossbeam-epoch",
    "crossbeam-utils",
    "either",
    # The transcendentals of `everygraph_core::math`: arithmetic only, no I/O, no
    # dependencies of its own.
    "libm",
}

# The only conditions code in everygraph-core may be compiled under. Each one is
# covered by a lint run in CONTRIBUTING.md: default features, no default features,
# and a 32-bit target (the host is 64-bit).
ALLOWED_CFG = {
    "test",
    'feature = "parallel"',
    'target_pointer_width = "32"',
    'target_pointer_width = "64"',
}

REQUIRED_FORBID = {
    "unsafe_code",
    "clippy::disallowed_methods",
    "clippy::disallowed_types",
    "clippy::disallowed_macros",
}


def check_dependencies():
    """Returns a list of problems with the dependency tree of everygraph-core."""
    command = [
        "cargo", "tree", "--package", "everygraph-core", "--edges", "normal,build",
        "--all-features", "--target", "all", "--prefix", "none", "--format", "{p}",
    ]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        return [f"`{' '.join(command)}` failed:\n{result.stderr.strip()}"]
    found = {line.split()[0] for line in result.stdout.splitlines() if line.strip()}
    found.discard("everygraph-core")
    return [
        f"dependency `{name}` of everygraph-core is not in ALLOWED_DEPENDENCIES"
        for name in sorted(found - ALLOWED_DEPENDENCIES)
    ]


def split_top_level(text):
    """Splits `text` at commas that are outside parentheses and string literals."""
    parts, depth, start, in_string = [], 0, 0, False
    for i, char in enumerate(text):
        if char == '"':
            in_string = not in_string
        elif in_string:
            continue
        elif char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
        elif char == "," and depth == 0:
            parts.append(text[start:i])
            start = i + 1
    parts.append(text[start:])
    return [part.strip() for part in parts if part.strip()]


def atoms(predicate):
    """The leaf conditions of a cfg predicate, with `not`, `all` and `any` removed."""
    combinator = re.fullmatch(r"(?:not|all|any)\s*\((.*)\)", predicate, re.DOTALL)
    if combinator:
        return [a for part in split_top_level(combinator.group(1)) for a in atoms(part)]
    return [re.sub(r"\s*=\s*", " = ", " ".join(predicate.split()))]


CFG = re.compile(r"\bcfg(_attr)?\s*!?\s*\(")


def check_cfg():
    """Returns a list of `cfg` uses in everygraph-core outside ALLOWED_CFG."""
    problems = []
    for path in sorted(CORE.rglob("*.rs")):
        if "target" in path.relative_to(CORE).parts:
            continue
        text = path.read_text(encoding="utf-8")
        for match in CFG.finditer(text):
            where = f"{path.relative_to(ROOT)}:{text.count(chr(10), 0, match.start()) + 1}"
            depth, end = 1, match.end()
            while depth and end < len(text):
                depth += {"(": 1, ")": -1}.get(text[end], 0)
                end += 1
            if depth:
                problems.append(f"{where}: unbalanced parentheses after `cfg`")
                continue
            arguments = split_top_level(text[match.end() : end - 1])
            # `cfg_attr(predicate, attributes…)`: only the first argument is a predicate.
            if not arguments or (len(arguments) != 1 and not match.group(1)):
                problems.append(f"{where}: cannot read `{text[match.start() : end]}`")
                continue
            for atom in atoms(arguments[0]):
                if atom not in ALLOWED_CFG:
                    problems.append(
                        f"{where}: `cfg({arguments[0]})` uses `{atom}`, which is not in "
                        "ALLOWED_CFG: code under it would not be linted"
                    )
    return problems


FORBID = re.compile(r"#!\[forbid\(([^\]]*)\)\]")


def check_forbid():
    """Returns the crate roots of everygraph-core that lack the `forbid` line."""
    roots = [CORE / "src" / "lib.rs", CORE / "src" / "main.rs"]
    for directory in ("tests", "benches", "examples", "src/bin"):
        roots += sorted((CORE / directory).glob("*.rs"))
    problems = []
    for path in roots:
        if not path.exists():
            continue
        forbidden = set()
        for match in FORBID.finditer(path.read_text(encoding="utf-8")):
            forbidden |= {lint.strip() for lint in match.group(1).split(",")}
        for lint in sorted(REQUIRED_FORBID - forbidden):
            problems.append(f"{path.relative_to(ROOT)}: crate root lacks `#![forbid({lint})]`")
    return problems


def check_clippy_config():
    """Returns clippy's complaints about crates/everygraph-core/clippy.toml itself."""
    command = ["cargo", "clippy", "--package", "everygraph-core", "--all-targets"]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    problems, message = [], None
    for line in result.stderr.splitlines():
        if line.startswith(("warning", "error")):
            message = line
        elif "-->" in line and "clippy.toml" in line and message:
            problem = f"{line.split('-->')[1].strip()}: {message}"
            if problem not in problems:
                problems.append(problem)
            message = None
    return problems


def main():
    checks = [
        ("dependencies of everygraph-core are on the allow-list", check_dependencies),
        ("every cfg in everygraph-core is on the allow-list", check_cfg),
        ("every crate root of everygraph-core forbids the lints", check_forbid),
        ("clippy accepts every entry of the core's clippy.toml", check_clippy_config),
    ]
    failed = False
    for title, check in checks:
        problems = check()
        print(f"{'FAILED' if problems else 'ok'}: {title}")
        for problem in problems:
            print(f"  {problem}")
        failed = failed or bool(problems)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
