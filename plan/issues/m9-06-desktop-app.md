---
title: Desktop application
milestone: M9 · After v0.1
labels: type:feature, area:viewer, P2, risk:low, size:L
---
**Why.** Analysts who do not use a terminal or Python need a double-click path with native file access (the browser tier is capped at ~100 M edges and cannot use out-of-core storage).

**Scope**
- [ ] Tauri shell around the viewer and the native engine (live mode without a visible server)
- [ ] File open/drag-and-drop for CSV/Parquet/`.eg`, ingest with preflight and progress, map, search, export
- [ ] Signed installers for Windows, macOS and Linux; auto-update
- [ ] Project files remembering views and annotations

**Acceptance**
- A non-programmer goes from a CSV to a shared map with no terminal

**Depends on** {{#m8-01-cli}}, {{#m5-08-live-mode}}, {{#m5-04-search-index}}
