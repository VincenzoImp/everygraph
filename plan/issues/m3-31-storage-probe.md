---
title: "Storage probe: the device under btrfs, ZFS and overlayfs, and its read rate"
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P2, risk:low, size:S
---
**Why.** `everygraph_io::storage` ({{#m3-02-planner}}, PR #249) tells NVMe, SSD, HDD, memory and network file systems apart, and follows device-mapper and md devices to their disks. btrfs, ZFS and overlayfs (every Docker root file system) report no block device of their own, so they come out `Unknown`. And nothing measures the device the graph is on: the rate the planner divides by is a constant ({{#m3-25-memory-headroom}} owns it).

**Scope**
- [ ] The source device of a mount from `/proc/self/mountinfo`, for file systems whose `st_dev` is anonymous (major 0), then the same walk to the disks
- [ ] A quick read probe of the graph file's storage: a few MiB read with `O_DIRECT`, sequential and at random in the sizes and queue depths that {{#m2-35-random-read-probe}} finds matter, within the planning-time bound of `docs/ux-contract.md` P-2. It reports rates; using them in the cost model is {{#m3-25-memory-headroom}}'s
- [ ] macOS and Windows: the storage kind and the free space through the platform's call, if the code stays small

**Acceptance**
- On this machine, a directory on the NVMe drive is `NVMe` under ext4, btrfs and overlayfs (a Docker container)
- The probe's sequential rate is within 2× of an `fio` read of the same file (exploratory), and planning stays within P-2's bound

**Depends on** {{#m3-02-planner}}, {{#m2-35-random-read-probe}}
