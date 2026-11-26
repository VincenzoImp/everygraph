#!/bin/sh
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Builds the C++ reference tools of bench/specs/references.toml from pinned sources, in
# one flavour: `generic` (no -march=native, as the Everygraph workspace builds) or
# `native` (-march=native for every tool). Each tool keeps its own optimization flags
# otherwise. Sources are downloaded once to REFS/src (the network is needed then).
#
#   build.sh generic|native REFS REPO
#
# Result: REFS/FLAVOUR/{gapbs,gbbs,pkc,pasgal,gve}, each with a COMMIT file.
# Parallel-KCore has no licence: it is downloaded and built here, never committed.
set -eu
flavour=$1 refs=$2 repo=$3
case $flavour in
  generic) arch= ;;
  native) arch=-march=native ;;
  *) echo "flavour: generic or native" >&2; exit 2 ;;
esac
# github.com/sbeamer/gapbs (BSD-3-Clause), the newest commit on 2026-11-26.
gap=941462c7c42e786e4e006a9211b0576ef78d23f7
# github.com/ldhulipala/gbbs (MIT): the last commit with makefiles, and the parlaylib
# commit its submodule pins. ConnectIt is in this tree.
gbbs=b1a90f5fa3b36b8be9fa5c336e3dc5783ae75d76 gbbs_parlay=6b4a4cdbfeb3c481608a42db0230eb6ebb87bf8d
# github.com/ucrparlay/Parallel-KCore (no licence) and github.com/ucrparlay/PASGAL (MIT),
# newest on 2026-11-26, with the parlaylib commits their submodules pin.
pkc=b4966c8ffc48f6e0de8c6a50ffb626af37a91bb8 pkc_parlay=e1f1dc0ccf930492a2723f7fbef8510d35bf57f5
pasgal=f7817baf0826803bfee157bd25fcbc1768387c4a pasgal_parlay=c655b8f3c127bd3c93918d1548db0ca5989cffa0
# github.com/puzzlef/leiden-communities-openmp (MIT), as bench/specs/baselines.toml.
gve=f741a40717a1b207d53289c5cdda36ce858477a7

# A build is redone when its pinned commit or the source of its driver changes.
src_id() {
  cat "$@" | sha256sum | cut -c1-12
}
gap_id="$gap (drivers $(src_id "$repo/bench/references/gap_dump.cc" "$repo/bench/references/gap_sources.cc"))"
gbbs_id="$gbbs (parlaylib $gbbs_parlay; PageRank.h return patched; driver $(src_id "$repo/bench/references/gbbs_driver.cc"))"
ucr_src=$(src_id "$repo/bench/references/ucr_driver.cc")

