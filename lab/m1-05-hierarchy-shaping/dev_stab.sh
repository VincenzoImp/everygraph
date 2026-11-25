#!/usr/bin/env bash
# Stability (exploratory), after the review of PR #162: six perturbations of the graph, every
# variant rebuilt on each, compared with the unperturbed tree level by level, levels matched by
# role (super, T, T+1, T+2), aggregates resolved, on the nodes the drop did not isolate:
#   d11..d15  1% of the undirected edges dropped, hash seeds 11..15
#   k21       1% dropped, seed 21, never a node's edge of smallest hash (no node isolated)
# Run after dev_all.sh on the same graph (it needs the unperturbed trees).
#   ./dev_stab.sh GRAPH STORE [VARIANTS...]      -> results/dev/GRAPH.stab.jsonl
set -eu
cd "$(dirname "$0")"
S=${S:-/home/tux/graphlab/scratch/m1-05-hierarchy-shaping}
g=$1; store=$2; shift 2
variants=${*:-capin sat capafter rec rec-up nest nest-up}
PY=${PY:-$S/venv/bin/python}
LIM=${LIM:-8G}
M=../m1-02-ooc-leiden/target/release/m102
out=${OUT:-results/dev/$g.stab.jsonl}
: > "$out"
mkdir -p "$S/stores"
for p in d11 d12 d13 d14 d15 k21; do
  seed=${p:1}
  keep=""; [ "${p:0:1}" = k ] && keep="--drop-keep"
  P=$S/stores/$g-$p
  [ -d "$P" ] || ./lim.sh $LIM $M cutgraph --store "$store" --out "$P" --drop 0.01 --seed $seed $keep --out-deg "$S/stores/$g-$p.deg.u32" --threads 16 > "$S/stores/$g-$p.json"
  NOEXPORT=1 NOMETRICS=1 ./dev_all.sh "$g-$p" "$P" $variants
  for v in $variants; do
    ./lim.sh $LIM $PY -I shape.py compare --tree "$S/$g/tree_$v.npz" --tree "$S/$g-$p/tree_$v.npz" --mask "$S/stores/$g-$p.deg.u32" \
      | python3 -I -c "import json,sys; d=json.loads(sys.stdin.read()); d.update(graph='$g', perturbation='$p', variant='$v'); print(json.dumps(d))" >> "$out"
  done
done
python3 -I - "$out" <<'EOF'
import json, sys, collections
rows = [json.loads(l) for l in open(sys.argv[1])]
agg = collections.defaultdict(list)
for r in rows:
    for lvl, x in r["by_role"].items():
        agg[(r["variant"], lvl)].append((x["ari"], x["nmi"]))
for (v, lvl), xs in sorted(agg.items()):
    a = sorted(x[0] for x in xs); m = sorted(x[1] for x in xs)
    print(f"{v:10s} {lvl:6s} n={len(xs)} ARI median {a[len(a)//2]:.3f} ({a[0]:.3f}-{a[-1]:.3f})  NMI median {m[len(m)//2]:.3f} ({m[0]:.3f}-{m[-1]:.3f})")
sup = collections.defaultdict(list)
for r in rows:
    if r["variant"] == "rec":
        sup[r["perturbation"]] = (r["super_built_a"], r["super_built_b"], r["cover90_T_a"], r["cover90_T_b"])
print("rec (auto) super level, unperturbed / perturbed, cover90 of T:", dict(sup))
# one JSON line for the benchmark harness: medians per variant and level
flat = {"graph": rows[0]["graph"] if rows else None, "perturbations": len({r["perturbation"] for r in rows})}
for (v, lvl), xs in sorted(agg.items()):
    a = sorted(x[0] for x in xs); m = sorted(x[1] for x in xs)
    flat[f"{v}__{lvl}__ari_median"] = a[len(a) // 2]
    flat[f"{v}__{lvl}__nmi_median"] = m[len(m) // 2]
flat["rec_auto_super_flips"] = sum(1 for p, s in sup.items() if s[0] != s[1])
print(json.dumps(flat))
EOF
