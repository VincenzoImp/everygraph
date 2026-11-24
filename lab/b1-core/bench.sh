#!/bin/bash
# B1 — Rust core: reproduces the headline numbers (ingest, in-RAM kernels, semi-external).
# Runs from a clean shell, no network (cargo --offline), < 20 min on an idle machine.
# Takes no lab locks itself: on a shared machine wrap it:  flock bigmem.lock flock bench.lock ./bench.sh
# (the 32 GB-budget Friendster ingest peaks at ~13 GB RSS; everything else stays below 10 GB anonymous memory).
# Output: results/bench/*.json (JSON arrays; one object per run).
#   THREADS=32 ./bench.sh            # default
#   SKIP_FRIENDSTER=1 ./bench.sh     # small graphs only (~3 min)
set -u
cd "$(dirname "$(readlink -f "$0")")"
HERE=$PWD
T=${THREADS:-32}
RAW=/home/tux/graphlab/data/raw
CSR=/home/tux/graphlab/data/csr
OUT=$HERE/results/bench
WORK=$HERE/tmp/bench
mkdir -p "$OUT" "$WORK" "$HERE/tmp"
: > "$OUT/ingest.jsonl"; : > "$OUT/kernels.jsonl"; : > "$OUT/semiext.jsonl"; : > "$OUT/validation.txt"

echo "== build (release, lto=fat, codegen-units=1, target-cpu=native)"
nice -n 10 cargo build --release --offline -j 16 2>&1 | tail -1
G=$HERE/target/release/glab
[ -x "$G" ] || { echo "build failed"; exit 1; }

echo "== plain-text inputs (decompress once)"
for name in lj orkut; do
  [ -s "$HERE/tmp/$name.txt" ] || pigz -dc "$RAW/com-$name.ungraph.txt.gz" > "$HERE/tmp/$name.txt"
done
if [ -z "${SKIP_FRIENDSTER:-}" ] && [ ! -s "$HERE/tmp/friendster.txt" ]; then
  pigz -dc "$RAW/com-friendster.ungraph.txt.gz" > "$HERE/tmp/friendster.txt"   # ~31 GB, ~100 s
fi

same() { # same DIR NAME -> byte comparison with the shared reference CSR
  local ok=1
  for f in offsets.u64 targets.u32 ids.u64; do cmp -s "$1/$f" "$CSR/$2/$f" || ok=0; done
  echo "$2 $3 identical_to_reference=$ok" | tee -a "$OUT/validation.txt"
}

echo "== ingest: lj, orkut (text -> glab-csr-v0)"
for name in lj orkut; do
  for rep in 1 2 3; do
    "$G" ingest --input "$HERE/tmp/$name.txt" --out "$WORK/$name" --tmp "$WORK/t" --mem-gb 8 --threads $T --out-mmap --label "plain-$name" --json "$OUT/ingest.jsonl" > /dev/null
  done
  same "$WORK/$name" $name plain
  "$G" ingest --input "$HERE/tmp/$name.txt" --out "$WORK/$name" --tmp "$WORK/t" --mem-gb 0.25 --threads $T --direct-in --label "plain-budget0.25GB-$name" --json "$OUT/ingest.jsonl" > /dev/null
  same "$WORK/$name" $name budget0.25GB
  "$G" ingest --input "$HERE/tmp/$name.txt" --out "$WORK/$name" --tmp "$WORK/t" --mem-gb 8 --threads 1 --label "plain-1thread-$name" --json "$OUT/ingest.jsonl" > /dev/null
  "$G" ingest --input "$HERE/tmp/$name.txt" --out "$WORK/$name" --tmp "$WORK/t" --threads $T --parse-only --label "parse-only-$name" --json "$OUT/ingest.jsonl" > /dev/null
  pigz -dc "$RAW/com-$name.ungraph.txt.gz" | "$G" ingest --input - --out "$WORK/$name" --tmp "$WORK/t" --mem-gb 8 --threads $T --label "pigz-pipe-$name" --json "$OUT/ingest.jsonl" > /dev/null
  same "$WORK/$name" $name pigz-pipe
  "$G" ingest --input "$RAW/com-$name.ungraph.txt.gz" --gz-inproc --out "$WORK/$name" --tmp "$WORK/t" --mem-gb 8 --threads $T --label "gz-inprocess-$name" --json "$OUT/ingest.jsonl" > /dev/null
  same "$WORK/$name" $name gz-inprocess
done

echo "== kernels in RAM: orkut, thread scaling"
for TT in $T 16 8 4 2 1; do
  [ "$TT" -gt "$T" ] && continue
  for algo in pr32 pr64 wcc bfs kcore tc; do
    case "$TT:$algo" in 1:tc|2:tc|1:pr64|2:pr64) continue;; esac   # keep the script short
    reps=2; [ $algo = tc ] && reps=1
    "$G" run --graph "$CSR/orkut" --algo $algo --threads $TT --load heap --reps $reps --label scale --json "$OUT/kernels.jsonl" > /dev/null
  done
