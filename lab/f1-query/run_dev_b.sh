#!/bin/bash
cd "$(dirname "$0")"
./run_dict.sh 4000000 65000000
RAYON_NUM_THREADS=4 taskset -c 14-15,30-31 ./target/release/props --graph work/csr/orkut --work work/props_orkut --threads 4 --reps 3 --out results/props_orkut_t4.json > /dev/null 2> logs/props_orkut_t4.err
