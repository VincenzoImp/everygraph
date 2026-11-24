#!/bin/bash
# F1 headline numbers. Re-runnable from a clean shell, no network, < 20 min on an idle machine.
# Writes results/bench_*.json. Heavy steps take the lab bench lock (whole machine).
# FULL=1 additionally runs every dictionary method/key-kind at 65M keys (~+15 min).
set -euo pipefail
cd "$(dirname "$0")"
export CARGO_TARGET_DIR=$PWD/target
nice -n 10 cargo build --release --offline -j 6 2>&1 | tail -1
mkdir -p results logs work/csr
L="flock /home/tux/graphlab/bench.lock"
T=32
# private CSR copies: page-cache eviction (fadvise DONTNEED) does not work on pages other processes keep mapped
for g in lj orkut; do
  [ -f work/csr/$g/targets.u32 ] || { mkdir -p work/csr/$g; cp /home/tux/graphlab/data/csr/$g/{offsets.u64,targets.u32,meta.json} work/csr/$g/; }
done

echo "== Q1 ID dictionary"
for kind in eth user; do
  for m in hash mphf fst sorted fc; do
    $L env RAYON_NUM_THREADS=$T ./target/release/dict --method $m --kind $kind --n 4000000 --dir work --threads $T \
       --out results/bench_dict_${m}_${kind}_4M.json > /dev/null 2> logs/bench_dict_${m}_${kind}_4M.err
  done
done
M65="hash mphf fc fst"; K65="eth"
[ "${FULL:-0}" = 1 ] && { M65="hash mphf fst sorted fc"; K65="eth user"; }
for kind in $K65; do
  for m in $M65; do
    $L env RAYON_NUM_THREADS=$T ./target/release/dict --method $m --kind $kind --n 65000000 --dir work --threads $T \
       --out results/bench_dict_${m}_${kind}_65M.json > /dev/null 2> logs/bench_dict_${m}_${kind}_65M.err
  done
done
$L env RAYON_NUM_THREADS=$T ./target/release/dict --method dedupe --kind eth --n 65000000 --dir work --threads $T \
   --out results/bench_dict_dedupe_eth_65M.json > /dev/null 2> logs/bench_dedupe.err
# raw-binary Ethereum keys in an FST (size check)
$L env RAYON_NUM_THREADS=$T ./target/release/dict --method fst --kind ethbin --n 4000000 --dir work --threads $T \
   --out results/bench_dict_fst_ethbin_4M.json > /dev/null 2> logs/bench_dict_fst_ethbin.err

echo "== Q2 point queries"
for g in orkut lj; do
  $L ./target/release/pq --graph work/csr/$g --name $g --threads $T --cold 0 --out results/bench_pq_${g}_warm.json > /dev/null 2> logs/bench_pq_${g}_warm.err
  $L ./target/release/pq --graph work/csr/$g --name $g --threads $T --warm 0 --prefetch 1 --random 1 --out results/bench_pq_${g}_cold_pf_rand.json > /dev/null 2> logs/bench_pq_${g}_cold.err
  $L ./target/release/pq --graph work/csr/$g --name $g --threads $T --warm 0 --prefetch 1 --random 0 --out results/bench_pq_${g}_cold_pf_norm.json > /dev/null 2> logs/bench_pq_${g}_cold2.err
done
$L ./target/release/pq --transpose work/csr/orkut --threads $T --out results/bench_csc_orkut_oriented.json > /dev/null 2> logs/bench_csc.err

echo "== Q3/Q4 properties, filtered views, temporal layouts (orkut)"
$L ./target/release/props --graph work/csr/orkut --work work/props_orkut --threads $T --reps 3 --out results/bench_props_orkut_t32.json > /dev/null 2> logs/bench_props.err

python3 tools/summ_dict.py "results/bench_dict_*.json" > results/bench_dict_table.md
python3 tools/summ_pq.py "results/bench_pq_*.json" > results/bench_pq_table.md
python3 tools/summ_props.py results/bench_props_orkut_t32.json > results/bench_props_table.md
echo "done; see results/bench_*.json"