done
echo "== probes: orkut PageRank (mmap vs heap, THP off, u64 ids, node order)"
for algo in pr32 pr64; do
  "$G" run --graph "$CSR/orkut" --algo $algo --threads $T --load mmap --reps 3 --label probe --json "$OUT/kernels.jsonl" > /dev/null
  "$G" run --graph "$CSR/orkut" --algo $algo --threads $T --load heap --thp off --reps 3 --label probe --json "$OUT/kernels.jsonl" > /dev/null
  "$G" run --graph "$CSR/orkut" --algo $algo --threads $T --load heap --ids u64 --reps 3 --label probe --json "$OUT/kernels.jsonl" > /dev/null
  for order in degree bfs random; do
    "$G" run --graph "$CSR/orkut" --algo $algo --threads $T --load heap --order $order --reps 3 --label probe --json "$OUT/kernels.jsonl" > /dev/null
  done
done

if [ -z "${SKIP_FRIENDSTER:-}" ]; then
  echo "== ingest: friendster (1.8 B edges, 32 GB text), 8 GB budget"
  # 32 GB budget (peak RSS ~13 GB: the spill stays in RAM, buckets go through the page cache). Lab result: 20.1 s
  "$G" ingest --input "$HERE/tmp/friendster.txt" --out "$WORK/friendster" --tmp "$WORK/t" --mem-gb 32 --threads $T --direct-in --out-mmap --label "friendster-budget32GB" --json "$OUT/ingest.jsonl" > /dev/null
  # 8 GB budget (peak RSS ~3.6 GB, 40 GB temp disk). Lab result: 33-45 s on a loaded machine
  "$G" ingest --input "$HERE/tmp/friendster.txt" --out "$WORK/friendster" --tmp "$WORK/t" --mem-gb 8 --threads $T --direct-in --out-mmap --label "friendster-budget8GB" --json "$OUT/ingest.jsonl" > /dev/null
  if [ -d "$CSR/friendster" ]; then same "$WORK/friendster" friendster budget8GB; fi
  "$G" verify --graph "$WORK/friendster" --threads $T | tee -a "$OUT/validation.txt" > /dev/null
  FR=$WORK/friendster
  echo "== kernels: friendster, CSR memory-mapped (warm)"
  "$G" run --graph "$FR" --algo wcc --threads $T --load mmap --reps 3 --label friendster-mmap --json "$OUT/kernels.jsonl" > /dev/null
  "$G" run --graph "$FR" --algo bfs --threads $T --load mmap --reps 3 --label friendster-mmap --json "$OUT/kernels.jsonl" > /dev/null
  "$G" run --graph "$FR" --algo pr32 --threads $T --load mmap --iters 20 --label friendster-mmap --json "$OUT/kernels.jsonl" > /dev/null
  echo "== larger than RAM: friendster under hard cgroup limits (cold page cache)"
  cg() { local lim=$1 label=$2; shift 2
    "$G" evict --graph "$FR"
    systemd-run --user --scope -q -p MemoryMax=$lim -p MemorySwapMax=0 timeout 240 "$@" --label "$label" --json "$OUT/semiext.jsonl" > /dev/null
    echo "{\"cmd\":\"ooc-exit\",\"limit\":\"$lim\",\"label\":\"$label\",\"exit_code\":$?}" >> "$OUT/semiext.jsonl"; }
  "$G" semiext --graph "$FR" --algo pr32 --io direct --threads $T --iters 10 --label nolimit-semiext-direct --json "$OUT/semiext.jsonl" > /dev/null
  cg 8G lim8G-semiext-direct-pr32 "$G" semiext --graph "$FR" --algo pr32 --io direct --threads $T --iters 10
  cg 8G lim8G-semiext-direct-wcc  "$G" semiext --graph "$FR" --algo wcc --io direct --threads $T
  cg 2G lim2G-semiext-direct-pr32 "$G" semiext --graph "$FR" --algo pr32 --io direct --threads $T --iters 5 --chunk-mb 4
  cg 8G lim8G-mmap-naive-pr32     "$G" run --graph "$FR" --algo pr32 --load mmap --threads $T --iters 2
  cg 16G lim16G-mmap-naive-pr32   "$G" run --graph "$FR" --algo pr32 --load mmap --threads $T --iters 5
fi

# jsonl -> json arrays
for f in ingest kernels semiext; do
  python3 -I - "$OUT/$f.jsonl" "$OUT/$f.json" <<'PY'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip().startswith("{")]
json.dump(rows, open(sys.argv[2], "w"), indent=1)
print(sys.argv[2], len(rows), "records")
PY
done
echo "== summary"
python3 -I "$HERE/scripts/summary.py" "$OUT"
cat "$OUT/validation.txt"
