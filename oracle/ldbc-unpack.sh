# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Sourced by run-ci.sh and run-nightly.sh (issue #177).
#
#   ldbc_unpack BENCH DATA OUT DATASET...
#
# Fetches the LDBC Graphalytics datasets of the registry into DATA (checksums verified)
# and unpacks each archive into OUT/<graph>/, the layout `everygraph-oracle ldbc` reads.
# The archives are untrusted input: tar does not restore owners or permissions, and
# GNU tar refuses member names with "..".
ldbc_unpack() {
    local bench="$1" data="$2" out="$3"
    shift 3
    "$bench" fetch --data "$data" "$@" > "$data.ldbc-fetch.log" 2>&1 \
        || { cat "$data.ldbc-fetch.log"; return 1; }
    local ds archive name
    for ds in "$@"; do
        for archive in "$data/$ds"/*.tar.zst; do
            name="$(basename "$archive" .tar.zst)"
            if [[ ! -d "$out/$name" ]]; then
                mkdir -p "$out/$name"
                tar --use-compress-program=unzstd --no-same-owner --no-same-permissions \
                    -xf "$archive" -C "$out/$name"
            fi
        done
    done
}
