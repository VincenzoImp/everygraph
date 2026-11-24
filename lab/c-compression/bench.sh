#!/bin/bash
# Experiment C — topology storage & compression. Reproduces the headline numbers.
# Writes results/{bakeoff,cold,cold8g,order}.jsonl, results/idwidth.json, results/summary.json.
# No network needed: inputs are data/csr/{orkut,twitter-sym}, the LAW twitter BV in data/raw,
# and dl/uk-2005 (downloaded once into this dir). Encodings are cached in enc/ and ord/ (rebuilt if missing).
# Official runs take bench.lock (32 threads); twitter-sized runs also take bigmem.lock.
# Expected wall time on an idle machine: ~15-20 min (most of it twitter-sym).
set -uo pipefail
cd "$(dirname "$0")"
D=$PWD
export PATH=$D/tools/bin:$PATH
DATA=/home/tux/graphlab/data/csr
(cd bench && cargo build --release --offline -j 6 2>&1 | tail -1) && cp target/release/glc glc.new && mv glc.new glc
mkdir -p results/logs enc ord
R=results
: > $R/bakeoff.jsonl; : > $R/cold.jsonl; : > $R/cold8g.jsonl; : > $R/order.jsonl

# ---- inputs / encodings (cached)
if [ ! -f g/twitter/tw.ef ]; then
  mkdir -p g/twitter; ln -sf /home/tux/graphlab/data/raw/twitter-2010.graph g/twitter/tw.graph
  ln -sf /home/tux/graphlab/data/raw/twitter-2010.properties g/twitter/tw.properties
  (cd g/twitter && webgraph build offsets tw && webgraph build ef tw) >/dev/null 2>&1
fi
if [ ! -d g/uk-csr ]; then
  mkdir -p g/uk; ln -sf $D/dl/uk-2005/uk-2005.graph g/uk/uk.graph; cp dl/uk-2005/uk-2005.properties g/uk/uk.properties
  (cd g/uk && webgraph build offsets uk && webgraph build ef uk) >/dev/null 2>&1
  RAYON_NUM_THREADS=8 ./glc bv2csr g/uk/uk g/uk-csr
fi
./encode_all.sh $DATA/orkut enc/orkut 16 > $R/logs/enc_orkut.log 2>&1
./encode_all.sh $D/g/uk-csr enc/uk 16 > $R/logs/enc_uk.log 2>&1
flock /home/tux/graphlab/bigmem.lock ./encode_all.sh $DATA/twitter-sym enc/twsym 16 > $R/logs/enc_twsym.log 2>&1

# ---- 1. webgraph-rs on the original LAW twitter-2010 (directed): CLI bench (static dispatch, 1 thread)
flock /home/tux/graphlab/bench.lock webgraph bench bvgraph -S -R 2 g/twitter/tw > $R/logs/tw_cli_seq.log 2>&1
flock /home/tux/graphlab/bench.lock webgraph bench bvgraph -S -R 2 -r 1000000 g/twitter/tw > $R/logs/tw_cli_rand.log 2>&1
# and through our harness (parallel scan / PR / random) on twitter-dir
BIG=1 ./run_bakeoff.sh twdir $DATA/twitter-dir g/twitter $R/bakeoff.jsonl warm 200000 twbv

# ---- 2. codec bake-off (warm, 32 threads + 1 thread)
./run_bakeoff.sh orkut $DATA/orkut enc/orkut $R/bakeoff.jsonl warm 200000 raw varint svb bp zstd3 lz4 vzstd3 bv7 bv0 bvd7
./run_bakeoff.sh uk $D/g/uk-csr enc/uk $R/bakeoff.jsonl warm 200000 raw varint svb bp zstd3 lz4 vzstd3 bv7 bv0
BIG=1 ./run_bakeoff.sh twsym $DATA/twitter-sym enc/twsym $R/bakeoff.jsonl warm 200000 raw varint svb bp vzstd3 bv7 bv0

# ---- 3. cold-cache scans (posix_fadvise DONTNEED on our own copies, then 32-thread scan with per-chunk MADV_WILLNEED)
for f in enc/orkut/raw.bin enc/twsym/raw.bin; do flock /home/tux/graphlab/bench.lock ./glc readbw $f 32 >> $R/readbw.jsonl; done
./run_bakeoff.sh orkut $DATA/orkut enc/orkut $R/cold.jsonl coldonly 1000 raw varint svb bp zstd3 lz4 vzstd3 bv7 bv0
BIG=1 ./run_bakeoff.sh twsym $DATA/twitter-sym enc/twsym $R/cold.jsonl coldonly 1000 raw varint svb bp zstd3 lz4 vzstd3 bv7 bv0
# same under an 8 GB hard memory limit (raw twitter-sym = 9.6 GB does not fit, compressed forms do)
PREFIX="systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0" BIG=1 \
  ./run_bakeoff.sh twsym8g $DATA/twitter-sym enc/twsym $R/cold8g.jsonl coldonly 1000 raw varint svb bp vzstd3 bv7 bv0

# ---- 4. node ordering (orkut): perms + encodings cached in ord/orkut (LLP via webgraph CLI if ord/orkut/llp.perm exists)
./order_all.sh $DATA/orkut ord/orkut orkut 16 > $R/logs/order_build.log 2>&1
flock /home/tux/graphlab/bench.lock ./run_order.sh $DATA/orkut enc/orkut ord/orkut 32 $R/order.jsonl

# ---- 5. id width
./glc idwidth $DATA/orkut > $R/idwidth.json

python3 -I summarize.py $R > $R/summary.txt
cat $R/summary.txt
