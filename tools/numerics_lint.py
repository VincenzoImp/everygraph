#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Writes, or checks, the numerics lint list in every `clippy.toml` that carries it.

Clippy reads the `clippy.toml` nearest to the crate it checks and does not merge files,
so the crates that have their own file carry a copy of the list (rules R2 and R9 of
docs/research/sota-numerics.md). This script is the one place the list is written.
Each copy sits between the BEGIN and END lines below, inside `disallowed-types` and
`disallowed-methods`.

With `--check` it also fails on an `impl Reduce for` a type that holds `f32` or `f64`
in the sources of a product crate: the edge-map framework adds a `Reduce` by reduction
blocks, which a mask and a materialized view cut differently, so a float reduced that way
can change its bits (`everygraph_kernels::edge_map`, "Determinism"). Such a float goes
through `ExactSum`.

Usage: python3 tools/numerics_lint.py           (rewrites the copies)
       python3 tools/numerics_lint.py --check   (exit status 1 if a copy differs)
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
FILES = [ROOT / "clippy.toml"] + [
    ROOT / "crates" / name / "clippy.toml"
    for name in (
        "everygraph-core", "everygraph-format", "everygraph-kernels",
        "everygraph-community", "everygraph-map", "everygraph-query",
    )
]
BEGIN = "    # BEGIN numerics list, written by tools/numerics_lint.py: edit it there.\n"
END = "    # END numerics list\n"

FUNCTIONS = ("sin cos tan asin acos atan atan2 sin_cos exp exp2 exp_m1 ln log log2 log10 "
             "ln_1p powf powi cbrt hypot sinh cosh tanh asinh acosh atanh").split()
PINNED = {"sin", "cos", "exp", "ln", "log2"}
PLATFORM = "platform maths, bits differ by platform"
RNG = ("not reproducible across threads, versions or platforms: use "
       "`everygraph_core::rng` (rule R9)")
RNG_METHODS = ["rand::thread_rng", "rand::rng", "rand::random", "getrandom::getrandom",
               "getrandom::fill", "getrandom::u32", "getrandom::u64", "fastrand::u32",
               "fastrand::u64", "fastrand::usize", "fastrand::f64", "fastrand::shuffle"]
RNG_TYPES = ["rand::rngs::StdRng", "rand::rngs::SmallRng", "rand::rngs::ThreadRng",
             "rand::rngs::OsRng", "rand_core::OsRng", "fastrand::Rng"]
POW = ("FreeBSD's `e_pow.c`, 636 ulp in its library (Gladman et al. 2026): use "
       "`everygraph_core::math::exp` and `ln`, or `x * x` (rule R2)")


def reason(f):
    if f == "powi":
        return f"{PLATFORM}: write `x * x`, or precompute the constant (rule R2)"
    if f in PINNED:
        return f"{PLATFORM}: use `everygraph_core::math::{f}` (rule R2)"
    return f"{PLATFORM}: use `everygraph_core::math`; add the function there (rule R2)"


def entry(path, why, invalid=False):
    extra = ", allow-invalid = true" if invalid else ""
    return f'    {{ path = "{path}", reason = "{why}"{extra} }},\n'


def types_block():
    return "".join(entry(p, RNG, True) for p in RNG_TYPES)


def methods_block():
    lines = [entry(f"{t}::{f}", reason(f)) for t in ("f64", "f32") for f in FUNCTIONS]
    lines += [entry(p, POW, True) for p in ("libm::pow", "libm::powf")]
    lines += [entry(p, RNG, True) for p in RNG_METHODS]
    return "".join(lines)


HEADER = ("    # Numerics (DESIGN D14; rules R2 and R9 of docs/research/sota-numerics.md).\n"
          "    # `allow-invalid`: the path is in a crate that may not be a dependency.\n")


def render(text):
    """Returns `text` with both lists' copies written."""
    for table, block in (("disallowed-types", types_block()),
                         ("disallowed-methods", methods_block())):
        body = BEGIN + HEADER + block + END
        opening = f"{table} = [\n"
        start = text.find(opening)
        if start < 0:
            text = text.rstrip("\n") + f"\n\n{opening}{body}]\n"
            continue
        start += len(opening)
        if text.startswith(BEGIN, start):
            end = text.index(END, start) + len(END)
            text = text[:start] + body + text[end:]
        else:
            text = text[:start] + body + "\n" + text[start:]
    return text


PRODUCT = ("core", "format", "io", "ingest", "kernels", "community", "map", "query", "cli",
           "py", "wasm")
REDUCE_IMPL = re.compile(r"impl\s*(?:<[^>{]*>)?\s*(?:[\w:]+::)?Reduce\s+for\s+([^{]+?)\s*\{")
FLOAT = re.compile(r"\bf(?:32|64)\b")


def definition(text, name):
    """The text of the `struct` or `enum` named `name` in `text`, or ''."""
    m = re.search(r"\b(?:struct|enum)\s+" + re.escape(name) + r"\b", text)
    if not m:
        return ""
    depth, i = 0, m.end()
    while i < len(text):
        c = text[i]
        if c in "({":
            depth += 1
        elif c in ")}":
            depth -= 1
            if depth == 0:
                return text[m.end():i + 1]
        elif c == ";" and depth == 0:
            return text[m.end():i]
        i += 1
    return text[m.end():]


def float_reductions():
    """`path: type` for each `impl Reduce for` a type with a float in a product crate."""
    found = []
    for crate in PRODUCT:
        for path in sorted((ROOT / "crates" / f"everygraph-{crate}" / "src").rglob("*.rs")):
            text = path.read_text(encoding="utf-8")
            for m in REDUCE_IMPL.finditer(text):
                ty = m.group(1).strip()
                name = re.match(r"\w+", ty)
                body = definition(text, name.group(0)) if name else ""
                if FLOAT.search(ty) or FLOAT.search(body):
                    found.append(f"{path.relative_to(ROOT)}: {ty}")
    return found


def main(check):
    stale = []
    for path in FILES:
        text = path.read_text(encoding="utf-8")
        wanted = render(text)
        if wanted != text:
            stale.append(path.relative_to(ROOT))
            if not check:
                path.write_text(wanted, encoding="utf-8")
    if check and stale:
        for path in stale:
            print(f"FAILED: {path} does not carry the current numerics list")
        print("Run python3 tools/numerics_lint.py to rewrite the copies.")
        return 1
    floats = float_reductions() if check else []
    if floats:
        for f in floats:
            print(f"FAILED: {f}: a float reduced by blocks; use `ExactSum`")
        return 1
    print("ok: the numerics list is the same in every copy" if check else
          f"wrote {len(stale)} of {len(FILES)} files")
    return 0


if __name__ == "__main__":
    sys.exit(main("--check" in sys.argv[1:]))
