# State of the art: hard memory budgets and reading data larger than RAM

Area **io-memory** of the state-of-the-art review of 2026-11-25. It covers the database and
operating-system side, not graph systems. Decisions covered: D2, D11, the memory promise
for the whole process (`design/DECISIONS.md`, m2-19), the safe mmap constructor, and FINDINGS
§ M1-01. The code read is `everygraph-core` (`Budget`, `Memory`, `Reservation`, `SegVec`,
`ScratchPool`, `ByteSource`) and `everygraph-io` (`FileSource`, `MmapSource`,
`ProcMemoryProbe`, sinks).

It builds on `docs/research/memory-bounded-algorithms.md` and does not repeat it.

The PDFs are in `/home/tux/graphlab/papers/io-memory/` and the text versions in `txt/`. I
read 4 works in full myself. Three helper agents read the others and reported numbers with
their tables or figures; I checked a sample of those numbers against the text (TriCache
19.3× and 45.0–47.2×, ELSAR "7% lower", IPS4o Table 1, MendSort 5,590 MB/s and 88%), and
all matched.

Numbers from papers are on the paper's machine and are **not comparable to ours as they
stand**. Nothing was built or run, except reading `/sys` and `rustc --version`. The machine
has kernel 6.12, `read_ahead_kb` 128, THP `always`, and rustc 1.100.0.

## 1. Scope and reading log

Read level: **full** = method and evaluation read (by me or by a helper, marked); **a/r** =
abstract and results only; **n/a** = not accessible. All PDF paths are relative to
`/home/tux/graphlab/papers/io-memory/`.

