#!/bin/bash
# HOLD 2 (bench lock): orkut scaling at low thread counts + design probes
cd /home/tux/graphlab/exp/b1-core
THREADS="4 2 1" ALGOS="pr32 wcc bfs kcore tc" scripts/scale_orkut.sh
scripts/probes_orkut.sh
echo H2_DONE
