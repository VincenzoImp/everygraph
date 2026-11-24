#!/bin/sh
# usage: pget.sh URL OUT PARTS  — parallel ranged download
URL=$1; OUT=$2; N=$3
SIZE=$(curl -sI "$URL" | tr -d '\r' | awk 'tolower($1)=="content-length:"{print $2}')
CH=$(( (SIZE + N - 1) / N ))
i=0; while [ $i -lt $N ]; do
  s=$(( i * CH )); e=$(( s + CH - 1 )); [ $e -ge $SIZE ] && e=$(( SIZE - 1 ))
  curl -sL --retry 20 --retry-delay 5 -r $s-$e -o "$OUT.part$(printf %02d $i)" "$URL" &
  i=$(( i + 1 ))
done; wait
cat "$OUT".part* > "$OUT" && rm -f "$OUT".part* && echo "ok $OUT $(stat -c %s "$OUT") expected $SIZE"
