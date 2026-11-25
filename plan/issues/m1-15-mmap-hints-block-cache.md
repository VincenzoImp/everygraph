---
title: Random access under a budget: mmap hints first, then a budgeted block cache
milestone: M1 · De-risking lab, round 2
labels: type:experiment, area:engine, P1, risk:medium, size:M, lab-evidence
---
**Question.** Under a memory limit, random access through a memory map re-reads the store many times over. The tracer's layout read 733 GB for Friendster's 15.5 GB CSR under 16 GiB (FINDINGS § M1-01). Level-0 Leiden re-reads the store 6–13× per sweep below 16 GiB (FINDINGS § M1-02: 406 and 799 GB read at 8 and 4 GiB). Two fixes are possible, in rising cost:

1. One line: `MADV_RANDOM` on the map. It turns off the kernel's 128 KiB read-around on every miss.
2. An explicit, budgeted block cache over `ByteSource`. It reads the exact byte range of each list from the Elias–Fano offsets (D4).

Is (1) enough? If not, how much does (2) recover?

**What the literature says** (`docs/research/sota-io-memory.md` § 3 D11, C1, C5; paper numbers, their machines, not comparable as they stand):
- Once misses are frequent, every source finds the page cache far slower than explicit control for random reads: TriCache 45.0–47.2× over mmap at a 10% hit rate (2× EPYC 7742, 8 P4618); uCache ×46–×78 at 128→16 GiB quotas (EPYC 9654P); Crotty et al. find mmap at about half of fio `O_DIRECT` once the cache fills.
- The mechanism, from the kernel source: a miss in a `MADV_NORMAL` map reads 32 pages around the faulting one; the heuristic that turns this off (`MMAP_LOTSAMISS`) is decremented by every hit, so with most of the file cached it never fires. Under `memory.max`, the extra pages push out pages still needed.
- Caveat to measure: since about Linux 6.3 (not verified) `MADV_RANDOM` also makes reclaim ignore the mapping's accessed bits, so hot hub lists lose their protection.

**Method.** Every run under `systemd-run --user --scope -p MemoryMax=… -p MemorySwapMax=0`, through the harness on an idle machine, interleaved A/B in one harness run.
- [ ] Step 1 (no new code beyond one `madvise` call): the lab layout (`lab/e-map` `gmap`, as in the tracer) on Friendster under `MemoryMax=16G`, default against `MADV_RANDOM`. Measure bytes read (cgroup `io.stat`), time, and `workingset_refault_file` from `memory.stat`. Also the cold point-query benchmark of FINDINGS § B1 with and without `MADV_RANDOM`: bytes read per query
- [ ] Step 2: a prototype budgeted block cache in the lab: fixed-size frames counted in `Budget` as anonymous memory, CLOCK eviction, sharded; misses read the exact list byte ranges (or a 4 KiB-aligned span) with sorted, coalesced frontier requests. Compare it with plain mmap (default and `MADV_RANDOM`) on M1-02's level-0 local moving at 8 and 4 GiB: bytes read (406 and 799 GB today) and seconds; the deterministic hash must be identical
- [ ] Record the kernel-memory terms the cgroup charges during each run (`pagetables`, `kernel`, `shmem` from `memory.stat`)

**Success.** Step 1 cuts the layout's bytes read under 16 GiB by at least half, or step 2's cache brings level-0 local moving at 8 GiB within 1.5× of the in-RAM time. Otherwise, a documented explanation of why not.

**Decision it feeds.**
- If the cache wins, {{#m2-23-block-cache}} is built and an ADR amends D11: regime 2 becomes "an explicit cache for random access", with mmap kept for warm data and scans.
- If `MADV_RANDOM` alone suffices for a stage, the hint goes into {{#m2-02-io-layer}}'s `advise()` callers and into the planner ({{#m3-02-planner}}).
- Either way, D11 is no longer used to justify plain mmap for a random-access stage under a budget smaller than its working set (`docs/research/memory-bounded-algorithms.md` addendum § G).

**Pointers.** `docs/research/sota-io-memory.md` § 3 (D11, FINDINGS § M1-01), § 4 C1 and C5; `docs/research/memory-bounded-algorithms.md` addendum § E; `lab/m1-01-tracer`, `lab/m1-02-ooc-leiden`.

**Depends on** {{#m1-01-e2e-tracer}}, {{#m1-02-ooc-aggregation}}
