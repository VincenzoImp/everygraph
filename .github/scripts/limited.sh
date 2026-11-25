#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Runs a command under a hard memory limit, in a systemd user scope of its own, and
# stops that scope afterwards so that nothing the command left behind keeps running on
# the self-hosted runner (CONTRIBUTING.md, "CI"). The scope is stopped by its exact unit
# name, never by a process name or pattern.
#
#   .github/scripts/limited.sh 8G cargo nextest run --workspace
#
# Needs a systemd user session for the runner's user (`systemd-run --user` must work);
# without one this fails rather than running unlimited.
set -u
if [ "$#" -lt 2 ]; then
    echo "usage: $0 <MemoryMax> <command> [args...]" >&2
    exit 2
fi
limit="$1"
shift
unit="everygraph-ci-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}-${GITHUB_JOB:-job}-$$"
status=0
systemd-run --user --scope --quiet --collect --unit="$unit" \
    -p MemoryMax="$limit" -p MemorySwapMax=0 -- "$@" || status=$?
systemctl --user stop "$unit.scope" 2>/dev/null || true
exit "$status"
