#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# The fuzz targets (issue #191; CONTRIBUTING.md, "Fuzzing"), as CI runs them.
#
#   tools/fuzz.sh list                    the targets, as CRATE/TARGET
#   tools/fuzz.sh replay [TARGET...]      run each corpus once (seconds; every PR)
#   tools/fuzz.sh run TARGET SECONDS      fuzz TARGET, adding what it finds to its corpus
#   tools/fuzz.sh cmin TARGET             shrink TARGET's corpus to the inputs that matter
#
# TARGET is a name from `list` (adjacency, container, text). The toolchain is the
# pinned nightly of tools/nightly-toolchain unless FUZZ_TOOLCHAIN says otherwise.
# Every run refuses a single allocation above 64 MiB, so that a size an input declares
# (a zstd window) never turns into memory (docs/research/sota-testing.md, challenge 10).

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
toolchain="${FUZZ_TOOLCHAIN:-$(cat "$root/tools/nightly-toolchain")}"
targets=(everygraph-format/adjacency everygraph-format/container everygraph-ingest/text)
limits=(-malloc_limit_mb=64 -rss_limit_mb=2048 -max_len=8192)

crate_of() {
    for t in "${targets[@]}"; do
        if [ "${t#*/}" = "$1" ]; then
            echo "${t%/*}"
            return
        fi
    done
    echo "unknown fuzz target: $1 (tools/fuzz.sh list)" >&2
    exit 2
}

fuzz() {
    local target="$1"
    shift
    local dir
    dir="$root/crates/$(crate_of "$target")"
    mkdir -p "$dir/fuzz/corpus/$target"
    (cd "$dir" && cargo "+$toolchain" fuzz "$@")
}

case "${1:-}" in
    list)
        printf '%s\n' "${targets[@]}"
        ;;
    replay)
        shift
        names=("$@")
        if [ ${#names[@]} -eq 0 ]; then
            for t in "${targets[@]}"; do names+=("${t#*/}"); done
        fi
        for target in "${names[@]}"; do
            echo "== replay $target"
            fuzz "$target" run "$target" "fuzz/corpus/$target" -- -runs=0 "${limits[@]}"
        done
        ;;
    run)
        [ $# -eq 3 ] || { echo "usage: tools/fuzz.sh run TARGET SECONDS" >&2; exit 2; }
        fuzz "$2" run "$2" "fuzz/corpus/$2" -- -max_total_time="$3" "${limits[@]}"
        ;;
    cmin)
        [ $# -eq 2 ] || { echo "usage: tools/fuzz.sh cmin TARGET" >&2; exit 2; }
        fuzz "$2" cmin "$2" "fuzz/corpus/$2" -- "${limits[@]}"
        ;;
    *)
        sed -n '4,14p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