fetch() {  # fetch OWNER/REPO COMMIT: the tree at REFS/src/REPO-COMMIT
  d=$refs/src/${1#*/}-$2
  if [ ! -d "$d" ]; then
    mkdir -p "$d.part"
    curl -sSfL "https://codeload.github.com/$1/tar.gz/$2" | tar -xz -C "$d.part" --strip-components=1
    mv "$d.part" "$d"
  fi
  echo "$d"
}

fresh() {  # fresh TOOL: an empty REFS/FLAVOUR/TOOL.part, unless TOOL is built for this commit
  out=$refs/$flavour/$1
  [ "$(cat "$out/COMMIT" 2>/dev/null)" = "$2" ] && return 1
  rm -rf "$out" "$out.part"
  mkdir -p "$out.part"
}

done_() {  # done_ TOOL COMMIT
  echo "$2" > "$refs/$flavour/$1.part/COMMIT"
  mv "$refs/$flavour/$1.part" "$refs/$flavour/$1"
}

# --- GAP: its own kernels; PageRank with ScoreT = double; SSSP also with WeightT = double;
# the dump drivers and the source list of bench/references.
if fresh gapbs "$gap_id"; then
  src=$(fetch sbeamer/gapbs $gap)
  o=$refs/$flavour/gapbs.part
  cp -r "$src/." "$o/"
  sed 's/^typedef float ScoreT;$/typedef double ScoreT;/' "$o/src/pr_spmv.cc" > "$o/src/pr_spmv64.cc"
  grep -q '^typedef double ScoreT;$' "$o/src/pr_spmv64.cc"
  flags="-std=c++11 -O3 -Wall -fopenmp $arch"
  make -s -C "$o" CXX_FLAGS="$flags" bfs cc sssp tc pr_spmv64
  # WeightT = double: the generator's weight draw needs an integer type, and the reader
  # refuses non-int32 .wsg files; the references spec writes them with double weights.
  cp -r "$o/src" "$o/src64"
  sed -i 's/^typedef int32_t WeightT;$/typedef double WeightT;/' "$o/src64/benchmark.h"
  sed -i 's/UniDist<WeightT_, rng_t_> udist(254, rng);/UniDist<NodeID_, rng_t_> udist(254, rng);/' "$o/src64/generator.h"
  sed -i 's/    if (weighted \&\& !std::is_same<WeightT_, SGID>::value) {/    if (false) {/' "$o/src64/reader.h"
  grep -q '^typedef double WeightT;$' "$o/src64/benchmark.h"
  g++ $flags "$o/src64/sssp.cc" -o "$o/sssp_f64"
  for k in bfs:bfs cc:cc sssp:sssp pr:pr_spmv64; do
    g++ $flags -I"$o/src" "-DKERNEL=\"${k#*:}.cc\"" "-DGAP_$(echo "${k%%:*}" | tr a-z A-Z)" \
      "$repo/bench/references/gap_dump.cc" -o "$o/dump_${k%%:*}"
  done
  g++ $flags -I"$o/src64" '-DKERNEL="sssp.cc"' -DGAP_SSSP "$repo/bench/references/gap_dump.cc" -o "$o/dump_sssp_f64"
  g++ $flags -I"$o/src" "$repo/bench/references/gap_sources.cc" -o "$o/gap_sources"
  done_ gapbs "$gap_id"
fi

# --- GBBS and ConnectIt, through bench/references/gbbs_driver.cc. GBBS's makefile adds
# -march=native; the generic flavour removes it. One line of PageRank.h is patched (below).
if fresh gbbs "$gbbs_id"; then
  o=$refs/$flavour/gbbs.part
  cp -r "$(fetch ldhulipala/gbbs $gbbs)/." "$o/"
  rm -rf "$o/external/parlaylib"
  cp -r "$(fetch cmuparlay/parlaylib $gbbs_parlay)" "$o/external/parlaylib"
  [ -z "$arch" ] && sed -i 's/-march=native//' "$o/makefile.variables"
  # PageRank() zeroes p_curr while it sums the change and then swaps, so when the loop ends
  # at the iteration cap the result is in p_curr, not in the p_next it returns (zeros).
  # The patch returns the right buffer; the timed iterations do not change.
  sed -i '/^sequence<double> PageRank(Graph& G/,/^}/ s/^  return p_next;$/  return iter > max_iters ? p_curr : p_next;/' \
    "$o/benchmarks/PageRank/PageRank.h"
  grep -q 'return iter > max_iters ? p_curr : p_next;' "$o/benchmarks/PageRank/PageRank.h"
  cp "$repo/bench/references/gbbs_driver.cc" "$o/benchmarks/Connectivity/WorkEfficientSDB14/gbbs_driver.cc"
  make -s -C "$o/gbbs"
  make -s -C "$o/gbbs/encodings"
  make -s -C "$o/benchmarks/Connectivity/WorkEfficientSDB14" ROOTDIR="$o/" gbbs_driver
  done_ gbbs "$gbbs_id"
fi

# --- Parallel-KCore and PASGAL, through bench/references/ucr_driver.cc, with their own
# flags (-O3 -mcx16; -march=native in their makefiles, here by flavour).
ucr() {  # ucr TOOL REPO COMMIT PARLAY DEFINE INCLUDES...
  tool=$1 repo_name=$2 commit=$3 parlay=$4 define=$5
  shift 5
  if fresh "$tool" "$commit (parlaylib $parlay; driver $ucr_src)"; then
    t=$(fetch "$repo_name" "$commit")
    p=$(fetch cmuparlay/parlaylib "$parlay")
    inc="-I$p/include"
    for d in "$@"; do inc="$inc -I$t/$d"; done
    g++ -std=c++20 -O3 -mcx16 $arch -pthread $inc "$define" "$repo/bench/references/ucr_driver.cc" \
      -o "$refs/$flavour/$tool.part/ucr_driver"
    done_ "$tool" "$commit (parlaylib $parlay; driver $ucr_src)"
  fi
}
ucr pkc ucrparlay/Parallel-KCore $pkc $pkc_parlay -DUCR_KCORE . KCore
ucr pasgal ucrparlay/PASGAL $pasgal $pasgal_parlay -DUCR_PASGAL src external

# --- GVE-Leiden as its main.sh builds it, in f32 (its default TYPE=float) and f64, one
# binary per thread count (MAX_THREADS is a compile-time constant), REPEAT_METHOD=1.
if fresh gve $gve; then
  s=$(fetch puzzlef/leiden-communities-openmp $gve)
  for type in float:f32 double:f64; do
    for t in 1 16; do
      g++ "-DTYPE=${type%%:*}" -DMAX_THREADS=$t -DREPEAT_METHOD=1 -std=c++17 -O3 $arch -fopenmp \
        "$s/main.cxx" -o "$refs/$flavour/gve.part/leiden-${type#*:}-t$t"
    done
  done
  done_ gve $gve
fi
