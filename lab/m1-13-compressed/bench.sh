#!/bin/bash
# Experiment #118 (m1-13): the default storage end to end. Reproduces every number of the report
# and writes JSON lines under results/ (one object per measurement) plus results/summary.{json,txt}.
#
#   ./bench.sh                 all stages, in order (about 4 hours on the lab machine)
#   ./bench.sh encode kernels  only the named stages
#   stages: build encode baseline scan open kernels kernels_big leiden leiden_big pq ablation capped perf summarize
#
# Environment: EVERYGRAPH_DATA (default /home/tux/graphlab/data, read-only inputs in csr/),
#              SCRATCH (default /home/tux/graphlab/scratch/m1-13: build outputs and encoded stores,
#              ~21 GB; delete it when done).
# Machine rules: run it alone on an idle machine. It takes no lock (do not wrap it in one) and
# starts nothing in the background. Before every measurement it waits until fewer than 1.5 CPUs
# are busy with other work (busy.py) and records that number and the 1-minute load average in the
# row. Everything is built with target-cpu=native. No network needed (cargo --offline).
set -u
cd "$(dirname "$(readlink -f "$0")")"
HERE=$PWD
DATA=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}/csr
S=${SCRATCH:-/home/tux/graphlab/scratch/m1-13}
R=$HERE/results
B=$S/target/release/m113
LAB=$HERE/..
mkdir -p "$R/logs" "$R/perf" "$S/store"
MAIN="lj orkut twitter-sym friendster"

log() { echo "[$(date +%H:%M:%S)] $*"; }

# Wait until the machine is quiet (other processes use < 1.5 CPUs); export the measured value.
gate() {
  local tries=0 busy
  while :; do
    busy=$(python3 -I "$HERE/busy.py")
    awk "BEGIN{exit !($busy < 1.5)}" && break
    tries=$((tries + 1))
    if [ $tries -ge 60 ]; then log "gate: still $busy CPUs busy after 5 min, measuring anyway (row is flagged by busy_cpus_before)"; break; fi
    sleep 5
  done
  export M113_BUSY=$busy
}
# Start of a stage: additionally require a 1-minute load average below 3 (waits up to 10 min).
stage_start() {
  local i=0
  while awk '{exit !($1 >= 3)}' /proc/loadavg && [ $i -lt 120 ]; do sleep 5; i=$((i + 1)); done
  log "== stage $1 (loadavg $(cut -d' ' -f1-3 /proc/loadavg))"
}

# kernel run: graph config threads algos reps [extra m113 args]
k() {
  local g=$1 cfg=$2 t=$3 algos=$4 reps=$5; shift 5
  gate
  "$B" kernel --graph "$DATA/$g" --store "$S/store/$g" --config "$cfg" --threads "$t" --algos "$algos" --reps "$reps" --json "$R/${OUT:-kernels}.jsonl" "$@" > /dev/null
}
# leiden run: graph config mem threads repeat [extra]
ld() {
  local g=$1 cfg=$2 mem=$3 t=$4 rep=$5; shift 5
  gate
  "$B" leiden run --graph "$DATA/$g" --store "$S/store/$g" --config "$cfg" --mem "$mem" --algo leiden --threads "$t" --repeat "$rep" --json "$R/${OUT:-leiden}.jsonl" "$@" > /dev/null
}
# run an original lab binary that prints one JSON object; append it with the gate values
orig() {
  local out=$1 tool=$2; shift 2
  gate
  local la; la=$(cut -d' ' -f1 /proc/loadavg)
  "$@" 2>/dev/null | python3 -I -c '
import sys, json
out, tool, la, busy = sys.argv[1:5]
for l in sys.stdin:
    l = l.strip()
    if l.startswith("{"):
        r = json.loads(l); r["tool"] = tool; r["loadavg_1m_start"] = float(la); r["busy_cpus_before"] = float(busy)
        open(out, "a").write(json.dumps(r) + "\n")' "$out" "$tool" "$la" "$M113_BUSY"
}

