#!/usr/bin/env bash
# SVG -> PNG through the shared Playwright install (headless Chromium, file:// URL, no server).
#   ./svgshot.sh IN.svg OUT.png [size]
set -eu
PW=${PW:-/home/tux/graphlab/scratch/pw}
in=$(realpath "$1"); out=$(realpath -m "$2"); size=${3:-1000}
cd "$PW" && node shot.mjs "$in" "$out" "$size" "$((size + 40))" 300