| Work | Venue, year | Link | Read | PDF |
|---|---|---|---|---|
| Crotty, Leis, Pavlo, "Are You Sure You Want to Use MMAP in Your DBMS?" | CIDR 2022 | https://db.cs.cmu.edu/papers/2022/cidr2022-p13-crotty.pdf | full (me) | 2022-crotty-mmap-dbms.pdf |
| Leis, Alhomssi, Ziegler, Loeck, Dietrich, "Virtual-Memory Assisted Buffer Management" (vmcache, exmap) | SIGMOD 2023 | https://www.cs.cit.tum.de/fileadmin/w00cfj/dis/_my_direct_uploads/vmcache.pdf | full (me) | 2023-leis-vmcache.pdf |
| Haas, Leis, "What Modern NVMe Storage Can Do, And How To Exploit It" | PVLDB 16, 2023 | https://www.vldb.org/pvldb/vol16/p2090-haas.pdf | full (me; §2–3 and §4.2–4.3) | 2023-haas-modern-nvme.pdf |
| Kuiper, Boncz, Mühleisen, "Robust External Hash Aggregation in the Solid State Age" | ICDE 2024 | https://hannes.muehleisen.org/publications/icde2024-out-of-core-kuiper-boncz-muehleisen.pdf | full (me) | 2024-kuiper-external-aggregation.pdf |
| Leis et al., LeanStore (pointer swizzling) | ICDE 2018 | https://db.in.tum.de/~leis/papers/leanstore.pdf | full (helper) | 2018-leis-leanstore.pdf |
| Neumann, Freitag, Umbra (variable-size pages) | CIDR 2020 | https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf | full (helper) | 2020-neumann-umbra.pdf |
| Leis, LeanStore retrospective | PVLDB 17(12), 2024 | https://www.vldb.org/pvldb/vol17/p4536-leis.pdf | full (helper) | 2024-leis-leanstore-vldb.pdf |
| Feng et al., TriCache | OSDI 2022 | https://www.usenix.org/system/files/osdi22-feng.pdf | full (helper) | 2022-feng-tricache.pdf |
| Meignan-Masson, Misono, Leis, Bhatotia, uCache | FAST 2026 | https://www.usenix.org/system/files/fast26-meignan-masson.pdf | full (helper) | 2026-meignan-ucache.pdf |
| Zhou, Hu, Pavlo, Stonebraker, Calico | arXiv 2604.00423, 2026 | https://arxiv.org/abs/2604.00423 | full (helper) | 2026-zhou-calico.pdf |
| Didona et al., libaio vs SPDK vs io_uring | SYSTOR 2022 | https://atlarge-research.com/pdfs/2022-systor-apis.pdf | full (helper) | 2022-didona-storage-apis.pdf |
| Jasny, El-Hindi, Ziegler, Leis, Binnig, "io_uring for High-Performance DBMSs" | PVLDB 19, 2026 | https://arxiv.org/abs/2512.04859 | full (helper; network part skimmed) | 2026-jasny-iouring-dbms.pdf |
| Ren, Trivedi, storage stacks on Optane | CHEOPS 2023 | https://atlarge-research.com/pdfs/2023-cheops-iostack.pdf | full (helper) | 2023-ren-storage-stacks.pdf |
| Pestka, Paradies, Pohl, "Asynchronous I/O — With Great Power…" | arXiv 2411.16254, 2024 | https://arxiv.org/abs/2411.16254 | a/r (numbers in plots only) | 2024-pestka-async-io.pdf |
| Li, Zhang, StreamCache | ATC 2024 | https://www.usenix.org/system/files/atc24-li-zhiyue.pdf | full (helper) | 2024-li-streamcache.pdf |
| Peng et al., ScalaCache | ATC 2024 | usenix.org ATC'24 (URL not re-checked) | a/r | 2024-peng-scalacache.pdf |
| Durner, Leis, Neumann, AnyBlob (object storage) | PVLDB 16, 2023 | https://www.vldb.org/pvldb/vol16/p2769-durner.pdf | full (helper) | 2023-durner-anyblob.pdf |
| Yang et al., SPDK | CloudCom 2017 | doi 10.1109/CloudCom.2017.14 | a/r (abstract only) | – |
| Papagiannis et al., Aquila | EuroSys 2021 | – | a/r (abstract only) | – |
| Jalalian et al., ExtMem (userspace paging) | ATC 2024 | https://www.usenix.org/system/files/atc24-jalalian.pdf | full (helper) | 2024-jalalian-extmem.pdf |
| Zhong et al., Lightswap (userfaultfd latency) | arXiv 2107.13848 | https://arxiv.org/abs/2107.13848 | relevant section | 2021-zhong-userspace-swapping-arxiv.pdf |
| Weiner et al., TMO (PSI, memory.high, senpai) | ASPLOS 2022 | https://www.pdl.cmu.edu/ftp/NVM/tmo_asplos22.pdf | full (helper) | 2022-weiner-tmo.pdf |
| Leijen, Zorn, de Moura, mimalloc | MSR-TR-2019-18 / APLAS 2019 | https://www.microsoft.com/en-us/research/uploads/prod/2019/06/mimalloc-tr-v1.pdf | full (helper) | 2019-leijen-mimalloc.pdf |
| Hunter et al., TCMalloc Temeraire | OSDI 2021 | https://www.usenix.org/system/files/osdi21-hunter.pdf | full (helper) | 2021-hunter-tcmalloc-temeraire.pdf |
| Powers et al., Mesh | PLDI 2019 | https://arxiv.org/abs/1902.04738 | full (helper) | 2019-powers-mesh.pdf |
| Pedreira et al., Velox (memory pools, arbitration) | PVLDB 15, 2022 | https://www.vldb.org/pvldb/vol15/p3372-pedreira.pdf | memory sections (helper) | 2022-pedreira-velox.pdf |
| Kuiper, Mühleisen, DuckDB sorting | ICDE 2023 | https://hannes.muehleisen.org/publications/ICDE2023-sorting.pdf | full (helper) | 2023-kuiper-sorting.pdf |
| Kuiper, Groß, Boncz, Mühleisen, "Saving Private Hash Join" | PVLDB 18(8), 2025 | https://www.vldb.org/pvldb/vol18/p2748-kuiper.pdf | full (helper) | 2025-kuiper-private-hash-join.pdf |
| Axtmann, Witt, Ferizovic, Sanders, IPS4o / IPS2Ra | ACM TOPC 2022 | https://arxiv.org/abs/2009.13569 | algorithm and results (helper) | 2022-axtmann-ips4o.pdf |
| Axtmann et al., IPS4o | ESA 2017 | https://arxiv.org/abs/1705.02257 | a/r | 2017-axtmann-ips4o-esa.pdf |
| Nyberg et al., AlphaSort | VLDB J. 1995 | https://www.vldb.org/journal/VLDBJ4/P603.pdf | full (helper) | 1994-nyberg-alphasort.pdf |
| Kristo, Kraska, ELSAR (learned external sort) | arXiv 2305.05671, 2023 | https://arxiv.org/abs/2305.05671 | full (helper) | 2023-kristo-elsar.pdf |
| Polyntsov et al., comparison-based external sort | arXiv 2207.12713 | https://arxiv.org/abs/2207.12713 | full (helper) | 2022-polyntsov-external-sort.pdf |
| Graefe, "Implementing sorting in database systems" | ACM CSUR 2006 | (helper copy; URL not re-checked) | §2–4 (helper) | 2006-graefe-sorting-dbms.pdf |
| Pang, Carey, Livny, memory-adaptive external sorting | VLDB 1993 | (helper copy) | full (helper) | 1993-pang-memory-adaptive-sort.pdf |
| Zhang, Larson, dynamic memory merge sort | VLDB 1997 | (helper copy) | method + setup | 1997-zhang-dynamic-memory-mergesort.pdf |
| Brodal, Fagerberg, Vinther, cache-oblivious sorting | JEA 2007 | (helper copy) | full (helper) | 2007-brodal-cache-oblivious-sort.pdf |
| Ajwani, Dementiev, Meyer, Osipov, cache-oblivious vs STXXL BFS | DIMACS 2006 | (helper copy) | relevant sections | 2006-ajwani-bfs-massive-graphs.pdf |
| Do, Graefe, offset-value coding; in-sort aggregation | TODS (arXiv 2209.08420); TODS 2022 | https://arxiv.org/abs/2209.08420 | intro, §4–6 | 2023-do-ovc-sorting.pdf, 2020-do-sort-aggregation.pdf, 2023-graefe-ovc-query-processing.pdf (abstract) |
| Mendelev, MendSort (JouleSort 2023) | sortbenchmark.org | sortbenchmark.org | full (helper) | 2023-mendelev-mendsort.pdf |
| Griffith, AMR5 sort | sortbenchmark.org 2023 | sortbenchmark.org | full (helper) | 2023-griffith-amr5sort.pdf |
| Luan et al., Exoshuffle-CloudSort | sortbenchmark.org 2022 | sortbenchmark.org | full (helper) | 2022-luan-exoshuffle-cloudsort.pdf |
| Shun, Dhulipala, Blelloch, Ligra+ | DCC 2015 | (helper copy) | a/r | 2015-shun-ligra-plus.pdf |
| Salwasser et al., TeraPart | arXiv 2410.19119 | https://arxiv.org/abs/2410.19119 | method + results | 2024-salwasser-terapart.pdf |
| Koohi Esfahani et al., ParaGrapher | arXiv 2404.19735 | https://arxiv.org/abs/2404.19735 | a/r | 2024-koohi-paragrapher.pdf |
| Aggarwal, Vitter, I/O model | CACM 1988 | – | n/a (403) | – |
| **Documentation (no PDF)**: cgroup-v2.rst, psi.rst, `mm/filemap.c`, `mm/readahead.c`, `mm/madvise.c` (torvalds master); madvise(2), mallopt(3), malloc_trim(3) (man7.org); jemalloc.3; mimalloc environment docs; DuckDB memory-management post (2024-07-09), OOM guide, `config.cpp` / `cgroups.cpp`, PR #14465 (temp-file compression, v1.2.0); ClickHouse `ServerSettings.cpp` and the memory-overcommit docs; Velox memory docs; PostgreSQL 18 resource docs; Rust `Allocator` docs | – | kernel.org, man7.org, duckdb.org, clickhouse.com, facebookincubator.github.io/velox | read by a helper; kernel facts spot-read by me through the agent's line references | – |

