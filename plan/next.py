#!/usr/bin/env python3
"""Show which issues are ready to start: open, with every dependency closed.

usage: plan/next.py [OWNER/REPO]        (default VincenzoImp/everygraph)

Dependencies are the issues referenced on a "**Depends on**" or "**Blocked by" line of
plan/issues/<slug>.md. State comes from GitHub; numbers from plan/published.json.
Output is grouped by milestone and ordered P0 → P2, then by file name.
"""
import json, pathlib, re, subprocess, sys

repo = sys.argv[1] if len(sys.argv) > 1 else "VincenzoImp/everygraph"
here = pathlib.Path(__file__).parent
numbers = json.loads((here / "published.json").read_text())
raw = subprocess.run(["gh", "issue", "list", "-R", repo, "--state", "all", "--limit", "500",
                      "--json", "number,state"], capture_output=True, text=True, check=True).stdout
state = {i["number"]: i["state"] for i in json.loads(raw)}

issues = {}
for path in sorted((here / "issues").glob("*.md")):
    head, body = re.match(r"---\n(.*?)\n---\n(.*)", path.read_text(), re.S).groups()
    meta = dict((k.strip(), v.strip()) for k, v in (l.split(":", 1) for l in head.splitlines()))
    deps = set()
    for line in body.splitlines():
        if line.startswith("**Depends on**") or line.startswith("**Blocked by"):
            deps |= set(re.findall(r"\{\{#([^}]+)\}\}", line))
    labels = [x.strip() for x in meta["labels"].split(",")]
    prio = next((p for p in ("P0", "P1", "P2") if p in labels), "P9")
    issues[path.stem] = dict(title=meta["title"], ms=meta["milestone"], deps=deps, prio=prio,
                             epic="type:epic" in labels, n=numbers.get(path.stem))


def closed(slug):
    return state.get(issues[slug]["n"]) == "CLOSED"


by_ms = {}
for slug, i in issues.items():
    if i["epic"] or i["n"] is None or closed(slug):
        continue
    waiting = sorted(d for d in i["deps"] if not closed(d))
    by_ms.setdefault(i["ms"], []).append((i["prio"], slug, i, waiting))

print(f"{sum(len(v) for v in by_ms.values())} open issues (epics excluded), "
      f"{sum(1 for s in issues if closed(s))} closed\n")
for ms in sorted(by_ms):
    ready = [x for x in sorted(by_ms[ms], key=lambda x: x[:2]) if not x[3]]
    blocked = [x for x in sorted(by_ms[ms], key=lambda x: x[:2]) if x[3]]
    print(f"{ms} — {len(ready)} ready, {len(blocked)} blocked")
    for prio, slug, i, _ in ready:
        print(f"  READY   {prio} #{i['n']:<4} {slug}")
    for prio, slug, i, waiting in blocked:
        print(f"  blocked {prio} #{i['n']:<4} {slug}  <- {', '.join('#%s' % issues[w]['n'] for w in waiting)}")
    print()
