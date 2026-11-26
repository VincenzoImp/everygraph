---
title: Storage probe: read bandwidth, and the device under btrfs, ZFS and overlayfs
milestone: M3 · Engine and kernels
labels: type:feature, area:engine, P2, risk:low, size:S
---
**Why.** The planner's time in the mapped regime divides the bytes it reads by a read rate. `everygraph_io::storage` ({{#m3-02-planner}}, PR #249) knows NVMe, SSD, HDD, memory and network file systems, and follows device-mapper and md devices to their disks. btrfs, ZFS and overlayfs (every Docker root file system) report no block device of their own, so they are `Unknown`, which the model reads as slow storage, and every rate except NVMe's is a placeholder.

**Scope**
- [ ] The source device of a mount from `/proc/self/mountinfo`, for file systems whose `st_dev` is anonymous (major 0)
- [ ] A quick read-bandwidth probe of the graph file's storage: a few MiB read with `O_DIRECT` at the start of an operation that will read the graph more than once, under the planning-time bound of P-2; its result replaces the placeholder rates
- [ ] macOS and Windows: the storage kind and free space through the platform's call, if the code stays small

**Acceptance**
- On this machine, the kind of a directory on the NVMe drive is `NVMe` under ext4, btrfs and overlayfs (a Docker container)
- The probe's rate is within 2× of an `fio` sequential read of the same file (exploratory), and planning stays within P-2's bound

**Depends on** {{#m3-02-planner}}
