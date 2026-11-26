# m2-35 — random-read rate by read size and queue depth (#263)

**Exploratory.** The CPU was loaded during both runs (load average 12–70 on 32 hardware
threads, `results/*-load.txt`); the disk was nearly idle (see "Load"). The absolute rates
are an exploratory measurement of the device, not official numbers. The decision rests on
the pread/io_uring ratios, which come from interleaved cells of the same run.

## Question

What random-read rate does the NVMe give for 4, 16 and 64 KiB reads at queue depth 1–256?
Does a `pread` thread pool reach at least 90% of io_uring's IOPS, buffered and `O_DIRECT`?

## Answer

- **The pread pool reaches io_uring everywhere.** Against the best io_uring variant (one,
  4 or 16 rings), the median ratio of IOPS is 0.99–1.03 in every cell, except 64 KiB
  buffered at QD 64 and 256, where it is 0.94 and 0.95 (lowest repetition 0.92). All are
  above 0.90. **io_uring is not needed for the block cache (#178).**
- **One io_uring ring on one thread is slower than the pool**: by 1.26× (QD 64) and 1.60×
  (QD 256) at 4 KiB direct, and by 1.2–3.7× in buffered mode from QD 16 up. Splitting the
  queue over 4 or 16 rings, each on its own thread, closes the gap. So the limit is one
  submitting core, not the interface.
- **Queue depth that saturates the device:**

  | read size | saturating QD | peak (direct, pread) | share of peak at QD 16 | at QD 64 |
  |---|---|---|---|---|
  | 4 KiB | not reached at 256 | ≥ 907 kIOPS, 3.7 GB/s at QD 256 | 23% | 67% |
  | 16 KiB | 64 | 171 kIOPS, 2.8 GB/s | 77% | 100% |
  | 64 KiB | 64 (QD 16 is close) | 75.7 kIOPS, 4.96 GB/s | 93% | 100% |

  At 4 KiB the rate still grows from QD 64 to QD 256 (1.5×). With the CPU loaded we cannot
  tell whether 907 kIOPS is the device's limit or the host's.
- **16 KiB reads are slower in bytes/s than 4 KiB reads at QD 256** (2.8 against 3.7 GB/s),
  in both engines and both modes. This is a property of the device or its firmware as
  measured here; we did not look for its cause.

## Numbers for the planner's device model (m3-25, m3-31)

Exploratory, Samsung 990 PRO 2 TB, ext4, random 4 KiB-aligned reads of a 32 GB file,
`O_DIRECT`. Buffered reads with the pool are within 10% of these at 4 and 16 KiB, and
4–25% lower at 64 KiB (most at QD 1):

| read size | latency at QD 1 (p50) | IOPS at QD 1 | IOPS at QD 16 | peak IOPS | peak bytes/s |
|---|---|---|---|---|---|
| 4 KiB | 75 µs | 13.4 k | 210 k | ≥ 907 k (QD 256) | 3.7 GB/s |
| 16 KiB | 88 µs | 11.3 k | 131 k | 171 k (QD 64) | 2.8 GB/s |
| 64 KiB | 119 µs | 8.1 k | 70 k | 75.7 k (QD 64) | 4.96 GB/s |

A simple model close to these points (within 7%): time of one read = 71 µs + 0.74 µs
per KiB, and IOPS = min(QD / latency, peak IOPS of the size). The min() overestimates the knee: 16 KiB
at QD 16 gives 131 k IOPS, not 16 / 88 µs = 182 k; use the table where it has a point. The sequential rate is
7.07 GB/s (`bench/results/io/20261125T205022Z-21498df39590.jsonl`, cited by m3-25), so a
random 64 KiB read at QD ≥ 64 gets 70% of it, and a 16 KiB read 40%.

For the block cache: a pool of 64 reader threads saturates the device at 16 and 64 KiB
reads. 4 KiB reads want up to 256. Merging neighbouring ranges into 64 KiB reads gives the
highest bytes/s.

## Method

- File: `$EVERYGRAPH_DATA/ldbc-graphalytics/graphs/com-friendster/com-friendster.e`,
  32 364 651 652 bytes, read-only. This is the Friendster edge list, not the compressed
  store (`store-m113/friendster-shuf1/adj.bin`, 11 GB). We chose it for its larger range
  of fresh pages. The device does not see the file's contents, only the offsets.
- Offsets: uniform random, 4 KiB-aligned, the whole read inside the file. Read sizes 4, 16
  and 64 KiB. Queue depths 1, 4, 16, 64 and 256.
- Engines:
  - `pread`: QD threads, each with one blocking `pread` at a time, taking offsets from one
    shared atomic index.
  - `io_uring`: the raw `io-uring` crate 0.7, one ring on one thread, kept QD deep
    (`submit_and_wait(1)`, refill on each completion). No SQPOLL, no registered buffers or
    files, no `IOPOLL`.
  - `io_uring_r4`, `io_uring_r16` (second run only): 4 or 16 threads, each with its own
    ring, the QD split evenly among them.
- Modes: `O_DIRECT`, and buffered with `POSIX_FADV_RANDOM` (the device read bytes show no
  read-ahead: 1.0048 and 1.0016 device bytes per requested byte over all cells).
- Each cell runs 1.5 s or until its offset list ends (8 cells of 4 KiB buffered at QD 256,
  pread or 16 rings, ended first, after 1.2 million reads). Rate = reads completed / wall time from start to
  the last completion. Latency is per read, from submission to completion.
- Three repetitions, interleaved: the loop is repetition → size → QD → variant, and the
  variant order rotates by one at each (size, QD) step. A 1 s warm-up at 4 KiB QD 64 runs
  first and is not reported.
- Memory: `systemd-run --user --scope -p MemoryMax=4G -p MemorySwapMax=0`. Buffered pages
  count against this limit, so 64 KiB buffered cells at high QD also pay for page reclaim
  in the cgroup (pread p99 at QD 256: 42 ms buffered against 4 ms direct).

### How buffered reads avoid the page cache without `drop_caches`

1. At start, `mincore` over a mapping of the whole file (never touched) found 0 resident
   pages in both runs.
2. Each buffered offset is drawn only if `mincore` says none of its pages is resident at
   that moment, and no page is read twice in one cell.
3. After each buffered cell, `POSIX_FADV_DONTNEED` on the whole file drops the clean pages
   the cell brought in.
4. Check: the device read bytes per cell (`/proc/diskstats`) equal the requested bytes
   (ratios above). Every buffered read was a device read.

### Load

`/proc/diskstats` of `nvme0n1` was sampled for 10 s before each run with none of our
I/O: device busy 7.0% and 0.2% of the time. During the cells, the median share of device
reads that were not ours is 0.03% and 0.02%; the worst cell had 30% (16 KiB QD 1, one
repetition of the first run, 0.3 MB/s of foreign reads) and all others below 11%. Other
writes reached the device at up to 630 MB/s in some cells (median 2.6 MB/s). The CPU was
loaded by other work throughout (load average 12–70).

## Results

First run: `results/exploratory-20261126.jsonl` (`run.sh`). Second run with the
multi-ring engines: `results/exploratory-20261126-rings.jsonl`
(`run.sh --engines pread,io_uring,io_uring_r4,io_uring_r16 --qds 16,64,256`, with
`TAG=exploratory-20261126-rings`). Each line is one cell; `kind: idle` holds the idle
sample. Cells give median kIOPS over three repetitions, with min–max. "Device util" is the
share of wall time with I/O in flight (`io_ticks`); on NVMe it does not reach 1 at
saturation.

### First run, `O_DIRECT`

| size | QD | pread kIOPS | io_uring kIOPS | pread MB/s | pread / io_uring | pread p50/p99 µs | io_uring p50/p99 µs |
|---|---|---|---|---|---|---|---|
| 4 KiB | 1 | 13.4 (13.2–13.4) | 13.1 (12.7–13.3) | 55 | 1.03 (1.01–1.04) | 75/93 | 76/105 |
| 4 KiB | 4 | 53.5 (53.1–55.3) | 55.2 (50.7–55.6) | 219 | 1.00 (0.96–1.06) | 75/119 | 72/119 |
| 4 KiB | 16 | 203.4 (202.7–204.9) | 190.5 (182.1–193.8) | 833 | 1.06 (1.06–1.12) | 75/146 | 80/150 |
| 4 KiB | 64 | 606.3 (581.9–609.9) | 473.0 (467.5–482.8) | 2483 | 1.26 (1.23–1.30) | 85/284 | 127/285 |
| 4 KiB | 256 | 906.9 (888.1–907.2) | 566.2 (410.6–594.4) | 3715 | 1.60 (1.53–2.16) | 212/1488 | 432/895 |
| 16 KiB | 1 | 11.3 (10.9–11.4) | 11.2 (10.7–11.4) | 186 | 1.01 (1.00–1.02) | 88/138 | 89/117 |
| 16 KiB | 4 | 43.2 (39.8–43.3) | 43.8 (43.7–44.6) | 707 | 0.99 (0.89–0.99) | 88/180 | 86/161 |
| 16 KiB | 16 | 126.4 (119.3–128.9) | 126.3 (122.5–128.3) | 2070 | 1.00 (0.94–1.03) | 107/314 | 107/313 |
| 16 KiB | 64 | 163.4 (160.4–170.4) | 163.5 (152.5–170.5) | 2677 | 1.00 (1.00–1.05) | 348/830 | 351/1170 |
| 16 KiB | 256 | 164.7 (162.7–169.0) | 165.5 (165.0–169.6) | 2698 | 0.99 (0.97–1.02) | 1480/3585 | 1486/2839 |
| 64 KiB | 1 | 8.1 (8.0–8.1) | 8.0 (7.7–8.3) | 530 | 1.01 (0.96–1.05) | 119/226 | 118/186 |
| 64 KiB | 4 | 29.9 (29.8–30.2) | 30.1 (29.6–30.6) | 1959 | 0.99 (0.98–1.02) | 123/242 | 122/261 |
| 64 KiB | 16 | 70.1 (69.0–70.3) | 68.3 (68.2–70.0) | 4596 | 1.03 (0.99–1.03) | 203/558 | 204/578 |
| 64 KiB | 64 | 73.9 (73.0–75.4) | 74.4 (73.6–74.5) | 4844 | 0.99 (0.98–1.02) | 816/1752 | 813/1606 |
| 64 KiB | 256 | 74.1 (74.0–75.4) | 73.9 (73.4–75.0) | 4854 | 1.01 (1.00–1.01) | 3353/5819 | 3359/6068 |

### First run, buffered

| size | QD | pread kIOPS | io_uring kIOPS | pread MB/s | pread / io_uring | pread p50/p99 µs | io_uring p50/p99 µs |
|---|---|---|---|---|---|---|---|
| 4 KiB | 1 | 13.0 (12.6–13.4) | 13.0 (12.7–13.2) | 53 | 1.00 (0.98–1.01) | 77/96 | 78/94 |
| 4 KiB | 4 | 52.9 (52.0–54.0) | 51.0 (50.5–51.6) | 217 | 1.04 (1.03–1.05) | 76/123 | 78/132 |
| 4 KiB | 16 | 197.0 (183.5–204.8) | 170.1 (151.7–174.9) | 807 | 1.17 (1.16–1.21) | 78/146 | 90/174 |
| 4 KiB | 64 | 584.3 (581.6–600.2) | 232.1 (231.7–260.5) | 2393 | 2.51 (2.30–2.52) | 90/284 | 256/489 |
| 4 KiB | 256 | 866.5 (851.8–870.0) | 232.3 (230.1–276.6) | 3549 | 3.67 (3.13–3.78) | 198/1663 | 1088/1616 |
| 16 KiB | 1 | 10.3 (10.2–11.2) | 10.3 (10.2–10.7) | 169 | 1.02 (0.98–1.05) | 96/150 | 96/143 |
| 16 KiB | 4 | 41.9 (41.2–43.1) | 39.1 (33.3–40.2) | 686 | 1.07 (1.07–1.24) | 92/165 | 95/204 |
| 16 KiB | 16 | 119.2 (116.0–122.3) | 93.2 (91.6–95.9) | 1953 | 1.28 (1.27–1.28) | 115/328 | 155/403 |
| 16 KiB | 64 | 166.5 (166.0–170.6) | 100.2 (95.0–116.8) | 2728 | 1.66 (1.43–1.80) | 346/822 | 576/1482 |
| 16 KiB | 256 | 165.7 (161.4–169.6) | 83.2 (66.8–119.8) | 2714 | 1.94 (1.42–2.48) | 1475/3652 | 3021/4770 |
| 64 KiB | 1 | 6.1 (6.1–6.8) | 6.4 (6.2–6.5) | 402 | 0.97 (0.94–1.06) | 152/343 | 145/356 |
| 64 KiB | 4 | 26.5 (26.0–26.9) | 24.6 (24.1–24.9) | 1734 | 1.09 (1.05–1.10) | 141/281 | 146/374 |
| 64 KiB | 16 | 63.1 (61.9–64.0) | 42.3 (39.3–44.7) | 4132 | 1.51 (1.38–1.60) | 216/882 | 338/1275 |
| 64 KiB | 64 | 69.4 (68.9–71.3) | 45.4 (39.3–46.3) | 4551 | 1.52 (1.50–1.81) | 766/6504 | 1320/3223 |
| 64 KiB | 256 | 66.6 (65.7–69.1) | 36.9 (34.8–41.9) | 4364 | 1.81 (1.57–1.98) | 3257/41040 | 6646/11877 |

### Second run, against several rings, `O_DIRECT`

| size | QD | pread | io_uring | io_uring_r4 | io_uring_r16 | pread / best io_uring |
|---|---|---|---|---|---|---|
| 4 KiB | 16 | 210.0 (207.6–211.0) | 196.8 (193.6–198.7) | 203.8 (196.3–209.0) | 208.0 (199.4–208.8) | 1.01 (1.01–1.04) |
| 4 KiB | 64 | 608.5 (603.5–611.9) | 502.4 (479.6–545.4) | 603.0 (598.6–605.6) | 609.2 (596.1–610.4) | 1.00 (1.00–1.01) |
| 4 KiB | 256 | 907.2 (703.9–918.0) | 559.1 (384.7–580.5) | 911.3 (831.6–919.4) | 908.2 (902.6–921.5) | 1.00 (0.85–1.00) |
| 16 KiB | 16 | 131.4 (125.9–131.5) | 129.5 (129.4–129.6) | 130.7 (127.3–131.3) | 130.2 (128.7–130.8) | 1.00 (0.99–1.01) |
| 16 KiB | 64 | 171.0 (166.9–171.1) | 170.0 (168.9–170.3) | 169.9 (169.1–170.2) | 170.8 (170.8–171.2) | 1.00 (0.98–1.00) |
| 16 KiB | 256 | 171.2 (170.7–171.2) | 170.4 (169.2–171.4) | 170.8 (170.7–170.8) | 170.8 (169.9–171.0) | 1.00 (1.00–1.01) |
| 64 KiB | 16 | 70.2 (65.2–70.6) | 69.7 (69.7–70.0) | 70.3 (70.2–70.9) | 69.9 (69.5–70.6) | 1.00 (0.93–1.00) |
| 64 KiB | 64 | 75.7 (75.6–76.0) | 75.5 (75.2–75.6) | 75.3 (72.7–76.0) | 75.9 (75.2–76.0) | 1.00 (1.00–1.01) |
| 64 KiB | 256 | 75.7 (74.1–76.1) | 75.0 (74.1–75.6) | 76.0 (75.7–76.1) | 75.9 (70.9–76.2) | 1.00 (0.97–1.00) |

### Second run, against several rings, buffered

| size | QD | pread | io_uring | io_uring_r4 | io_uring_r16 | pread / best io_uring |
|---|---|---|---|---|---|---|
| 4 KiB | 16 | 203.3 (195.7–206.8) | 169.4 (167.0–176.6) | 200.4 (198.5–201.2) | 201.3 (198.7–201.6) | 1.02 (0.97–1.03) |
| 4 KiB | 64 | 604.4 (586.1–605.8) | 254.3 (215.6–289.5) | 551.3 (543.9–573.5) | 597.8 (594.9–601.5) | 1.00 (0.99–1.01) |
| 4 KiB | 256 | 855.9 (835.6–870.6) | 256.7 (247.7–263.8) | 737.3 (711.7–774.3) | 833.6 (785.9–884.5) | 1.03 (0.98–1.06) |
| 16 KiB | 16 | 125.2 (124.2–126.5) | 107.2 (103.3–111.2) | 121.9 (121.5–125.7) | 125.2 (125.2–127.3) | 0.99 (0.99–1.00) |
| 16 KiB | 64 | 169.7 (168.5–170.7) | 123.8 (104.3–137.1) | 166.4 (166.1–170.8) | 165.8 (162.2–170.5) | 1.01 (0.99–1.03) |
| 16 KiB | 256 | 170.9 (170.5–172.5) | 110.7 (86.7–112.7) | 168.7 (168.2–169.1) | 170.9 (170.6–170.9) | 1.00 (1.00–1.01) |
| 64 KiB | 16 | 66.2 (64.2–67.8) | 47.0 (42.4–48.0) | 61.7 (61.5–64.8) | 66.1 (62.7–66.8) | 1.01 (1.00–1.02) |
| 64 KiB | 64 | 66.4 (64.7–71.5) | 40.9 (35.4–48.5) | 68.8 (61.0–75.9) | 67.8 (64.8–72.8) | 0.94 (0.94–1.09) |
| 64 KiB | 256 | 67.1 (63.9–69.7) | 40.6 (34.8–42.0) | 70.1 (67.5–75.6) | 68.8 (68.7–71.7) | 0.95 (0.92–0.96) |

## Not measured

- Tuned io_uring: registered buffers and files, SQPOLL, `IOPOLL` on a polled NVMe queue.
  These cut CPU per read; they could matter only where the host, not the device, is the
  limit (4 KiB at QD 256).
- An idle CPU. The 4 KiB QD 256 point may move on an idle machine. `run.sh` reproduces
  every number for the official window.
- Other devices, file systems, or a file on another partition. Mixed reads and writes.
- The compressed store itself (see Method).
