#!/usr/bin/env bash
# Run one command under a hard memory limit (CLAUDE.md § 7): ./lim.sh 8G cmd args...
# Exploratory runs only; official numbers come from bench/specs/hierarchy-shaping.toml.
# NOLIM=1 (set by the spec): run the command directly, inside the harness's own limited scope
# (a nested systemd-run scope would leave the harness's cgroup and escape its measurement).
set -u
lim=$1
shift
[ -n "${NOLIM:-}" ] && exec "$@"
exec systemd-run --user --scope -q -p MemoryMax="$lim" -p MemorySwapMax=0 "$@"
