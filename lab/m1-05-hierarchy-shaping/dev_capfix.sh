#!/usr/bin/env bash
# Exploratory (shared machine; numbers not quoted as timings): the patched m102 keeps the plain
# run's hash, and the capped refinement fixes `--cap` with --iterations 2 (ADR 0016's known bug).
#   ./dev_capfix.sh > results/capfix_lj.jsonl
set -u
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
D=/home/tux/graphlab/data
M=../m1-02-ooc-leiden/target/release/m102
mkdir -p "$S"
run() { ./lim.sh 8G $M leiden run --config comp --store $D/store-m113/lj --mem mmap --threads 16 --det "$@" 2>/dev/null | tail -1; }
for it in 1 2; do
  run --cap 64 --max-passes 20 --min-ratio 3 --iterations $it > "$S/capfix_it$it.json"
done
run > "$S/capfix_plain.json"
python3 -I - "$S" <<'EOF'
import json, sys
S = sys.argv[1]
for f in ["capfix_plain", "capfix_it1", "capfix_it2"]:
    d = json.loads(open(f"{S}/{f}.json").read())
    print(json.dumps({"run": f, "graph": "lj", "det": True, "threads": 16, "cap": d["cap"], "iterations": d["iterations"],
                      "hash_levels": d["hash_levels"], "modularity": d["modularity"], "communities": d["communities"],
                      "level_sizes": d["level_sizes"], "fanout_max": [x["max"] for x in d["fanout"]],
                      "fanout_p50": [x["p50"] for x in d["fanout"]]}))
EOF