## 2. The state of the art, as a map

| Question | Main approaches | Who is best, with numbers (paper's machine) |
|---|---|---|
| Caching a read-only file larger than RAM | (a) OS page cache through `mmap`; (b) DBMS buffer pool with explicit `pread`, libaio or io_uring and `O_DIRECT`; (c) hybrids that use virtual memory for translation but keep control (vmcache, Umbra, Calico, uCache, TriCache) | **Random reads:** explicit control wins by large factors once misses are frequent. Crotty (EPYC 7713, 1 PM1733, 100 threads, 2 TB range, 100 GB cache): fio `O_DIRECT` about 900 K reads/s; mmap `MADV_RANDOM` near zero for about 5 s once the cache fills, then about half of fio. TriCache (2× EPYC 7742, 8 P4618): 45.0–47.2× over mmap at a 10% hit rate, 18.6–31.5× at 90%. uCache (EPYC 9654P, 1 CM7): ×46–×78 over mmap at 128→16 GiB quotas. **Sequential scans on one drive:** close to even. Umbra (i7-7820X, 960 EVO): cold sum 1.15 GiB/s mmap vs 1.13 GiB/s buffer manager. StreamCache: buffered ≈ `O_DIRECT` at about 2.5 GB/s on one SSD, but buffered gains ≤ 35% from 1 to 8 SSDs. Crotty: mmap scans drop once eviction starts; with 10 SSDs the gap is about 20×. **All hits:** mmap is fastest; software translation costs 23–64% (TriCache on Ligra, in memory). |
| Block size and eviction | 4 KiB pages for OLTP; 32 KiB–256 KiB for OLAP and temporary data; CLOCK, or cooling FIFO, everywhere | Haas–Leis: 4 KiB gives 6 GB/s random against a 6.5 GB/s maximum (PM1733). DuckDB: 256 KiB pages, LRU. uCache: 32 KiB buffers beat 8 × 4 KiB. LeanStore: hit rate of random/FIFO/cooling ≈ LRU within 0.6 points (Zipf 1.0). |
| Saturating NVMe | Many I/Os in flight: about 128 per drive for 4 KiB random | Haas–Leis: 1,000 in flight on 8 drives for "decent", 3,000 to saturate (12.5 M IOPS). vmcache Fig. 9: io_uring with 256 in flight per thread saturates 1.58 M IOPS with 3 threads; blocking `pread` needs 11. Jasny (EPYC 9654P, 8× CM7-R): fibers +10× over synchronous; registered buffers +11%, passthrough +20%, IOPOLL +21%, SQPOLL +32% with an extra core; "io_uring is not a panacea". Large sequential reads need far fewer threads (one core reads 90 GiB/s at 256 KiB with passthrough and IOPOLL). SPDK is best per core (Didona: 305 vs 171 KIOPS on one core) but bypasses the kernel. |
| No-OOM under a limit | Ledger with reservations; one pool for persistent and temporary data that spills; reconciliation with the process's real usage; arbitration between operators | **DuckDB:** `memory_limit` (default 80% of RAM, min with the cgroup's `memory.max`) covers the buffer pool and intermediates; spilling is graceful, with no cliff (Kuiper'24: only system to finish SF128 in 32 GB and 600 s). Operators that cannot spill can still OOM. Bundled jemalloc with purge settings. **ClickHouse:** hooks every allocation (MemoryTracker) and resets it every 50–100 ms to the cgroup or jemalloc resident value. **Velox:** hierarchical pools with reservations, a large-object mmap allocator, and an arbitrator that spills or aborts. Allocations through `malloc` are uncapped. **Umbra:** `MADV_DONTNEED` on eviction "is critical" to stay within the pool. **TMO / Meta:** PSI and `memory.high` / `memory.reclaim` for control. |
| What a cgroup charges | anon, page cache (incl. tmpfs/shmem), kernel memory: page tables, kernel stacks, slab, sockets | `memory.max`: reclaim first (page cache is reclaimable), then the OOM killer. "Regular 0-order allocations always succeed unless OOM killer chooses current task", so `malloc` and `try_reserve` never see the limit. Page tables of a file mapping are charged and **not** freed by reclaim, only by `munmap` (about 2 MiB per GiB touched with 4 KiB pages). |
| External sort / aggregation on NVMe | Two passes, by distribution (range or radix partition, then an in-RAM sort per bucket) or by merge; large I/O; I/O overlapped with compute | ELSAR (Xeon 6230, NVMe, 5–40× RAM): within 7% of the drive's read+write bandwidth with one partition pass and no merge; 17% fewer I/O bytes than Nsort. MendSort: 1 TB with 27 GB of RAM in two passes, 64–128 MB direct transfers, temp on separate drives, CPU 88%. Graefe 2006: "one partitioning step … before … a comparison-based sort on each resulting bucket". Cache-oblivious variants lose on disk: STXXL 25 min vs 96 min. |
| Spilled-byte reduction | Frame-of-reference narrowing; adaptive zstd per temp block; gap coding of sorted runs | DuckDB 1.2.0 adaptive zstd: TPC-H SF1000 Q18 temp went from 38–39 to 33–36 GB and time from 65.5 to 59.1 s. Compressed materialization: no factor reported. Gap coding of sorted adjacency: about 2× on Twitter (Ligra+), 2.7–3.4× on web graphs (TeraPart). Sort benchmarks forbid compression, so there is no evidence from them. |

## 3. Decision by decision

### D2 — portable core, `ByteSource`, segmented arrays, fallible allocation

- **Best known.** DuckDB's file-system abstraction (which also runs in wasm) and AnyBlob's
  ranged object reads put the same kind of layer below their buffer managers. AnyBlob's
  numbers for an HTTP range backend: 8–16 MiB requests, hundreds in flight, re-send any
  request still pending after about 600 ms (S3, c5n.18xlarge).
- **Verdict: level on the trait; ahead in having it pure-Rust and no-OS in the core.**
  - The positional, re-readable trait is exactly what a portable buffer cache needs (§ 4,
    C1). vmcache, Umbra and Calico all need `mmap` and `madvise`, so they do not port to
    wasm32. A cache written against `read_at` does.
  - `advise()` exists, but no production code calls it yet.
- **`SegVec` with a 1 GiB cap.** Fine.
  - Rust's `Allocator` trait is stable since 1.100.0 (checked: installed rustc 1.100.0).
    So `SegVec` segments could be `Vec<T, A>` with a budgeted page allocator, but the
    workspace MSRV is 1.87. Raising it is a decision.
  - `try_reserve` only reports allocator failure. It cannot turn a cgroup kill into an
    error (cgroup-v2.rst), which confirms that reserve-before-allocate in the ledger is the
    only way.

### D11 — three regimes, and the memory accountant behind the no-OOM guarantee

**Regime 1, mmap warm (the only in-RAM mode).**
- **Verdict: level.** Crotty's own advice is "maybe use mmap" when the data fits and is
  read-only, which is our case. Hits through the TLB beat any software translation (vmcache
  Table 2: a hash-table lookup costs 336 ns against 219 ns for a raw read at 128 GB).

**Regime 2, mmap under pressure.**
- **Verdict: behind for random access; roughly level for scans on one drive, at the price
  of the problems below.**
- Every source that measures random reads larger than the cache finds the page cache far
  slower than explicit control (§ 2). Our own data agree: the tracer re-read 733 GB (layout)
  and 1.80–2.74 TB (aggregation) for a 15 GB graph, and level-0 Leiden re-reads the store
  6–13× per sweep below 16 GiB.
- **Mechanism (kernel source, read by a helper):**
  - A miss in a `MADV_NORMAL` mapping reads 32 pages (128 KiB) around the faulting page
    (Crotty § 2.2 says the same).
  - The heuristic that turns this read-around off needs more than 100 net misses
    (`MMAP_LOTSAMISS`). The counter is decremented by every hit, so with most of the file
    cached the read-around never stops.
  - Under `memory.max`, the extra pages push out pages that are still needed.
  - 733 / 15 ≈ 49× is consistent with "up to 32× per miss × several passes". That is
    arithmetic, not a measurement of the mechanism.
- **What `MADV_RANDOM` changes:** it removes the read-around. Since about Linux 6.3 (not
  verified) the kernel also ignores that mapping's accessed bits in reclaim, so hot pages
  lose their protection.
- **Hints that look stronger than they are:**
  - `POSIX_FADV_RANDOM` has no effect on mmap faults.
  - `MADV_WILLNEED` reads at most 128 KiB per call on this machine.
  - So the lab's "WillNeed on a whole frontier" works only when it is issued in chunks.
- **Hidden costs:**
  - Users' tools show mapped file pages as RSS, so a budgeted run looks over budget.
  - A disk read error under a mapping is a `SIGBUS`, not an error value (Crotty § 3.3).
  - Page tables are charged to the cgroup and stay until unmap.

**Regime 3, semi-external streams.**
- **Verdict: level.** Large explicit reads with a fixed RSS are what ELSAR, MendSort and
  DuckDB do.
- `FileSource` already has 8 MiB direct requests and a 32 MiB in-flight cap. MendSort uses
  64–128 MB transfers, but across 5 drives; ELSAR found no gain from direct I/O on its
  NVMe. So buffered `pread` with `FADV_DONTNEED` behind the cursor is an acceptable fallback.

**The accountant.**
- **Reservations by purpose, refusals before allocating:** level with Velox (pool
  reservations rounded to 1–8 MB) and DuckDB (everything through the buffer manager).
- **Planning refusals before work starts:** ahead of DuckDB and Velox, which fail mid-query.
- **Behind on reconciliation with real usage:**
  - ClickHouse resets its tracker to the cgroup or jemalloc resident value every 50–100 ms
    because drift otherwise makes "every allocation fail".
  - DuckDB reads `memory.max` to set its default.
  - Our `ProcMemoryProbe` reads the limit once. Nothing compares `Budget::used()` with
    `memory.current` or `memory.stat` while a run is going.
- **Behind on kernel memory:** page tables (about 2 MiB/GiB mapped), kernel stacks
  (16 KiB per thread), slab and tmpfs scratch are charged by the cgroup and absent from the
  ledger's headroom rule.

### The memory promise covers the process (m2-19)

- **Best known.**
  - Large buffers bypass `malloc`. Velox has an mmap allocator for big objects and lets
    `malloc` handle small ones. Umbra and vmcache use anonymous mappings plus
    `MADV_DONTNEED`. TCMalloc returns memory at a steady background rate.
  - glibc's mmap threshold is dynamic: freeing an mmapped chunk of up to 32 MiB raises the
    threshold to that size, so later allocations below 32 MiB come from arenas and are kept.
    The number of arenas is 8 × cores on 64-bit. Setting `M_MMAP_THRESHOLD`,
    `M_TRIM_THRESHOLD` or `M_MMAP_MAX` turns the dynamic adjustment off (mallopt(3), glibc
    `malloc.c`).
  - That matches PR #158's 1.2–2.3× retention (inference: the PR did not identify which
    allocations were retained).
