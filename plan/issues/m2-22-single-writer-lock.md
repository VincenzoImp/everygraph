---
title: One writer at a time on a graph file, across processes
milestone: M2 · Format and ingest
labels: type:feature, area:format, P1, risk:medium, size:S
---
**Why.** The container's update protocol assumes a single writer (`docs/spec/eg-container.md` § 8, § 11): two writers appending to one file at once start from the same committed end and write over each other's sections, and the last header write then points at a table whose sections the other writer may have overwritten. Within one process `FileSink` holds its file alone; between processes nothing stops a `map` run on the command line and a notebook committing a result to the same `NAME.eg` at the same time. `File::try_lock` (an advisory `flock`/`LockFileEx`) is stable from Rust 1.89; the MSRV is 1.100 since PR #214, so it can be used directly.

**Scope**
- [ ] An exclusive advisory lock taken by `FileSink::open` and `FileSink::create` for as long as the sink lives, released on drop and by the operating system when the process dies (so a crash never leaves a stale lock), shared locks for nothing
- [ ] Use `File::try_lock` (stable at the MSRV 1.100)
- [ ] A writer that finds the file locked gets `ResourceBusy` with a message naming the other process when the platform can tell; the command line and Python turn it into ux-contract's "Another run of this job is in progress" (R-7) or a message of their own
- [ ] Document what advisory means: a program that does not take the lock is not stopped
- [ ] Windows rename durability: `persist` cannot sync a directory there; check whether `MoveFileExW` with `MOVEFILE_WRITE_THROUGH` is needed and reachable, and record the answer in its docs

**Acceptance**
- Two processes opening `FileSink` on one file: the second is refused while the first lives and succeeds once it is killed with `kill -9` (no stale lock)
- The container's `kill -9` test still passes with the lock in place

**Depends on** {{#m2-01-container-spec}}
