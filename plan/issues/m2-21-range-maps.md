---
title: Map a graph file by range, so a process can commit to a graph it holds mapped
milestone: M2 · Format and ingest
labels: type:feature, area:format, P1, risk:high, size:M
---
**Why.** Results are kept in the graph file (`docs/api-sketch.md` § 2.8, ux-contract C-2), and the in-RAM mode is a memory map of that file (DESIGN D11). Today `MmapSource` maps whole files, so the in-process guard refuses a `FileSink` on a mapped file: a session that holds its graph mapped must drop every map before it can commit a result, and a commit by *another* process to a file this one maps breaks the recorded invariant (`design/DECISIONS.md`, "Writing a graph file again: the container's commits") — a commit rewrites 128 bytes of a header block and may cut off an uncommitted tail, both inside a whole-file map. The container's protocol never touches the range from offset 8192 to the committed end of any earlier commit (`docs/spec/eg-container.md` § 11), so a map of that range only is safe against every commit.

**Scope**
- [ ] `MmapSource` maps a byte range of a file (offset rounded down to a page, the slice handed out exactly the range); the `unsafe` block in `everygraph-io/src/sys.rs` stays one call, with its SAFETY comment and the CONTRIBUTING `unsafe` table updated
- [ ] The in-process guard (`src/claim.rs`) records mapped ranges; `FileSink` declares where it writes (header blocks, and from the committed end on) and is refused only when a write or a length change would reach a mapped range
- [ ] The container's readers that map (the in-RAM mode, #46 and the kernels' entry points) map `8192 … committed end` and read the header blocks with positional reads
- [ ] DECISIONS row and the `MmapSource` docs say exactly what is then safe across processes
- [ ] Independent review (Tier 2: `unsafe`, the guard)

**Acceptance**
- A process maps a graph, computes a result, commits it to the same file without dropping the map, and the mapped bytes compare equal before and after the commit
- A second process commits to a file the first maps by range, 100 times, with no `SIGBUS` and no change to the mapped bytes
- The guard still refuses a write that would reach a mapped range (test)

**Depends on** {{#m2-01-container-spec}}
