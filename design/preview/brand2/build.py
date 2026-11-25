#!/usr/bin/env python3
"""Inline src/systems.css, src/systems.js, src/tokens.json and src/palettes.json into
systems.html so the output is one self-contained file. Usage: python3 build.py
(palettes.json is produced by `node tools/palette.mjs`)."""
import pathlib
here = pathlib.Path(__file__).parent
rd = lambda n: (here / "src" / n).read_text()
out = (rd("systems.src.html")
       .replace("/*@css*/", rd("systems.css"))
       .replace("/*@tokens*/", rd("tokens.json"))
       .replace("/*@palettes*/", rd("palettes.json"))
       .replace("/*@js*/", rd("systems.js")))
(here / "systems.html").write_text(out)
print("built systems.html", len(out), "bytes")
