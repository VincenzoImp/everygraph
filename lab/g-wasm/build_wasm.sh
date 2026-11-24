#!/bin/bash
# Builds all wasm variants into web/pkg-<variant>/ . Uses the private toolchain in tools/.
set -euo pipefail
cd "$(dirname "$0")"
export RUSTUP_HOME=$PWD/tools/rustup CARGO_HOME=$PWD/tools/cargo PATH=$PWD/tools/cargo/bin:$PWD/tools/binaryen/binaryen-version_133/bin:$PATH
STABLE=1.96.1
NIGHTLY=${NIGHTLY:-nightly}   # tested: nightly 2026-11-23 (rustc 1.102.0-nightly 20fe935d5)
J="nice -n 10 taskset -c 12-13,28-29"
MEM4G="-C link-arg=--max-memory=4294967296"
build() { # name toolchain target rustflags extra-cargo-args features
  local name=$1 tc=$2 tgt=$3 rf=$4 extra=$5 feat=$6
  echo "== $name"
  RUSTFLAGS="$rf" CARGO_TARGET_DIR=$PWD/target/w-$name $J cargo +$tc build --release -j 6 -p gwasm --target $tgt $extra $feat
  local out=$PWD/target/w-$name/$tgt/release/gwasm.wasm
  mkdir -p web/pkg-$name
  if [[ "$feat" == *threads* ]]; then
    wasm-bindgen --target web --out-dir web/pkg-$name $out
    wasm-opt -O3 --enable-threads --enable-bulk-memory --enable-mutable-globals --enable-nontrapping-float-to-int --enable-sign-ext $( [[ "$rf" == *simd128* ]] && echo --enable-simd ) web/pkg-$name/gwasm_bg.wasm -o web/pkg-$name/gwasm_bg.wasm
  else
    local feats="--enable-bulk-memory --enable-mutable-globals --enable-nontrapping-float-to-int --enable-sign-ext --enable-multivalue --enable-reference-types"
    [[ "$rf" == *simd128* ]] && feats="$feats --enable-simd"
    [[ "$tgt" == wasm64* ]] && feats="$feats --enable-memory64"
    wasm-opt -O3 $feats $out -o web/pkg-$name/gwasm.wasm
  fi
  ls -la web/pkg-$name/*.wasm
}
build st     $STABLE  wasm32-unknown-unknown "$MEM4G" "" ""
build st-simd $STABLE wasm32-unknown-unknown "$MEM4G -C target-feature=+simd128" "" ""
TF="-C target-feature=+atomics,+bulk-memory,+mutable-globals -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--export=__heap_base -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base $MEM4G"
build mt     $NIGHTLY wasm32-unknown-unknown "$TF" "-Z build-std=panic_abort,std" "--features threads"
build mt-simd $NIGHTLY wasm32-unknown-unknown "$TF -C target-feature=+simd128" "-Z build-std=panic_abort,std" "--features threads"
# wasm64 (memory64): tier-3 target, nightly + build-std. 16 GiB max (V8 cap).
build w64    $NIGHTLY wasm64-unknown-unknown "-C link-arg=--max-memory=17179869184" "-Z build-std=panic_abort,std" ""
build w64-simd $NIGHTLY wasm64-unknown-unknown "-C link-arg=--max-memory=17179869184 -C target-feature=+simd128" "-Z build-std=panic_abort,std" ""