- **Verdict: behind, but cheap to close.**
  - No pure-Rust, multithreaded, general-purpose allocator is mature (talc, rlsf and
    dlmalloc are single-lock or embedded).
  - The `mimalloc` and `tikv-jemallocator` crates compile C, so they can never be the
    core's allocator. They are possible in the CLI only.
  - A Python extension cannot replace the interpreter's allocator. This argues for
    allocator-independent large buffers (§ 4, C2).

### Safe mmap constructor (`design/DECISIONS.md`)

- **Verdict: sound, but incomplete.**
  - The recorded invariant covers modification and truncation. It does not cover I/O
    errors: a media error, a network or removable file system, or a file on a drive that
    disappears all end the process with `SIGBUS` under a map, where `pread` returns `EIO`
    (Crotty § 3.3).
  - `MADV_POPULATE_READ` (Linux 5.14) pre-faults a range and returns an error instead of a
    signal. That makes it the right way to warm regime 1.
- **Changes proposed:**
  - The planner maps only files on local block devices. Everything else goes through
    `FileSource`.
  - The DECISIONS row gains one sentence on I/O errors.

### FINDINGS § M1-01: 733 GB re-read with default readahead

- **Diagnosis (mechanism above):** read-around amplification with a hot set larger than the
  page-cache room the cgroup leaves.
