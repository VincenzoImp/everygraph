#!/usr/bin/env python3
"""Inline src/tokens.css and src/shared.js into the three mockup pages so each
output file is self-contained. Usage: python3 build.py"""
import pathlib
here = pathlib.Path(__file__).parent
css = (here / "src/tokens.css").read_text()
js = (here / "src/shared.js").read_text()
for name in ("identity", "landing", "notebook"):
    src = here / f"src/{name}.html"
    if not src.exists():
        continue
    out = src.read_text().replace("/*@tokens*/", css).replace("/*@shared*/", js)
    (here / f"{name}.html").write_text(out)
    print("built", name)
