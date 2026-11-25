---
title: Fault-injecting ByteSource and ByteSink for tests
milestone: M2 · Format and ingest
labels: type:infra, area:engine, P1, risk:low, size:S
---
**Why.** Error paths that are never exercised drop errors: Gunawi et al. (FAST'08) found 13% of error-propagating calls dropped in Linux 2.6.15 file systems, 19.6% on sync, wait and write paths. Nothing in the workspace makes a write fail with ENOSPC or a read with EIO (no `fail_at` or injection in the code); the container's `Recording` sink fails at one operation, for writes only. After a failed `fsync`, ext4 and XFS keep the new data in the page cache and btrfs reverts it, and a retry is useless (Rebello et al., ATC'20). `ByteSource` and `ByteSink` (D2) are already the seam a simulator needs; FoundationDB attributes its record to exactly this style of testing (SIGMOD'21 § 4). Decided 2026-11-25 (`docs/research/README.md` T3).

**Scope**
- [ ] In a test-support module: a wrapper over any `ByteSource`/`ByteSink`, driven by a seed, that returns `ENOSPC` at the k-th write or `set_len`, `EIO` at the k-th read or sync, a short read, or (for the HTTP source) a transient error; and, after a failed sync, serves either the new bytes (ext4/XFS behaviour) or the old ones (btrfs)
- [ ] Swept over every k up to the operation count of a small run for: ingest and its spills ({{#m2-05-text-reader}}, {{#m2-08-csr-build}}), the dictionary build ({{#m2-07-id-mapping}}), the container writer and `compact` (#28, {{#m2-01-container-spec}}), resume manifests ({{#m2-13-preflight-resume}})
- [ ] Assertions per k: a typed error; scratch cleaned (`docs/ux-contract.md` C-8, C-9); the previous `.eg` intact; never a "success" after a failed sync
- [ ] A count of the error paths each sweep reaches (a `TEST()`-style counter, FoundationDB § 4), reported by the test
- [ ] A real ENOSPC on Linux without root, tried once: an unprivileged user namespace with a `size=`-limited tmpfs (`/proc/sys/kernel/unprivileged_userns_clone` is 1 on the lab machine; not tried by the report). On GitHub-hosted runners, `sudo mount -t tmpfs -o size=…`

**Acceptance**
- The sweep runs in seconds to a minute in CI, for every k, on the text reader's spill and the container writer first
- No sweep ends in a panic, a leaked scratch file or a damaged previous file

**Feeds.** {{#m2-13-preflight-resume}}, {{#m2-04-arrays-accountant}} (the k-th reservation failure of {{#m3-10-no-oom-ci}} is the same pattern for memory), #28, {{#m8-01-cli}} (C-8).

**Pointers.** `docs/research/sota-testing.md` § 2 ("Devices and errors", "Deterministic simulation"), § 3 (D2 row), § 4 challenge 4; `crates/everygraph-core` (`ByteSource`, `ByteSink`), `crates/everygraph-io`.

**Depends on** {{#m2-02-io-layer}}
