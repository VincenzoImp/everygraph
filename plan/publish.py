#!/usr/bin/env python3
"""Publish the plan to GitHub: labels, milestones and issues. Idempotent: re-running updates title,
body, labels and milestone of issues already published (matched through published.json, else by title).

usage: publish.py OWNER/REPO [--dry-run]

Reads labels.json, milestones.json and issues/*.md next to this file. Each issue file starts with
a front matter block:
    ---
    title: ...
    milestone: M1 — ...
    labels: area:storage, type:feature, P0
    ---
    <markdown body>
Files are published in filename order, so issue numbers follow the plan order. A '{{#slug}}'
placeholder in a body is replaced with the issue number of the file whose name is slug
(two passes: create everything, then patch bodies that contain references).

What this script does not do:
- It never closes or deletes an issue. A file that is removed leaves its GitHub issue open and
  unowned, drops its number from published.json, and breaks every '{{#slug}}' that cites it.
  So an issue that is merged into another keeps its file, which says where its scope went, and
  the GitHub issue is closed by hand.
- A renamed file keeps its issue only while its title is unchanged (the fallback match). Rename
  and retitle in two separate runs, or edit published.json in the same commit.
- The title is the text after 'title:', taken literally: do not quote it.
The file name is a stable identifier. The milestone and labels of the front matter are the truth,
also for a file whose prefix names an earlier milestone (for example m3-09, now in M9).
"""
import json, pathlib, subprocess, sys, re, time

repo = sys.argv[1]; dry = "--dry-run" in sys.argv
here = pathlib.Path(__file__).parent


def gh(*args, inp=None):
    r = subprocess.run(["gh", "api", *args], input=inp, capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(f"gh api {' '.join(args)[:200]}\n{r.stderr}")
    return json.loads(r.stdout) if r.stdout.strip() else None


def paged(path):
    out, page = [], 1
    while True:
        chunk = gh(f"{path}{'&' if '?' in path else '?'}per_page=100&page={page}")
        out += chunk
        if len(chunk) < 100:
            return out
        page += 1


def parse(path):
    text = path.read_text()
    m = re.match(r"---\n(.*?)\n---\n(.*)", text, re.S)
    if not m:
        raise SystemExit(f"{path}: missing front matter")
    meta = dict(line.split(":", 1) for line in m.group(1).splitlines() if ":" in line)
    meta = {k.strip(): v.strip() for k, v in meta.items()}
    return {"slug": path.stem.split("-", 1)[0] if False else path.stem, "title": meta["title"],
            "milestone": meta.get("milestone"), "labels": [x.strip() for x in meta.get("labels", "").split(",") if x.strip()],
            "body": m.group(2).strip() + "\n"}


issues = [parse(p) for p in sorted((here / "issues").glob("*.md"))]
labels = json.loads((here / "labels.json").read_text())
milestones = json.loads((here / "milestones.json").read_text())

known_labels = {l["name"] for l in labels}
known_ms = {m["title"] for m in milestones}
for i in issues:
    for l in i["labels"]:
        assert l in known_labels, f"{i['slug']}: unknown label {l}"
    assert i["milestone"] in known_ms, f"{i['slug']}: unknown milestone {i['milestone']}"
titles = [i["title"] for i in issues]
assert len(set(titles)) == len(titles), "duplicate titles"
slugs = {i["slug"] for i in issues}
for i in issues:
    for ref in re.findall(r"\{\{#([^}]+)\}\}", i["body"]):
        assert ref in slugs, f"{i['slug']}: unknown reference {ref}"
print(f"{len(issues)} issues, {len(milestones)} milestones, {len(labels)} labels validated")
if dry:
    for m in milestones:
        print(f"  {m['title']}: {sum(i['milestone'] == m['title'] for i in issues)} issues")
    sys.exit(0)

have = {l["name"] for l in paged(f"repos/{repo}/labels")}
for l in labels:
    if l["name"] not in have:
        gh(f"repos/{repo}/labels", "-f", f"name={l['name']}", "-f", f"color={l['color']}", "-f", f"description={l.get('description', '')}")
ms = {m["title"]: m["number"] for m in paged(f"repos/{repo}/milestones?state=all")}
for m in milestones:
    if m["title"] not in ms:
        ms[m["title"]] = gh(f"repos/{repo}/milestones", "-f", f"title={m['title']}", "-f", f"description={m['description']}")["number"]
existing = {i["title"]: i["number"] for i in paged(f"repos/{repo}/issues?state=all") if "pull_request" not in i}
known = json.loads((here / "published.json").read_text()) if (here / "published.json").exists() else {}
numbers = {}
for i in issues:
    # published.json (slug -> number) survives title edits; fall back to the exact title
    if i["slug"] in known or i["title"] in existing:
        numbers[i["slug"]] = known.get(i["slug"], existing.get(i["title"]))
        continue
    payload = {"title": i["title"], "body": i["body"], "labels": i["labels"], "milestone": ms[i["milestone"]]}
    numbers[i["slug"]] = gh(f"repos/{repo}/issues", "--input", "-", inp=json.dumps(payload))["number"]
    print(f"#{numbers[i['slug']]} {i['title']}")
    time.sleep(1.2)   # stay clear of secondary rate limits
for i in issues:
    body = re.sub(r"\{\{#([^}]+)\}\}", lambda m: f"#{numbers[m.group(1)]}", i["body"])
    payload = {"title": i["title"], "body": body, "labels": i["labels"], "milestone": ms[i["milestone"]]}
    gh(f"repos/{repo}/issues/{numbers[i['slug']]}", "-X", "PATCH", "--input", "-", inp=json.dumps(payload))
    time.sleep(0.5)
(here / "published.json").write_text(json.dumps(numbers, indent=1))
print("done:", len(numbers), "issues")