- **Three fixes, in rising cost and benefit:**
  1. `MADV_RANDOM` on the map. One line, not yet tried.
  2. An explicit block cache that reads the exact byte range of each list from the D4
     Elias–Fano offsets (C1).
  3. Locality-ordered work, which m5-01 already asks for.
- **Same diagnosis applies to:** the tile builder (§ M1-01: 68–491 s per zoom level under
  16 GiB) and level-0 Leiden below 16 GiB (§ M1-02).

### Planned issues

| Issue | Best known | Verdict |
|---|---|---|
| m2-02 I/O layer | DuckDB: own `pread` buffer manager, no mmap. MendSort: large direct transfers. Jasny: io_uring pays only with batching or fibers. | Level. Deferring io_uring is correct for scans on one drive. The block cache is missing (C1). |
| m2-04 accountant | Velox, DuckDB | Level for the ledger. The run-time reconciliation is missing (C3). |
| m2-19 allocator policy | Velox / Umbra large-object mmap; glibc tunables; ClickHouse-style purge | Behind; C2. |
| m3-03 semi-external | ELSAR, MendSort, StreamCache | Level. Add `FADV_DONTNEED` behind the cursor for the buffered mode. |
| m3-10 no-OOM CI | ClickHouse reconciliation; cgroup `memory.events` | The counting `GlobalAlloc` is fine for tests, but it is approximate: the compiler may elide allocations (GlobalAlloc docs). Add `memory.stat` kernel terms (`pagetables`, `kernel`, `shmem`) and `memory.events` `oom_kill` to the record. |
| m7-03 out-of-core queries | TriCache, uCache, Calico, vmcache | Already plans "an explicit block cache (no mmap) shared with the browser tier", but in M7. Layout (m5-01), tiles (m5-03) and Leiden (m4-04) need it first (C1). |
| m1-02 / m4-04 bucket aggregation | ELSAR, Graefe, DuckDB | **Level.** One range-partition pass, then a sort per bucket and an ordered commit, is the best-known shape for data up to 40× RAM. The lab demotes RAM chunks of *later* buckets first, which is DuckDB's measured policy ("partition 42 before … partition 0"). Gaps: 12-byte unpacked spill records (C4); per-list `sort_unstable` (C6). |
| m2-08 CSR build | DuckDB hybrid in-memory/spill; frame-of-reference narrowing | Behind in the lab: the lab ingest spills all or nothing (29.6 GB of temp at a 32 GB budget). The 6-byte entries are planned; C4 adds what is missing. |
| m3-15 segment size | – | Nothing in this area changes it. |

