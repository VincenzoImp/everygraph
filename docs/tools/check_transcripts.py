#!/usr/bin/env python3
"""Check the code blocks of a specification against the terminal rules.

usage: docs/tools/check_transcripts.py FILE.md [FILE.md ...]

For every fenced code block:
  * no line is wider than 80 columns;
  * JSON blocks (```json) parse, and a block that is a JSON Schema is itself
    checked for the keywords this repository's schemas use.
For blocks that show what a person sees (```console and ```text):
  * the ASCII rendering (docs/ux-contract.md, "Symbols and fallbacks") is
    built, must be pure ASCII and must also fit 80 columns;
  * a line that is redrawn in place (it contains a progress-bar cell) stops
    at column 78 in both renderings.
For a document that numbers its rules (**P-3**) and has a "Test hooks"
section:
  * every rule is cited in that section, alone or inside a range (P-6 – P-8).
For blocks marked as renderings of others (docs/ux-contract.md § 4.5):
  * a block preceded by `<!-- block: NAME -->` is a source; a block preceded by
    `<!-- rendering: KIND of NAME [+ NAME ...] -->` must equal the rendering the
    checker computes from those sources, where KIND is `ascii` (every symbol
    replaced), `narrow` (bars removed, as at 60 columns) or `piped` (commands,
    unfinished phases and bars removed, then ASCII; sources joined by one empty
    line).

Exit status 0 when every block passes, 1 otherwise. No third-party imports.
"""
import json
import re
import sys
import unicodedata

MAX, MAX_REDRAWN = 80, 78

# Unicode -> ASCII, as in docs/ux-contract.md. Wider fallbacks are the reason
# the ASCII rendering is measured separately.
FALLBACK = {
    "✓": "ok", "✗": "error:", "█": "#", "░": ".", "·": "|", "—": "-",
    "→": "->", "…": "...", "×": "x", "±": "+/-", "▶": ">", "─": "-",
    "│": "|", "┌": "+", "┬": "+", "┐": "+", "├": "+", "┼": "+", "┤": "+",
    "└": "+", "┴": "+", "┘": "+",
}
SHOWN = {"console", "text"}          # blocks a person sees on a terminal
FENCE = re.compile(r"^(\s*)(```+)\s*([A-Za-z0-9_-]*)\s*$")


def width(s):
    """Terminal cells: wide and full-width characters count two."""
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in s)


def ascii_form(line):
    out = "".join(FALLBACK.get(c, c) for c in line)
    if out.startswith("! "):         # the warning symbol at the left edge
        out = "warning: " + out[2:]
    return out


def ascii_lines(line):
    """The ASCII rendering of one line, as the lines it occupies.

    Rule W-3 of docs/ux-contract.md: a chain of identifiers joined by arrows
    that no longer fits once the symbols are replaced wraps before an arrow,
    never inside an identifier; continuation lines are indented two columns.
    """
    a = ascii_form(line)
    if width(a) <= MAX or " → " not in line or line.startswith("$ "):
        return [a]
    out, current = [], ""
    for i, piece in enumerate(a.split(" -> ")):
        part = piece if i == 0 else "-> " + piece
        joined = part if not current else current + " " + part
        if current and width(joined) > MAX:
            out.append(current)
            current = "  " + part
        else:
            current = joined
    return out + [current]


MARK = re.compile(r"^<!-- (block|rendering): (.+?) -->$")


def blocks(path, marks=None):
    """Yield (first line number, language, lines) for each fenced block.

    When `marks` is a dict, the mark comment on the line just before a block
    (MARK) is stored in it under the block's first line number.
    """
    lang, start, body, fence, previous = None, 0, [], None, ""
    with open(path, encoding="utf-8") as f:
        for no, raw in enumerate(f, 1):
            line = raw.rstrip("\n")
            m = FENCE.match(line)
            if lang is None and m:
                lang, start, body, fence = m.group(3) or "", no + 1, [], m.group(2)
                mark = MARK.match(previous)
                if mark and marks is not None:
                    marks[start] = (mark.group(1), mark.group(2))
            elif lang is not None and m and m.group(2) == fence and not m.group(3):
                yield start, lang, body
                lang = None
            elif lang is not None:
                body.append(line)
            previous = line
    if lang is not None:
        raise SystemExit(f"{path}:{start}: code block is not closed")


BAR = re.compile(r"^(  \S+\s+)([█░]{16})(  |$)")


def without_bar(line):
    m = BAR.match(line)
    return (m.group(1) + line[m.end():]).rstrip() if m else line


def render(kind, lines):
    """The rendering of a Unicode transcript, as docs/ux-contract.md § 4.5."""
    if kind == "ascii":
        return [ascii_form(x) for x in lines]
    if kind == "narrow":
        return [without_bar(x) for x in lines]
    if kind == "piped":
        out, continued = [], False
        for x in lines:
            if x.startswith("$ ") or continued:
                continued = x.endswith("\\")
                continue
            m = BAR.match(x)
            if m and "░" in m.group(2):
                continue            # piped: only finished phases are printed
            out.append(ascii_form(without_bar(x)))
        return out
    raise ValueError(f"unknown rendering {kind!r}")


