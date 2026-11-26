#!/usr/bin/env bash
# LLP orders and BV sizes through the webgraph-rs CLI (webgraph-cli 0.5.0, Apache-2.0 OR MIT).
#   ./webgraph.sh llp <graph> <MemoryMax>            -> $S/perm/<graph>/llp.perm, results/llp.jsonl
#   ./webgraph.sh bv <graph> <MemoryMax> "<orders>"  -> results/bv.jsonl (BV bits per arc, default flags)
# Install: cargo install webgraph-cli --version 0.5.0 --locked --root $S/wg
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-09-hierarchy-order}
D=${EVERYGRAPH_DATA:-/home/tux/graphlab/data}
B=${M109:-$S/target/release/m109}
W=${WEBGRAPH:-$S/wg/bin/webgraph}
cmd=$1; g=$2; lim=$3
P=$S/perm/$g; BV=$S/bv/$g
mkdir -p "$P" "$BV" results
meta() { python3 -c "import json;print(json.load(open('$D/csr/$g/meta.json'))['$1'])"; }
n=$(meta n); m=$(meta entries)
calc() { python3 -c "print(round($1, 4))"; }
lim() { systemd-run --user --scope -q -p MemoryMax="$lim" -p MemorySwapMax=0 "$@"; }
# BV graph of <graph> under order $1 (identity: the input ids) at $BV/$1
bv() {
  local perm=""; [ "$1" = identity ] || perm="--perm $P/$1.perm"
  lim bash -c "'$B' arcs --graph '$D/csr/$g' $perm 2>/dev/null | '$W' from arcs --num-nodes $n --num-arcs $m '$BV/$1'" > "$BV/$1.log" 2>&1
}
case $cmd in
  llp)
    [ -e "$BV/identity.graph" ] || bv identity
    lim "$W" build ef "$BV/identity" > "$BV/ef.log" 2>&1
    lim "$W" build dcf "$BV/identity" > "$BV/dcf.log" 2>&1
    t0=$(date +%s.%N)
    mkdir -p "$S/tmp"
    TMPDIR=$S/tmp lim "$W" run llp -t 16 -s 1 --fmt java "$BV/identity" "$BV/llp.java" > "$BV/llp.log" 2>&1
    t1=$(date +%s.%N)
    "$B" frombin --perm "$BV/llp.java" --out "$P/llp.perm" 2>/dev/null
    echo "{\"graph\": \"$g\", \"name\": \"llp\", \"threads\": 16, \"order_s\": $(calc "$t1 - $t0"), \"tool\": \"webgraph-cli 0.5.0 run llp -t 16 -s 1 (default gammas)\"}" >> results/orders.jsonl
    ;;
  bv)
    for o in $4; do
      [ -e "$BV/$o.graph" ] || bv "$o"
      bytes=$(stat -c %s "$BV/$o.graph")
      echo "{\"graph\": \"$g\", \"order\": \"$o\", \"bv_bytes\": $bytes, \"entries\": $m, \"bv_bits_per_arc\": $(calc "8 * $bytes / $m")}" >> results/bv.jsonl
      [ "$o" = identity ] || rm -f "$BV/$o".*
    done
    ;;
esac
