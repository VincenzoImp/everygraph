#!/usr/bin/env bash
# Run one command under a hard memory limit (CLAUDE.md § 7): ./lim.sh 8G cmd args...
# Exploratory runs only; official numbers come from bench/specs/hierarchy-shaping.toml.
set -u
lim=$1
shift
exec systemd-run --user --scope -q -p MemoryMax="$lim" -p MemorySwapMax=0 "$@"