stage_build() {
  stage_start build
  (cd "$HERE" && CARGO_TARGET_DIR=$S/target nice -n 10 cargo build --release --offline -j 16 2>&1 | tail -1)
  (cd "$HERE" && CARGO_TARGET_DIR=$S/target nice -n 10 cargo test --release --offline -j 16 2>&1 | grep "test result")
  # the lab prototypes, unchanged, for the baseline check (b1: target-cpu=native via its .cargo/config.toml; b2, f1: default target)
  (cd "$LAB/b1-core" && CARGO_TARGET_DIR=$S/t-b1 nice -n 10 cargo build --release --offline -j 16 2>&1 | tail -1)
  (cd "$LAB/b2-leiden" && CARGO_TARGET_DIR=$S/t-b2 nice -n 10 cargo build --release --offline -j 16 2>&1 | tail -1)
  (cd "$LAB/f1-query" && CARGO_TARGET_DIR=$S/t-f1 nice -n 10 cargo build --release --offline -j 16 --bin pq 2>&1 | tail -1)
  [ -x "$B" ] || { echo "build failed"; exit 1; }
}

# 1. encode into the proposed default format: bits/edge, encode time; exactness check
stage_encode() {
  stage_start encode
  : > "$R/encode.jsonl"; : > "$R/verify.jsonl"
  for g in $MAIN twitter-dir twitter-dir-in; do
    gate; "$B" encode --graph "$DATA/$g" --out "$S/store/$g" --threads 16 --reps 3 --fsync --json "$R/encode.jsonl" > /dev/null
    gate; "$B" verify --graph "$DATA/$g" --store "$S/store/$g" --threads 16 --json "$R/verify.jsonl" > /dev/null || { echo "VERIFY FAILED $g"; exit 1; }
  done
  for g in $MAIN; do   # single-thread encode time (store written to a temporary directory)
    gate; "$B" encode --graph "$DATA/$g" --out "$S/store/tmp-1t" --threads 1 --reps 1 --json "$R/encode.jsonl" > /dev/null
    rm -rf "$S/store/tmp-1t"
  done
}

# 2a. the lab prototypes as they are: do the idle numbers of docs/FINDINGS.md reproduce here?
stage_baseline() {
  stage_start baseline
  : > "$R/baseline_b1.jsonl"; : > "$R/baseline_b2.jsonl"; rm -f "$R"/baseline_f1_*.json
  local G1=$S/t-b1/release/glab G2=$S/t-b2/release/b2 G3=$S/t-f1/release/pq
  for t in 1 16; do
    for a in pr32 pr64 wcc bfs kcore; do orig "$R/baseline_b1.jsonl" b1-core "$G1" run --graph "$DATA/orkut" --algo $a --threads $t --load heap --reps 3; done
  done
  orig "$R/baseline_b1.jsonl" b1-core "$G1" run --graph "$DATA/orkut" --algo kcore --threads 8 --load heap --reps 3
  for t in 4 16 32; do orig "$R/baseline_b1.jsonl" b1-core "$G1" run --graph "$DATA/orkut" --algo tc --threads $t --load heap --reps 2; done
  orig "$R/baseline_b1.jsonl" b1-core "$G1" run --graph "$DATA/orkut" --algo pr32 --threads 32 --load heap --reps 3
  for a in wcc bfs; do orig "$R/baseline_b1.jsonl" b1-core "$G1" run --graph "$DATA/friendster" --algo $a --threads 32 --load mmap --reps 3; done
  orig "$R/baseline_b1.jsonl" b1-core "$G1" run --graph "$DATA/friendster" --algo pr32 --threads 32 --load mmap --iters 20 --reps 1
  for g in lj orkut; do for t in 1 16; do
    orig "$R/baseline_b2.jsonl" b2-leiden "$G2" run --graph "$DATA/$g" --algo leiden --threads $t --repeat 3
    orig "$R/baseline_b2.jsonl" b2-leiden "$G2" run --graph "$DATA/$g" --algo leiden --threads $t --repeat 3 --det
  done; done
  for g in lj orkut friendster; do
    gate; "$G3" --graph "$DATA/$g" --name $g --threads 32 --cold 0 --out "$R/baseline_f1_$g.json" > /dev/null 2> "$R/logs/baseline_f1_$g.err"
  done
  gate; "$G3" --graph "$DATA/twitter-dir" --in "$DATA/twitter-dir-in" --name twitter-dir --threads 32 --cold 0 --out "$R/baseline_f1_twitter-dir.json" > /dev/null 2> "$R/logs/baseline_f1_twitter-dir.err"
}