## 4. Challenges, ranked

**C1. A budgeted block cache over `ByteSource`, as the out-of-core path for random access.
It replaces "mmap under pressure" wherever the access is not a scan.**

- **What.**
  - Fixed-size frames, accounted as `Anonymous` in `Budget`, CLOCK eviction, sharded.
  - Misses read the exact compressed byte range of a list (or a 4 KiB-aligned span of it)
    from the Elias–Fano offsets, with batched, sorted and coalesced frontier requests
    (FlashGraph, as already in m3-01).
  - Pure logic in the core over `read_at`. `FileSource` `pread` now, io_uring later. The
    same code works over OPFS and HTTP ranges.
  - mmap stays for regime 1 (warm) and for scans until m3-03 lands.
- **Expected gain.**
  - TriCache: 45–47× over mmap at a 10% hit rate. uCache: ×46–×78. Crotty: mmap ≤ ½ of fio
    after the cache fills. Their machines are many-core with 1–8 enterprise SSDs, so our
    factor will be smaller.
  - Our own upper bound is the bytes: 733 GB re-read where the exact bytes needed are a
    few passes over 15 GB (estimate).
  - Our hit path is cheaper than TriCache's per-access instrumentation: one lookup per list
    or block, not per edge.
- **Cost and risk.**
  - About 600–1,000 lines in core plus tests (estimate). No `unsafe`, no C.
  - Results are unchanged: it caches bytes, not results, so determinism holds.
  - The budget becomes exact: no page-cache surprises, RSS equals the ledger. With
    `O_DIRECT` misses there is no double buffering.
  - Risk: a second code path to keep equal to mmap, which the oracle's cross-source check
    covers.
- **Cheapest experiment.**
  1. Lab `gmap` layout on Friendster under `MemoryMax=16G`: default against `MADV_RANDOM`,
     interleaved. Measure bytes read (cgroup `io.stat`), time, and
     `workingset_refault_file`. This settles whether a one-line fix is enough.
  2. Then M1-02 level-0 local moving at 8 and 4 GiB through a prototype cache against plain
     mmap: bytes read (406 and 799 GB now) and seconds.
- **Issue.** New "m2-21 block cache over ByteSource" (feeds m3-01, m4-04, m5-01, m5-03,
  m7-03). D11 is amended by ADR so that regime 2 is "explicit cache" for random access.

**C2. Large buffers never touch the general allocator.**

- **What.**
  - `SegVec` segments, `ScratchPool` slots and bucket buffers above about 1 MiB come from a
    page backend injected through `Memory`: anonymous mmap on native (2 MiB aligned, THP
    allowed, `munmap` on drop), `Vec` on wasm32. The core keeps no OS calls: the backend is
    a trait implemented in `everygraph-io`.
  - The CLI fixes glibc at start-up: `mallopt(M_ARENA_MAX, 2)`, a fixed
    `M_MMAP_THRESHOLD` (which turns off the dynamic growth to 32 MiB) and
    `M_TRIM_THRESHOLD`, plus `malloc_trim(0)` between stages. That is the `libc` crate,
    already a dependency of `-io`.
  - mimalloc v3 (MIT; `purge_delay=0`, `purge_decommits=1`) stays an A/B candidate for the
    CLI only.
