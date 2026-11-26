#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Mutation testing with cargo-mutants (issue #193; CONTRIBUTING.md, "Mutation testing"),
# as CI runs it. The options shared by every run are in .cargo/mutants.toml.
#
#   tools/mutants.sh diff BASE             write the product-code diff since BASE to
#                                          target/mutants/pr.diff; print its mutant count
#   tools/mutants.sh pr SHARD              test the mutants of that diff (SHARD is k/n)
#   tools/mutants.sh crate PACKAGE [SHARD] test every mutant of PACKAGE (milestone audit)
#   tools/mutants.sh report [TITLE]        Markdown summary of the last run, for a PR
#
# Each run tests only the mutated crate's own tests. Under CI (CI=true) it mutates the
# checkout in place, so that the cached target directory is used; elsewhere it works on
# a copy. Exit status of `pr` and `crate`: that of cargo-mutants (2: missed mutants,
# 3: timeouts; CONTRIBUTING.md says which a job treats as failures).

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$root/target/mutants"
out="$work/mutants.out"

# The product code a pull request's mutants come from. In the core only the memory
# accounting: the rest of the core is checked by Miri, ThreadSanitizer and i686 runs.
scope=(
    crates/everygraph-core/src/memory.rs
    crates/everygraph-core/src/memory/
    crates/everygraph-format/src/
    crates/everygraph-kernels/src/
    crates/everygraph-ingest/src/
    crates/everygraph-community/src/
)

mutants() {
    local place=()
    [ "${CI:-}" = true ] && place=(--in-place)
    mkdir -p "$work"
    (cd "$root" && cargo mutants "${place[@]}" --output "$work" "$@")
}

count() {
    python3 -c 'import json, sys; print(len(json.load(open(sys.argv[1]))))' "$1"
}

case "${1:-}" in
    diff)
        [ $# -eq 2 ] || { echo "usage: tools/mutants.sh diff BASE" >&2; exit 2; }
        mkdir -p "$work"
        (cd "$root" && git diff "$2" HEAD -- "${scope[@]}") > "$work/pr.diff"
        if [ -s "$work/pr.diff" ]; then
            (cd "$root" && cargo mutants --list --workspace --in-diff "$work/pr.diff") | wc -l
        else
            echo 0
        fi
        ;;
    pr)
        [ $# -eq 2 ] || { echo "usage: tools/mutants.sh pr k/n" >&2; exit 2; }
        mutants --workspace --in-diff "$work/pr.diff" --shard "$2"
        ;;
    crate)
        [ $# -eq 2 ] || [ $# -eq 3 ] || { echo "usage: tools/mutants.sh crate PACKAGE [k/n]" >&2; exit 2; }
        mutants --package "$2" ${3:+--shard "$3"}
        ;;
    report)
        title="${2:-cargo-mutants}"
        [ -f "$out/mutants.json" ] || { echo "### $title"$'\n\n'"No run: cargo-mutants did not start."; exit 0; }
        n() { if [ -f "$out/$1.txt" ]; then wc -l < "$out/$1.txt" | tr -d ' '; else echo 0; fi; }
        total="$(count "$out/mutants.json")"
        tested=$(( $(n caught) + $(n missed) + $(n timeout) + $(n unviable) ))
        echo "### $title"
        echo
        echo "$tested of $total mutants tested: $(n caught) caught, $(n missed) missed, $(n timeout) timed out, $(n unviable) unviable."
        [ "$tested" -lt "$total" ] && echo "The run stopped before the end (time box); the rest were not tested."
        if [ "$(n missed)" -gt 0 ]; then
            echo
            echo "Missed: no test failed with this change. Each is a test gap to close or explain (advisory)."
            echo
            echo '```'
            cat "$out/missed.txt"
            echo '```'
        fi
        if [ "$(n timeout)" -gt 0 ]; then
            echo
            echo "Timed out: the mutant may make a loop endless; check that no test hangs."
            echo
            echo '```'
            cat "$out/timeout.txt"
            echo '```'
        fi
        ;;
    *)
        sed -n '4,15p' "$0" | sed 's/^# \{0,1\}//' >&2
        exit 2
        ;;
esac