# scans and random access to one list (comparable with lab C's bake-off), both decoders
stage_scan() {
  stage_start scan
  : > "$R/scan.jsonl"
  for g in $MAIN; do for t in 1 16; do
    for cfg in "raw" "comp" "comp --batch 1" "comp --batch 1 --decoder lab" "comp-block"; do
      gate; "$B" scan --graph "$DATA/$g" --store "$S/store/$g" --config $cfg --threads $t --reps 3 --json "$R/scan.jsonl" > /dev/null
    done
  done; done
}

# 2c. configuration C: cost of opening (warm and cold page cache), decode time and memory
stage_open() {
  stage_start open
  : > "$R/open.jsonl"
  local o="--json $R/open.jsonl"
  for g in $MAIN; do
    for rep in 1 2 3; do
      gate; "$B" open --graph "$DATA/$g" --what raw --mem heap --threads 16 $o > /dev/null
      gate; "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what comp --mem heap --threads 16 $o > /dev/null
      gate; "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what comp --mem mmap --threads 16 $o > /dev/null
      gate; "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what decoded --mem heap --threads 16 $o > /dev/null
      gate; "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what decoded --mem mmap --threads 16 $o > /dev/null
    done
    gate; "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what decoded --mem mmap --threads 1 $o > /dev/null
    for rep in 1 2; do   # cold: files evicted from the page cache first
      gate; "$B" open --graph "$DATA/$g" --what raw --mem heap --threads 16 --cold $o > /dev/null
      gate; "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what comp --mem heap --threads 16 --cold $o > /dev/null
      gate; "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what decoded --mem heap --threads 16 --cold $o > /dev/null
      gate; "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what decoded --mem mmap-lazy --threads 16 --cold $o > /dev/null
    done
    # leave both representations warm again
    "$B" open --graph "$DATA/$g" --what raw --mem heap --threads 16 > /dev/null
    "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what comp --mem heap --threads 16 > /dev/null
  done
}

# 2/3. kernels on lj and orkut: A raw, B compressed on the fly, C decoded at open; 1 and 16 threads, best of 3
stage_kernels() {
  stage_start kernels
  : > "$R/kernels.jsonl"
  for g in lj orkut; do for t in 16 1; do
    for cfg in raw comp decoded; do
      k $g $cfg $t pr32,pr64,wcc,bfs,bfs-ds,tc,tc-comp 3
      k $g $cfg $t kcore 5      # PKC timings vary run to run at 16 threads: more repetitions
    done
  done; done
}

# twitter-sym and Friendster. What is left out (time): see the report.
stage_kernels_big() {
  stage_start kernels_big
  : > "$R/kernels_big.jsonl"
  export OUT=kernels_big
  for cfg in raw comp; do
    k twitter-sym $cfg 16 pr32,pr64,wcc,bfs,bfs-ds,kcore 3
    k twitter-sym $cfg 16 tc,tc-comp 1
    k twitter-sym $cfg 1 wcc,bfs,bfs-ds,kcore 3
    k twitter-sym $cfg 1 pr32,pr64 1
  done
  k twitter-sym decoded 16 pr32,wcc,bfs,kcore 3
  for cfg in raw comp; do
    k friendster $cfg 16 wcc,bfs,bfs-ds,kcore 3
    k friendster $cfg 16 pr32,pr64 2
    k friendster $cfg 16 tc,tc-comp 1
    k friendster $cfg 1 wcc,bfs,bfs-ds 3
  done
  k friendster decoded 16 wcc,bfs 3
  unset OUT
}

