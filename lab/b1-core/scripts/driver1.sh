#!/bin/bash
# wait for the official friendster ingest to finish, then run kernels / probes / out-of-core
cd /home/tux/graphlab/exp/b1-core
until grep -q INGEST_FR_DONE logs/ingest_fr_official.log; do sleep 2; done
BL=/home/tux/graphlab/bench.lock
THREADS="32 16 8" flock $BL scripts/scale_orkut.sh
THREADS="4 2 1" flock $BL scripts/scale_orkut.sh
echo ORKUT_SCALE_DONE
flock $BL scripts/probes_orkut.sh
echo PROBES_DONE
scripts/scale_fr.sh
echo FR_SCALE_DONE
scripts/ooc_fr.sh
echo OOC_DONE
