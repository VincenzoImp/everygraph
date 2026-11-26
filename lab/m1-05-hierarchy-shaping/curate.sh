#!/usr/bin/env bash
# The screenshots kept in the repository: a selection of $S/screens (all of map.sh's PNGs), as
# JPEG (quality 85) to keep the repository small.
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
mkdir -p results/screens
rm -f results/screens/*.jpg
mv results/screens/*.png "$S/screens/" 2>/dev/null || true
for f in lj-rec-1-overview lj-rec-2-aggregate lj-rec-4-inside-largest lj-rec-5-node-level lj-rec-up-1-overview \
         lj-capin-1-overview lj-capin+agg-1-overview lj-capin-4-inside-largest lj-sat-4-inside-largest lj-nest-4-inside-largest \
         orkut-rec-1-overview orkut-rec-up-1-overview orkut-rec-4-inside-largest orkut-capin-4-inside-largest \
         web-google-rec-1-overview web-google-rec-2-aggregate web-google-capin-1-overview \
         roadnet-ca-rec-1-overview roadnet-ca-capin-1-overview \
         friendster-rec-1-top friendster-rec-2-largest; do
  [ -f "$S/screens/$f.png" ] && convert "$S/screens/$f.png" -quality 85 "results/screens/$f.jpg"
done
du -sh results/screens
