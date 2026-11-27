# Everygraph style

The shared assets of the Everygraph visual system (decision D24). The viewer, the
documentation site, the notebook cards and the terminal output take their colours, type
and marks from here. The rules that values cannot say are in
[`design/preview/style/SPEC.md`](../preview/style/SPEC.md); the approved previews are in
[`design/preview/style/`](../preview/style/).

| File | What it is |
|---|---|
| `tokens.css` | The source. Every colour, size, radius and duration as a CSS variable, light and dark themes (`data-theme="dark"`). |
| `tokens.json` | Generated from `tokens.css` for readers without CSS: the command line's terminal styles and the Python text form. Sections `base`, `light`, `dark`; names without `--`; `var()` resolved. |
| `fonts.css`, `fonts/` | Inter and JetBrains Mono, SIL Open Font License 1.1. The licence texts sit next to the fonts and travel with them. |
| `icons.svg` | The icon set as an SVG sprite: 16-unit grid, 1.5 stroke, `currentColor`. `<svg width="16" height="16"><use href="icons.svg#search"/></svg>`. |
| `mark/` | Generated from `tokens.css`: the Nest (light, dark, one colour), the Seal and the small Seal for 16 px and below (light and dark page), and `favicon.svg`, which follows the system theme. |
| `package.json` | The npm manifest: what the package ships (the files above) and its exports. |
| `tools/` | `build.py` (generates `tokens.json` and `mark/`), `measure.mjs` (contrast and palette floors), `lint_colours.py` (no colour literals in product stylesheets), `search-palette.mjs` (searches a categorical palette) and `color.mjs` (the colour maths the two Node tools share). No dependencies beyond Python's standard library and Node. |

In npm the package is `@everygraph/style`; it is kept private until the first release.

## Rules for users of the package

- Use `var(--name)`; never write a colour literal in a product stylesheet. CI checks this
  (`tools/lint_colours.py`).
- Data colours (`--d*`, `--s*`, `--v*`) are for data only. The ten categorical colours are
  always shown with labels; the first five are the region set.

## Changing a token

1. Edit `tokens.css`.
2. `python3 design/system/tools/build.py` regenerates `tokens.json` and `mark/`.
3. `node design/system/tools/measure.mjs` measures contrast, palette separation and ramps,
   and fails if a value falls below its floor.
4. For a new categorical palette, `tools/search-palette.mjs` searches one; measure it again.

CI runs `build.py --check`, `measure.mjs` and `lint_colours.py` on every pull request
and push (`.github/workflows/docs.yml`).
