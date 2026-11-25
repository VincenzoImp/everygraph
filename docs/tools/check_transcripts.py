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
    "→": "->", "…": "...", "×": "x", "←": "<-", "▶": ">", "─": "-",
    "│": "|", "┌": "+", "┬": "+", "┐": "+", "├": "+", "┼": "+", "┤": "+",
    "└": "+", "┴": "+", "┘": "+", "≤": "<=", "≥": ">=", "±": "+/-",
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


def blocks(path):
    """Yield (first line number, language, lines) for each fenced block."""
    lang, start, body, fence = None, 0, [], None
    with open(path, encoding="utf-8") as f:
        for no, raw in enumerate(f, 1):
            line = raw.rstrip("\n")
            m = FENCE.match(line)
            if lang is None and m:
                lang, start, body, fence = m.group(3) or "", no + 1, [], m.group(2)
            elif lang is not None and m and m.group(2) == fence and not m.group(3):
                yield start, lang, body
                lang = None
            elif lang is not None:
                body.append(line)
    if lang is not None:
        raise SystemExit(f"{path}:{start}: code block is not closed")


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
