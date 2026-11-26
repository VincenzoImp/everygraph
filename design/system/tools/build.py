#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Generate tokens.json and the mark files from tokens.css.

    python3 design/system/tools/build.py           write the generated files
    python3 design/system/tools/build.py --check   fail if any of them is out of date

tokens.json is for readers that cannot read CSS: the command line's terminal styles and
the Python text form. Its sections are "base" (theme-free values), "light" and "dark";
names drop the leading "--" and every var() reference is resolved.
The mark geometry is the one in design/preview/style/SPEC.md, "The mark".
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BLOCKS = {":root": "base", ':root, [data-theme="light"]': "light", '[data-theme="dark"]': "dark"}


def parse(css):
    css = re.sub(r"/\*.*?\*/", "", css, flags=re.S)
    out = {}
    for selector, body in re.findall(r"([^{}]+)\{([^}]*)\}", css):
        section = BLOCKS.get(selector.strip())
        if section is None:
            sys.exit(f"tokens.css: unexpected block {selector.strip()!r}")
        props = re.findall(r"--([\w-]+)\s*:\s*([^;]+);", body)
        out[section] = {name: " ".join(value.split()) for name, value in props}
    if set(out) != set(BLOCKS.values()):
        sys.exit(f"tokens.css: expected blocks {sorted(BLOCKS.values())}, found {sorted(out)}")
    return out


def resolve(tokens):
    def sub(value, scope):
        def one(m):
            name = m.group(1)
            if name not in scope:
                sys.exit(f"tokens.css: var(--{name}) is not defined")
            return sub(scope[name], scope)

        return re.sub(r"var\(--([\w-]+)\)", one, value)

    base = tokens["base"]
    out = {"base": {k: sub(v, base) for k, v in base.items()}}
    for theme in ("light", "dark"):
        scope = {**base, **tokens[theme]}
        out[theme] = {k: sub(v, scope) for k, v in tokens[theme].items()}
    if out["light"].keys() != out["dark"].keys():
        sys.exit("tokens.css: the light and dark themes define different tokens")
    return out


def ring(cx, cy, r, width, colour):
    return f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{colour}" stroke-width="{width}"/>'


def dot(cx, cy, r, colour):
    return f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{colour}"/>'


def svg(body, label="Everygraph"):
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img" '
            f'aria-label="{label}">{body}</svg>\n')


def nest(ink, dot_colour):
    return ring(16, 16, 13, 2.4, ink) + ring(18.5, 18, 7, 2.4, ink) + dot(20, 19.2, 2.6, dot_colour)


def seal(square, rings, dot_colour):
    # The Nest scaled into the square, so the clear space is the square's margin.
    return (f'<rect width="32" height="32" rx="3" fill="{square}"/>'
            '<g transform="translate(16 16) scale(0.875) translate(-16 -16)">'
            + ring(16, 16, 13, 2.4, rings) + ring(18.5, 18, 7, 2.4, rings) + "</g>"
            + dot(19.5, 18.8, 2.275, dot_colour))


def seal_small(square, rings, dot_colour):
    # 16 px and below: inner ring dropped, ring 3.6 and dot r 3.6 (SPEC.md, "The small Seal").
    return (f'<rect width="32" height="32" rx="3" fill="{square}"/>'
            + ring(16, 16, 10.6, 3.6, rings) + dot(18.4, 17.9, 3.6, dot_colour))


def marks(t):
    light, dark = t["light"], t["dark"]
    # On a Seal the square is the opposite theme's surface, so the dot takes that theme's accent.
    files = {
        "nest-light.svg": svg(nest(light["ink"], light["accent"])),
        "nest-dark.svg": svg(nest(dark["ink"], dark["accent"])),
        "nest-mono.svg": svg(nest("currentColor", "currentColor")),
        "seal-light.svg": svg(seal(light["ink"], light["bg"], dark["accent"])),
        "seal-dark.svg": svg(seal(dark["ink"], dark["bg"], light["accent"])),
        "seal-small-light.svg": svg(seal_small(light["ink"], light["bg"], dark["accent"])),
        "seal-small-dark.svg": svg(seal_small(dark["ink"], dark["bg"], light["accent"])),
    }
    style = "rect{{fill:{}}}.r{{stroke:{}}}.d{{fill:{}}}"
    favicon = ("<style>" + style.format(light["ink"], light["bg"], dark["accent"])
               + "@media (prefers-color-scheme:dark){"
               + style.format(dark["ink"], dark["bg"], light["accent"]) + "}</style>"
               '<rect width="32" height="32" rx="3"/>'
               '<circle class="r" cx="16" cy="16" r="10.6" fill="none" stroke-width="3.6"/>'
               '<circle class="d" cx="18.4" cy="17.9" r="3.6"/>')
    files["favicon.svg"] = svg(favicon)
    return {f"mark/{name}": body for name, body in files.items()}


def generated():
    tokens = resolve(parse((ROOT / "tokens.css").read_text()))
    head = {"$comment": "Generated from tokens.css by tools/build.py. Do not edit."}
    files = {"tokens.json": json.dumps({**head, **tokens}, indent=1, ensure_ascii=False) + "\n"}
    files.update(marks(tokens))
    return files


def main():
    check = sys.argv[1:] == ["--check"]
    stale = []
    for name, body in generated().items():
        path = ROOT / name
        if check:
            if not path.exists() or path.read_text() != body:
                stale.append(name)
        else:
            path.parent.mkdir(exist_ok=True)
            path.write_text(body)
    if stale:
        sys.exit("out of date with tokens.css (run design/system/tools/build.py): " + ", ".join(stale))
    print("design/system: generated files " + ("match" if check else "written from") + " tokens.css")


if __name__ == "__main__":
    main()
