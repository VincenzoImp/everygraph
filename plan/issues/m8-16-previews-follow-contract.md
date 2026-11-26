---
title: The terminal and notebook previews follow the UX contract
milestone: M8 · CLI, docs and v0.1 launch
labels: type:docs, area:cli, P2, risk:low, size:S
---
**Why.** `docs/ux-contract.md` § 9 decided several changes to the approved previews, and its "Open points" table gave the edits to {{#m5-14-design-system}}. That issue closed with the shared assets only, so the two pages still show what the contract replaced. The command line's golden files and the notebook cards are checked against these pages, so a stale page invites a stale output.

**Scope**
- [ ] `design/preview/style/terminal.html`, by `docs/ux-contract.md` § 9: the default budget `(default: half of free RAM)` with a free-memory figure that keeps 8.0 GB true (item 2); scratch `.everygraph-7c1e` beside the output, not `/tmp` (item 14); "communities", never "groups", with the noun dropped where the line has no room (item 15); no `--precision low` way forward (item 16); every `✓` line with its total time (item 17); `discard:` names what to delete (item 18)
- [ ] The colour of the suggested command: the note "keeps blue, as specified, until that is decided" replaced by what `design/preview/style/SPEC.md` decided
- [ ] `design/preview/style/notebook.html`: `limit=` for `khop`, no "start over" on interruption, no bullets in the exception text, "cut short", "communities" in the step table, no `precision="low"` in the error card
- [ ] The two rows of the "Open points" table of `docs/ux-contract.md` removed

**Done when.** Neither page contains a form that `docs/ux-contract.md` § 9 replaced (`grep` for `fifth of free RAM`, `/tmp/everygraph`, `groups`, `precision`, `rm -r`, `budget=`), and the pages still render in the three themes.

**Design reference.** `docs/ux-contract.md` § 9 and `design/DECISIONS.md` decide every change; this issue only typesets them. No new choice.
