---
title: The terminal, notebook, export and viewer previews follow the UX contract
milestone: M8 · CLI, docs and v0.1 launch
labels: type:docs, area:cli, P2, risk:low, size:M
---
**Why.** `docs/ux-contract.md` § 9 decided several changes to the approved previews, and its "Open points" table gave the edits to {{#m5-14-design-system}}. That issue closed with the shared assets only, so the pages still show what the contract replaced. The command line's golden files, the notebook cards, the export page and the viewer are checked against these pages, so a stale page invites a stale output.

**Scope**
- [ ] `design/preview/style/terminal.html`, by `docs/ux-contract.md` § 9: the default budget `(default: half of free RAM)` with a free-memory figure that keeps 8.0 GB true (item 2); scratch `.everygraph-7c1e` beside the output, not `/tmp` (item 14); "communities", never "groups", with the noun dropped where the line has no room (item 15); no `--precision low` way forward (item 16); every `✓` line with its total time (item 17); `discard:` names what to delete (item 18)
- [ ] The colour of the suggested command: the note "keeps blue, as specified, until that is decided" replaced by what `design/preview/style/SPEC.md` decided
- [ ] `design/preview/style/notebook.html`: `limit=` for `khop`, no "start over" on interruption, no bullets in the exception text, "cut short", "communities" in the step table, no `precision="low"` in the error card
- [ ] `design/preview/style/export.html` and `design/preview/style/viewer.html`: "communities", never "groups", where community detection's result is meant (item 15; captions, the region strip, the article text and the "How it was made" panel). Parquet's "row groups" is a different thing and stays. Any other form that § 9 replaced is changed the same way
- [ ] A docs-CI check (in `docs/tools/`, run by the docs job) that fails when a page under `design/preview/style/` contains a retired form. The list of retired patterns lives in one place, one line each with the § 9 item it comes from; the patterns are exact enough that allowed uses ("row groups") do not match
- [ ] The two rows of the "Open points" table of `docs/ux-contract.md` (§ 10) removed

**Acceptance**
- [ ] The docs-CI check passes on the four pages and fails on a copy of each page with one retired form put back (`fifth of free RAM`, `/tmp/everygraph`, "groups" for communities, `precision`, `rm -r`, `budget=` for `khop`): the check's test names it
- [ ] The four pages still render in the three themes

**Design reference.** `docs/ux-contract.md` § 9 and `design/DECISIONS.md` decide every change; this issue only typesets them. No new choice.