- **Expected gain.**
  - Retention of large buffers drops to 0 by construction (Umbra, Velox).
  - PR #158's own A/B: `MALLOC_ARENA_MAX=1 MALLOC_TRIM_THRESHOLD_=0` took 241 MiB down to
    122 MiB at a 128 MiB budget.
- **Cost and risk.**
  - About 200 lines including about 40 of `unsafe` in `-io`, which needs a tier-2 review.
  - It works inside Python, where the interpreter's allocator cannot be changed.
  - No effect on results.
- **Cheapest experiment.** PR #158's dictionary build at a 128 MiB budget with URL keys.
  Four variants, interleaved: glibc default / `mallopt` pins / pins + page backend /
  mimalloc. Metric: anonymous peak ÷ ledger peak, and time.
- **Issue.** m2-19; amend its scope.

**C3. Reconcile the ledger with the cgroup while running, and stop thrashing early.**

- **What.**
  - A monitor in `everygraph-io` reads `memory.current`, `memory.stat` (`anon`,
    `pagetables`, `kernel`, `shmem`, `workingset_refault_file`, `pgmajfault`) and
    `memory.pressure` every 100 ms.
  - On anonymous drift beyond the headroom, it trims or purges.
  - When file refaults per pass exceed about one file's size, the stage is stopped with a
    refusal that names the regime that would work, instead of running 460–648 s and being
    killed (tracer rows 9, 11–12).
  - The headroom rule gains kernel terms: about 2 MiB per GiB mapped, 16 KiB per thread.
- **Expected gain.**
  - Turns the tracer's silent thrash-then-kill into an early, explicit refusal.
  - ClickHouse found reconciliation necessary in production.
- **Cost and risk.**
  - About 300 lines, Linux only. Elsewhere the monitor is a no-op.
  - Its decisions must not change results: it may stop or refuse a run, never switch
    precision.
- **Cheapest experiment.** Replay the tracer's Leiden-16G case (lab b2) with the monitor
  logging only. Measure the time to detection against the time to kill.
- **Issue.** m3-10 and m2-19; new rule in m3-02.

**C4. Write fewer bytes when spilling.**

- **What.**
  - (a) A RAM/file hybrid for ingest buckets, as M1-02 already does for aggregation.
    DuckDB keeps everything that fits and spills only the overflow, so there is no cliff.
  - (b) Frame-of-reference narrowing of spill records. The source as an offset inside its
    bucket's range: about 32 K nodes per bucket on Friendster, so 16 bits. Ingest
    (src, dst) goes from 8 B to 6 B (planned). Aggregation (cu, cv, w) goes from 12 B to
    about 8–9 B with a 16-bit `cu` offset and a narrow `w` where the range allows.
  - (c) Optional adaptive zstd per spill block, kept only when it writes faster (DuckDB
    1.2.0).