def check_renderings(path):
    """Problems where a marked rendering differs from what its sources give."""
    marks, bodies, problems, compared = {}, {}, [], 0
    for start, _lang, body in blocks(path, marks):
        bodies[start] = body
    sources = {name: bodies[s] for s, (k, name) in marks.items() if k == "block"}
    for start, (k, spec) in sorted(marks.items()):
        if k != "rendering":
            continue
        m = re.fullmatch(r"(\w+) of (.+)", spec)
        if not m:
            problems.append(f"{path}:{start}: cannot read the mark {spec!r}")
            continue
        kind, names = m.group(1), [n.strip() for n in m.group(2).split("+")]
        missing = [n for n in names if n not in sources]
        if missing:
            problems.append(f"{path}:{start}: no block named {missing}")
            continue
        expected = []
        for i, n in enumerate(names):
            expected += ([""] if i else []) + render(kind, sources[n])
        actual = bodies[start]
        compared += 1
        for i in range(max(len(expected), len(actual))):
            e = expected[i] if i < len(expected) else "(nothing)"
            a = actual[i] if i < len(actual) else "(nothing)"
            if e != a:
                problems.append(f"{path}:{start + i}: {kind} rendering differs:"
                                f"\n    expected: {e}\n    found:    {a}")
                break
    return problems, compared


RULE = re.compile(r"\*\*([A-Z])-(\d+)\*\*")
CITED = re.compile(r"([A-Z])-(\d+)(?:\s*–\s*[A-Z]-(\d+))?")


def rules_without_a_hook(path):
    """Numbered rules that the "Test hooks" section never cites."""
    text = open(path, encoding="utf-8").read()
    defined = sorted({(a, int(n)) for a, n in RULE.findall(text)})
    m = re.search(r"^## .*Test hooks.*$", text, re.M)
    if not defined or not m:
        return None
    rest = text[m.end():]
    end = re.search(r"^## ", rest, re.M)
    hooks = rest[:end.start()] if end else rest
    cited = set()
    for line in hooks.splitlines():
        if not line.startswith("|"):
            continue
        for a, first, last in CITED.findall(line.split("|")[1]):
            for n in range(int(first), int(last or first) + 1):
                cited.add((a, n))
    return len(defined), [f"{a}-{n}" for a, n in defined if (a, n) not in cited]


def check(path):
    problems, count, widest, widest_ascii = [], 0, 0, 0
    for start, lang, body in blocks(path):
        count += 1
        for i, line in enumerate(body):
            where = f"{path}:{start + i}"
            w = width(line)
            widest = max(widest, w)
            if w > MAX:
                problems.append(f"{where}: {w} columns: {line}")
            if lang in SHOWN:
                rendered = ascii_lines(line)
                a = "\n".join(rendered)
                wa = max(width(r) for r in rendered)
                widest_ascii = max(widest_ascii, wa)
                if not a.isascii():
                    bad = sorted({c for c in a if ord(c) > 127})
                    problems.append(f"{where}: no ASCII fallback for {bad}")
                if wa > MAX:
                    problems.append(f"{where}: {wa} columns in ASCII: {a}")
                if "█" in line or "░" in line:
                    if w > MAX_REDRAWN or wa > MAX_REDRAWN:
                        problems.append(
                            f"{where}: redrawn line is {max(w, wa)} columns")
        if lang == "json":
            try:
                doc = json.loads("\n".join(body))
            except ValueError as e:
                problems.append(f"{path}:{start}: JSON does not parse: {e}")
                continue
            if isinstance(doc, dict) and "$schema" in doc:
                for key in ("$id", "title", "type"):
                    if key not in doc:
                        problems.append(f"{path}:{start}: schema lacks {key}")
    return problems, count, widest, widest_ascii


def main(paths):
    failed = False
    for path in paths:
        problems, count, widest, widest_ascii = check(path)
        for p in problems:
            print(p)
        rendering_problems, compared = check_renderings(path)
        for p in rendering_problems:
            print(p)
        problems += rendering_problems
        if compared:
            print(f"    {path}: {compared} renderings compared with their "
                  f"sources, {len(rendering_problems)} differ")
        coverage = rules_without_a_hook(path)
        if coverage:
            total, missing = coverage
            for rule in missing:
                problems.append(f"{path}: rule {rule} has no test hook")
            print(f"    {path}: {total} numbered rules, "
                  f"{len(missing)} without a test hook")
        state = "FAIL" if problems else "ok"
        print(f"{state}  {path}: {count} code blocks, widest line {widest} "
              f"columns, widest ASCII rendering {widest_ascii} columns, "
              f"{len(problems)} problems")
        failed = failed or bool(problems)
    return 1 if failed else 0


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    sys.exit(main(sys.argv[1:]))