# Leiden, 1 iteration, async and deterministic. All three configurations in anonymous memory
# (the lab loader, mmap, is measured in `baseline` and again here as raw/mmap for lj and orkut).
stage_leiden() {
  stage_start leiden
  : > "$R/leiden.jsonl"
  for g in lj orkut; do for t in 16 1; do
    for cm in "raw heap" "comp heap" "decoded heap" "raw mmap" "comp mmap"; do
      ld $g $cm $t 3
      ld $g $cm $t 3 --det
    done
  done; done
}
stage_leiden_big() {
  stage_start leiden_big
  : > "$R/leiden_big.jsonl"
  export OUT=leiden_big
  for cm in "raw heap" "comp heap"; do
    ld twitter-sym $cm 16 2
    ld twitter-sym $cm 16 1 --det
    ld friendster $cm 16 1
  done
  unset OUT
}

# point queries: warm, single thread, 100k inputs for the sub-microsecond queries, 10k for the rest
stage_pq() {
  stage_start pq
  : > "$R/pq.jsonl"
  for g in lj orkut; do
    for cm in "raw mmap" "raw heap" "comp mmap" "comp heap" "decoded heap"; do
      set -- $cm
      gate; "$B" pq --graph "$DATA/$g" --store "$S/store/$g" --config $1 --mem $2 --reps 3 --json "$R/pq.jsonl" > /dev/null 2> "$R/logs/pq_${g}_$1_$2.err"
    done
  done
  for g in twitter-sym friendster; do
    for cm in "raw mmap" "raw heap" "comp mmap" "comp heap"; do
      set -- $cm
      gate; "$B" pq --graph "$DATA/$g" --store "$S/store/$g" --config $1 --mem $2 --reps 1 --json "$R/pq.jsonl" > /dev/null 2> "$R/logs/pq_${g}_$1_$2.err"
    done
  done
  # directed twitter-2010 with its in-adjacency (what lab F1 measured: "1.5 ms on twitter-2010")
  for cm in "raw mmap" "raw heap" "comp mmap" "comp heap"; do
    set -- $cm
    gate; "$B" pq --graph "$DATA/twitter-dir" --store "$S/store/twitter-dir" --in-graph "$DATA/twitter-dir-in" --in-store "$S/store/twitter-dir-in" \
      --config $1 --mem $2 --reps 1 --json "$R/pq.jsonl" > /dev/null 2> "$R/logs/pq_twitter-dir_$1_$2.err"
  done
}

