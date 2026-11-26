#!/bin/sh
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Runs a command of bench/specs/references.toml on the 16 physical cores of the lab
# machine (cpus 0-15; cpu i and i+16 are SMT siblings), OpenMP threads bound one per
# core. Every tool runs this way, whatever its thread count. Rayon and Parlay threads
# are not bound one by one: taskset limits them to the same 16 cpus.
#
#   pin.sh COMMAND [ARGS...]
OMP_PLACES=cores OMP_PROC_BIND=spread exec taskset -c 0-15 "$@"
