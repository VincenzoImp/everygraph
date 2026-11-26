#!/bin/bash
# fetch_src.sh OWNER/REPO [REF] — download a source tarball into $S/src/<repo> and record the commit.
set -euo pipefail
S=${S:-/home/tux/graphlab/scratch/m1-18-layout-baseline}
r=$1; ref=${2:-HEAD}; n=${r#*/}
mkdir -p "$S/src/$n"
curl -sSfL "https://api.github.com/repos/$r/commits/$ref" -o "$S/src/$n.commit.json"
curl -sSfL "https://codeload.github.com/$r/tar.gz/$ref" | tar xz -C "$S/src/$n" --strip-components=1
python3 -I -c "import json,sys;j=json.load(open(sys.argv[1]));print(sys.argv[2],j['sha'],j['commit']['committer']['date'])" "$S/src/$n.commit.json" "$r"