- **Expected gain.** On this drive the ingest is bound by bytes written (ADR 0026).
  - 55.4 GB at about 1.5 GB/s sustained ≈ 37 s, which matches the measured 36.97 s
    (arithmetic).
  - Temp −25% from (b) and the compressed output (about 10.5 GB instead of 15.5 GB, from
    #118's 9.8 GiB) bring the bytes to about 40 GB, about 27 s by the same arithmetic.
    That is an **estimate**: hybrid buckets (a) at 8 GB save more and are not estimated
    here.
  - Bytes under the drive's roughly 10 GB fast-write burst are about 3.5× cheaper
    (FINDINGS § B1).
  - Aggregation spills of 20–27 GB shrink by 25–33% (estimate).
  - (c) gave DuckDB about 8–15%.
- **Cost and risk.**
  - (a) and (b) are small. The output bytes do not change, because the spill format is
    private, so determinism is safe.
  - (c) adds a zstd dependency. The pure-Rust `ruzstd` decodes only. **Do not take it into
    the core** unless m2-17 brings a pure-Rust encoder.
- **Cheapest experiment.** Friendster ingest at 8 GB under the harness warm-up protocol,
  interleaved current against (a)+(b). Measure `io.write_bytes` and seconds.
- **Issue.** m2-08 (add the explicit hybrid and the bucket-order eviction rule); m4-04.

**C5. Fix the mmap hints now: cheap, and independent of C1.**

- **What.**
  - `MADV_RANDOM` for random-access phases. Watch the reclaim caveat on 6.3+ kernels and
    measure, because hot hub lists lose their accessed-bit protection.
  - `MADV_SEQUENTIAL` plus `MADV_DONTNEED`/`FADV_DONTNEED` behind the cursor for scans.
  - `MADV_WILLNEED` issued in ≤ 128 KiB chunks (it is capped per call).
  - `MADV_POPULATE_READ` to warm regime 1 (errors instead of `SIGBUS`).
  - Drop any expectation that `POSIX_FADV_RANDOM` affects maps.
- **Expected gain.** Up to the 32× read-around factor on random misses (upper bound,
  arithmetic). For the frontier prefetch the lab already measured 8.7 → 0.47 ms.
- **Cost.** A few lines in `MmapSource::advise` callers and documentation in
  `AccessHint::WillNeed`.
- **Experiment.** Shared with C1, step 1.
- **Issue.** m2-02 / m3-01.

**C6. One radix sort per bucket on a packed key, instead of a counting sort followed by
`sort_unstable` per list.**

- **Expected gain.**
  - IPS4o TOPC Table 1, sequential: std::sort is 2.4–3.4× and BlockPDQ (the lineage of Rust's
    `sort_unstable`) 1.5–2.7× slower than I1S2Ra on uint32, uint64 and 16-byte pairs.
    That is the helper's reading of the geometric means; I checked the uint32 and Pair rows.
  - In parallel the edge shrinks, because permutation is memory-bound.
  - Our per-list sorts are tiny (social-graph degrees), so the gain holds only if the whole
    bucket is sorted on the packed key at once.
  - It matters only where the build is CPU-bound: in-RAM ingest of lj and orkut, and the
    16.2 s in-RAM level-0 aggregation. It does not help the write-bound 8 GB ingest.
- **Cost.**
  - About 150 lines of LSD radix (pure Rust; `rdst` is MIT/Apache, `voracious_radix_sort`
    MIT but changing fast).
  - The output is identical: the key order is total.
- **Experiment.** Friendster buckets in RAM, interleaved A/B on the bucket-processing phase
  only.
- **Issue.** m2-08 and m4-03.

**C7 (watch, not now). io_uring with batched submission for cache misses.**

- With blocking `pread` each thread has one I/O in flight. 16–32 threads means a queue depth
  of 16–32, well below the about 128 per drive that saturates random 4 KiB reads (Haas–Leis;
  vmcache: 11 threads of `pread` against 3 of io_uring for 1.58 M IOPS).
- Worth it only once C1 exists and a measurement shows misses waiting on queue depth.
- IOPOLL needs `O_DIRECT`. Batches of 128 or more add 200 µs or more of latency (Jasny
  Table 2). Not portable.
- Issue: m2-02's io_uring hook, unchanged.

## 5. What we have and the literature does not (honest check)

| Claim | Prior art found | Verdict |
|---|---|---|
| Results bit-identical for any memory budget and thread count, out of core included | None of the sorting, aggregation or buffer-manager works discusses determinism across budgets. DuckDB, ELSAR and Exoshuffle schedule dynamically and rely on bucket order plus a deterministic per-bucket sort, which is our ordered commit. Deterministic floating-point reduction is known outside this area. | The **guarantee** is new in this space as far as found; the **techniques** are not. |
| Refusal before work, with the minimum budget, instead of failing mid-run | HyPer picks an external algorithm at plan time (Kuiper'24); DuckDB and Velox raise OOM during execution; semi-external planning by O(n) state goes back to Abello et al. | Partly new: a refusal that is planned and typed is better than DuckDB and Velox. |
| Ledger by purpose, with reservations owning their buffers (`SegVec`, `ScratchPool`) | Velox hierarchical pools with reservations; DuckDB buffer manager; ClickHouse MemoryTracker | Level, not new. |
| `ByteSource` portable down to a browser `File` | DuckDB-Wasm does the same with its file-system layer (known; not read here); AnyBlob for object stores | Not new. |
| In-process guard: sinks refuse to write a file (device, inode) that a live map holds | Not found in the works read | Small, possibly new; not a research claim. |
| RAM/file hybrid buckets where the smallest uncommitted bucket always keeps its reservation (no deadlock) | DuckDB's temporary memory manager assigns memory at sync points; Velox arbitration | Level. |

## 6. What to ignore, and why

- **SPDK.** Best per core (Didona: 305 vs 171 KIOPS), but it needs the drive unbound from
  the kernel, root and huge pages, and it is C. Not a desktop tool.
- **exmap.** A kernel module; it fixes TLB-shootdown and allocation scalability at about
  1.5 M page turnovers per second on 128 threads, which is not our regime.
- **userfaultfd / ExtMem / Lightswap.** Missing faults for anon, shmem and hugetlbfs only
  (not ext4 page cache); 6 µs to hundreds of µs per fault.
- **Pointer swizzling (LeanStore'18, Umbra).** Our data are read-only arrays addressed by
  offset; there are no pointers to swizzle.
- **TriCache's compiler instrumentation, uCache's unikernel, Calico, Aquila.** We control
  the access sites (one per list), so a library cache is enough.
- **ScalaCache.** Measured on a simulated computational SSD.
- **SQPOLL / IOPOLL / NVMe passthrough.** Gains of 20–32% at millions of IOPS, with spare
  cores and `O_DIRECT`; revisit only after C7.
- **Cache-oblivious sorting.** Loses on disk (STXXL 25 min vs 96 min; "TPIE is the clear
  winner").
- **ELSAR's learned CDF.** Our sampled-quantile range partition is the same idea.
- **Offset-value coding.** It speeds comparison merges of wide keys; ours are integers.
- **TMO / senpai offloading.** Needs swap or zswap; we run with `MemorySwapMax=0` and own
  our data.
- **Mesh.** Compacts small-object heaps; our large buffers leave the heap entirely (C2).
- **Replacement selection, polyphase merge.** One distribution pass suffices at our ratios.
