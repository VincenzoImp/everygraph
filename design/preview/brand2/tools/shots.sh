#!/bin/sh
# Re-takes every round-2 screenshot. Usage: sh tools/shots.sh   (from design/preview/brand2)
set -e
SHOT=/home/tux/graphlab/scratch/pw/shot.mjs
OUT=../screens
h() { node tools/measure.mjs "$1" 1440 | sed 's/.*"height":\([0-9]*\).*/\1/'; }
node $SHOT systems.html $OUT/c-01-systems-overview.png 1440 900 500
for n in 1 2 3 4 5; do
  alt=dark; [ "$n" = 3 ] && alt=light
  u="systems.html#only=$n";                              node $SHOT "$u" $OUT/c-1$n-system-$n.png 1440 "$(h "$u")" 600
  u="systems.html#only=$n&part=context";                 node $SHOT "$u" $OUT/c-2$n-system-$n-context.png 1440 "$(h "$u")" 600
  u="systems.html#only=$n&theme=$alt&part=components,context"; node $SHOT "$u" $OUT/c-3$n-system-$n-$alt.png 1440 "$(h "$u")" 600
done
