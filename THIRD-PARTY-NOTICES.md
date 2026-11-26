# Third-party material in this repository

Everything in this repository is the Everygraph contributors' work under the licences
named in [README.md](README.md#licence), except the files below, which keep their own
licence. Dependencies that Cargo downloads are not in the repository; their licences are
checked by `cargo deny check` against `deny.toml`.

None of these files is part of a crate, so none of them ends up in a build of the
engine.

## Fonts of the design system

Shipped with the design system (`design/system/`) and used by the previews under
`design/preview/`, through `design/system/fonts.css`. Unmodified.

| Files | Font | Copyright | Licence | Licence text |
|---|---|---|---|---|
| `design/system/fonts/InterVariable.woff2`, `InterVariable-Italic.woff2` | Inter | 2016 The Inter Project Authors (<https://github.com/rsms/inter>) | SIL Open Font License 1.1 (`OFL-1.1`) | `design/system/fonts/Inter-LICENSE.txt` |
| `design/system/fonts/JetBrainsMono-Regular.woff2`, `-Medium`, `-Bold`, `-Italic` | JetBrains Mono | 2020 The JetBrains Mono Project Authors (<https://github.com/JetBrains/JetBrainsMono>) | SIL Open Font License 1.1 (`OFL-1.1`) | `design/system/fonts/JetBrainsMono-OFL.txt` |

The OFL allows the fonts to be used, embedded and redistributed with other software,
provided that the licence text travels with them and the fonts are not sold on their
own. The licence files sit next to the fonts for that reason; keep them together.

## Generated JavaScript in the browser experiment

`lab/g-wasm/web/pkg-mt/` and `lab/g-wasm/web/pkg-mt-simd/` are build output of
`wasm-bindgen`, committed so the experiment can be rerun without rebuilding. One file in
each was copied in by the build from the `wasm-bindgen-rayon` crate:

| Files | Copyright | Licence |
|---|---|---|
| `lab/g-wasm/web/pkg-mt*/snippets/wasm-bindgen-rayon-38edf6e439f6d70d/src/workerHelpers.no-bundler.js` | 2022 Google Inc. | Apache-2.0 (notice in the file header) |

The rest of those directories is glue that `wasm-bindgen` generated from our own code.