# why the slowdowns are what they are
stage_ablation() {
  stage_start ablation
  : > "$R/ablation.jsonl"
  export OUT=ablation
  # (a) the lab C decoder (scalar prefix sum in a second pass) instead of the fused one
  for t in 1 16; do k orkut comp $t pr32,pr64,bfs,kcore 3 --decoder lab --label decoder-lab; done
  k twitter-sym comp 16 pr32,bfs 3 --decoder lab --label decoder-lab
  # (b) decode granularity for sequential scans: list by list (decode and kernel body
  #     interleaved), small batches, the default (8192 entries), a whole ~64K-entry block
  for g in lj orkut; do for t in 1 16; do
    for batch in 1 256 1024 65536; do k $g comp $t pr32,pr64 3 --batch $batch --label granularity; done
    k $g comp $t tc 3 --batch 1 --label granularity
  done; done
  for batch in 1 65536; do
    k twitter-sym comp 16 pr32,pr64 3 --batch $batch --label granularity
    k friendster comp 16 pr32 2 --batch $batch --label granularity
  done
  # (c) a block-level index only (no per-node offsets): prefix scans decode whole lists, random
  #     access walks the block from its start
  for t in 1 16; do k orkut comp-block $t wcc 3 --label block-index-only; done
  k twitter-sym comp-block 16 wcc 3 --label block-index-only
  k friendster comp-block 16 wcc 3 --label block-index-only
  k orkut comp-block 16 bfs,kcore 1 --label block-index-only
  gate; "$B" pq --graph "$DATA/orkut" --store "$S/store/orkut" --config comp-block --mem heap --reps 1 --n-small 20000 --n-big 300 \
      --queries degree,neighbors,common_neighbors,two_hop_size,bidir_bfs_path --label block-index-only --json "$R/ablation.jsonl" > /dev/null 2>&1
  # (g) an Elias-Fano index of entry offsets next to the byte offsets: deg(v) without touching the data
  for g in lj orkut; do for t in 1 16; do k $g comp $t wcc,bfs,bfs-ds,pr32 3 --deg-index --label degree-index; k $g comp $t kcore 5 --deg-index --label degree-index; done; done
  for g in twitter-sym friendster; do k $g comp 16 wcc,bfs,bfs-ds 3 --deg-index --label degree-index; done
  for g in orkut twitter-sym friendster; do
    gate; "$B" pq --graph "$DATA/$g" --store "$S/store/$g" --config comp --mem heap --reps 1 --deg-index \
      --queries degree,neighbors,bidir_bfs_path --label degree-index --json "$R/ablation.jsonl" > /dev/null 2>&1
  done
  gate; "$B" pq --graph "$DATA/twitter-dir" --store "$S/store/twitter-dir" --in-graph "$DATA/twitter-dir-in" --in-store "$S/store/twitter-dir-in" --config comp --mem heap --reps 1 --deg-index \
      --queries degree,neighbors,bidir_bfs_path --label degree-index --json "$R/ablation.jsonl" > /dev/null 2>&1
  # (d) page cache (mmap, 4 KiB pages) vs anonymous memory (THP) for the same bytes
  for g in orkut twitter-sym; do for cfg in raw comp; do k $g $cfg 16 pr32,bfs,kcore 3 --mem mmap --label mem-mmap; done; done
  # (e) triangles without a derived oriented adjacency (decode the full list of v for every edge)
  for g in lj orkut; do for cfg in raw comp; do k $g $cfg 16 tc-naive 1 --label tc-naive; done; done
  for g in twitter-sym friendster; do k $g raw 16 degsq 1 --label tc-naive; done
  # (f) all hardware threads for the compute-bound kernel (triangles), as in FINDINGS
  for cfg in raw comp; do k orkut $cfg 32 tc,tc-comp 3 --label smt; done
  unset OUT
}

# planner evidence: the raw arrays do not fit the memory limit, the compressed store does.
# Files are evicted first so that their pages are charged to the limited cgroup.
stage_capped() {
  stage_start capped
  : > "$R/capped.jsonl"
  cap() { # limit graph config mem algos reps iters
    local lim=$1 g=$2 cfg=$3 mem=$4 algos=$5 reps=$6 iters=$7
    "$B" evict --graph "$DATA/$g" --store "$S/store/$g"
    gate
    systemd-run --user --scope -q -p MemoryMax=$lim -p MemorySwapMax=0 timeout 900 \
      "$B" kernel --graph "$DATA/$g" --store "$S/store/$g" --config $cfg --mem $mem --threads 16 --algos $algos --reps $reps --iters $iters \
      --label "MemoryMax=$lim" --json "$R/capped.jsonl" > /dev/null 2> "$R/logs/capped_${g}_${cfg}_$lim.err"
    echo "{\"cmd\":\"capped-exit\",\"graph\":\"$g\",\"config\":\"$cfg\",\"mem\":\"$mem\",\"limit\":\"$lim\",\"algos\":\"$algos\",\"exit_code\":$?}" >> "$R/capped.jsonl"
  }
  for spec in "8G twitter-sym" "12G friendster"; do
    set -- $spec
    cap $1 $2 raw mmap-lazy pr32,wcc,bfs 2 5
    cap $1 $2 comp mmap-lazy pr32,wcc,bfs 2 5
    cap $1 $2 decoded mmap-lazy pr32 1 5    # expected to be killed: the decoded copy is anonymous memory
  done
  for g in twitter-sym friendster; do   # warm everything again
    "$B" open --graph "$DATA/$g" --what raw --mem heap --threads 16 > /dev/null
    "$B" open --graph "$DATA/$g" --store "$S/store/$g" --what comp --mem heap --threads 16 > /dev/null
  done
}

