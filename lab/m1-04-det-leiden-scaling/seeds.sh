#!/usr/bin/env bash
# Quality spread of each deterministic rule over "schedule seeds": --bucket-shift k groups vertices
# u with the same u >> k into one level-0 sub-round bucket (default k = 4, the A/B's value), so each k
# is a different deterministic schedule of the same rule. Modularity only (timings not used).
# Every run under MemoryMax=8G via counts.py. ~15 minutes.
set -euo pipefail
cd "$(dirname "$0")"
OUT=${OUT:-results/seeds-lj-orkut.jsonl}
for k in 0 1 2 3 5 6; do
  python3 counts.py --threads 16 --variants auto,s32,s64,L16k,L64k --extra "--bucket-shift $k" --tag "shift$k" --out "$OUT"
done
