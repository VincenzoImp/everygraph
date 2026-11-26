#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Fail when a product stylesheet hard-codes a colour instead of using a token.

    python3 design/system/tools/lint_colours.py

Checks every tracked .css, .scss and .less file and every <style> block of a tracked .html
file, except under design/ (where the tokens and the previews live) and lab/ (research
code). A colour is a hex literal or a colour function; write var(--name) instead.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXEMPT = ("design/", "lab/")
COLOUR = re.compile(r"#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(", re.I)
STYLE = re.compile(r"<style[^>]*>(.*?)</style>", re.S | re.I)


def stylesheets():
    files = subprocess.run(["git", "ls-files", "*.css", "*.scss", "*.less", "*.html"],
                           cwd=ROOT, capture_output=True, text=True, check=True).stdout.split()
    for name in files:
        if name.startswith(EXEMPT):
            continue
        text = (ROOT / name).read_text(encoding="utf-8")
        if name.endswith(".html"):
            # Keep only the <style> blocks, as blank lines elsewhere so line numbers stay true.
            text = STYLE.sub(lambda m: "\0" + m.group(1) + "\0", text)
            text = "".join(part if i % 2 else "\n" * part.count("\n")
                           for i, part in enumerate(text.split("\0")))
        yield name, text


def main():
    found = []
    for name, text in stylesheets():
        text = re.sub(r"/\*.*?\*/", lambda m: re.sub(r"[^\n]", " ", m.group()), text, flags=re.S)
        for n, line in enumerate(text.splitlines(), 1):
            for m in COLOUR.finditer(line):
                found.append(f"{name}:{n}: literal colour {m.group()!r}; use a token from design/system/tokens.css")
    if found:
        sys.exit("\n".join(found))
    print("no literal colours in product stylesheets")


if __name__ == "__main__":
    main()