# user-space hardware counters for whole processes (graph loading is mostly kernel time and
# therefore mostly outside these counts); repetitions are raised so the kernel dominates
stage_perf() {
  stage_start perf
  : > "$R/perf.jsonl"
  local EV=cycles:u,instructions:u,branch-misses:u,L1-dcache-load-misses:u,cache-misses:u,dTLB-load-misses:u
  p() { # label -- command...
    local label=$1; shift
    gate
    perf stat -x, -e $EV -o "$R/perf/$label.csv" -- "$@" > "$R/perf/$label.out" 2> "$R/perf/$label.err"
    python3 -I - "$label" "$R/perf/$label.csv" "$R/perf/$label.out" >> "$R/perf.jsonl" <<'PY'
import sys, json
label, csv, out = sys.argv[1:4]
ev = {}
for l in open(csv):
    f = l.strip().split(",")
    if len(f) > 3 and f[0].replace(".", "").isdigit():
        ev[f[2]] = float(f[0])
rows = [json.loads(l) for l in open(out) if l.startswith("{")]
r = rows[-1] if rows else {}
secs = r.get("seconds_all") or r.get("times_s") or []
print(json.dumps({"label": label, "counters_user_space_whole_process": ev, "kernel_seconds_all": secs,
                  "config": r.get("config"), "algo": r.get("algo", "leiden"), "graph": r.get("graph"), "threads": r.get("threads")}))
PY
  }
  for cfg in raw comp; do
    p orkut_pr32_1t_$cfg "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config $cfg --threads 1 --algos pr32 --reps 3
  done
  p orkut_pr32_1t_comp_listbylist "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config comp --threads 1 --algos pr32 --reps 3 --batch 1
  p orkut_pr32_16t_comp_listbylist "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config comp --threads 16 --algos pr32 --reps 10 --batch 1
  p orkut_bfs_1t_comp_degindex "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config comp --threads 1 --algos bfs --reps 40 --deg-index
  p orkut_pr32_1t_comp_labdecoder "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config comp --threads 1 --algos pr32 --reps 3 --decoder lab
  for cfg in raw comp; do
    p orkut_bfs_1t_$cfg "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config $cfg --threads 1 --algos bfs --reps 40
    p orkut_wcc_1t_$cfg "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config $cfg --threads 1 --algos wcc --reps 40
    p orkut_kcore_1t_$cfg "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config $cfg --threads 1 --algos kcore --reps 5
    p orkut_pr32_16t_$cfg "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config $cfg --threads 16 --algos pr32 --reps 10
    p lj_leiden_1t_$cfg "$B" leiden run --graph "$DATA/lj" --store "$S/store/lj" --config $cfg --mem heap --algo leiden --threads 1 --repeat 2 --no-eval
    p twitter_pr32_16t_$cfg "$B" kernel --graph "$DATA/twitter-sym" --store "$S/store/twitter-sym" --config $cfg --threads 16 --algos pr32 --reps 2
    p twitter_bfs_16t_$cfg "$B" kernel --graph "$DATA/twitter-sym" --store "$S/store/twitter-sym" --config $cfg --threads 16 --algos bfs --reps 20
  done
  p orkut_tc_1t_fwdraw "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config raw --threads 1 --algos tc --reps 1
  p orkut_tc_1t_fwdcomp "$B" kernel --graph "$DATA/orkut" --store "$S/store/orkut" --config raw --threads 1 --algos tc-comp --reps 1
}

stage_summarize() {
  python3 -I "$HERE/summarize.py" "$R" > "$R/summary.txt" && cat "$R/summary.txt"
}

STAGES=${*:-build encode baseline scan open kernels kernels_big leiden leiden_big pq ablation capped perf summarize}
for st in $STAGES; do
  "stage_$st" || { echo "stage $st failed"; exit 1; }
done
log "done: $STAGES"
