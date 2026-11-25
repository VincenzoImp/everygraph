#!/usr/bin/env python3
"""Check the architecture decision records under docs/adr/ (standard library only).

Checks:
  1. ADR 0000 and one ADR per decision D1..D25 exist, named NNNN-<slug>.md, one per number.
  2. Each ADR has a status from the vocabulary of docs/adr/README.md and the required
     sections; an ADR whose status is not `accepted` has a "Closed by" section citing #N.
  3. Each decision row D1..D25 of the table in docs/DESIGN.md links to its ADR.
  4. The index in docs/adr/README.md links every ADR, with the status the ADR states.
  5. Every #N cited in docs/adr/ is an issue in plan/published.json.
  6. Every lab/... path cited in docs/adr/ exists in the repository.

Usage: python3 docs/tools/check_adrs.py [--root REPO]
Exit status 0 when every check passes, 1 otherwise.
"""

import argparse
import glob
import json
import pathlib
import re
import sys

N_DECISIONS = 25
STATUSES = ("accepted", "accepted, partly validated", "open — needs experiment")
SUPERSEDED = re.compile(r"^superseded by ADR \d{4}$")
SECTIONS = ("Context", "Decision", "Evidence", "Consequences", "What would change our mind")
NAME = re.compile(r"^(\d{4})-[a-z0-9]+(?:-[a-z0-9]+)*\.md$")
STATUS_LINE = re.compile(r"^- \*\*Status:\*\* (.+?)\s*$", re.M)
ISSUE_REF = re.compile(r"(?<![\w/&#])#(\d+)\b")
LAB_PATH = re.compile(r"(?<![\w/.-])lab/[A-Za-z0-9_.*/-]+")


def sections(text):
    return {m.group(1).strip() for m in re.finditer(r"^## (.+)$", text, re.M)}


def section_body(text, title):
    m = re.search(r"^## " + re.escape(title) + r"\s*$(.*?)(?=^## |\Z)", text, re.M | re.S)
    return m.group(1) if m else ""


def check(root):
    errors = []
    adr_dir = root / "docs" / "adr"
    files = {}
    for p in sorted(adr_dir.glob("*.md")):
        if p.name == "README.md":
            continue
        m = NAME.match(p.name)
        if not m:
            errors.append(f"{p.name}: name is not NNNN-<slug>.md")
            continue
        num = int(m.group(1))
        if num in files:
            errors.append(f"ADR {num:04d}: two files ({files[num].name}, {p.name})")
        files[num] = p

    expected = set(range(0, N_DECISIONS + 1))
    for num in sorted(expected - files.keys()):
        errors.append(f"ADR {num:04d}: missing")
    for num in sorted(files.keys() - expected):
        errors.append(f"ADR {num:04d}: no decision D{num} in DESIGN.md")

    statuses = {}
    issue_refs, lab_paths = [], []
    for num, p in sorted(files.items()):
        text = p.read_text(encoding="utf-8")
        found = STATUS_LINE.findall(text)
        if len(found) != 1:
            errors.append(f"{p.name}: expected one '- **Status:**' line, found {len(found)}")
            status = None
        else:
            status = found[0]
            if status not in STATUSES and not SUPERSEDED.match(status):
                errors.append(f"{p.name}: status '{status}' is not in the vocabulary")
        statuses[num] = status
        present = sections(text)
        for s in SECTIONS:
            if s not in present:
                errors.append(f"{p.name}: missing section '## {s}'")
        if num >= 1 and not re.search(rf"^- \*\*Decision:\*\* D{num}\b", text, re.M):
            errors.append(f"{p.name}: header does not name decision D{num}")
        if status and status != "accepted" and not SUPERSEDED.match(status):
            if not ISSUE_REF.search(section_body(text, "Closed by")):
                errors.append(f"{p.name}: status '{status}' but no '## Closed by' citing #N")
        issue_refs += [(p.name, int(n)) for n in ISSUE_REF.findall(text)]
        lab_paths += [(p.name, x) for x in LAB_PATH.findall(text)]

    # 3. DESIGN.md decision table
    design = (root / "docs" / "DESIGN.md").read_text(encoding="utf-8")
    for k in range(1, N_DECISIONS + 1):
        row = re.search(rf"^\| \[D{k}\]\(adr/([^)]+)\) \|", design, re.M)
        if not row:
            errors.append(f"DESIGN.md: row D{k} does not link to an ADR")
            continue
        target = row.group(1)
        if k in files and target != files[k].name:
            errors.append(f"DESIGN.md: D{k} links to {target}, expected {files[k].name}")
        elif not (adr_dir / target).exists():
            errors.append(f"DESIGN.md: D{k} links to missing file {target}")

    # 4. README index
    readme = (adr_dir / "README.md").read_text(encoding="utf-8")
    index = {}
    for m in re.finditer(r"^\| \[(\d{4})\]\(([^)]+)\) \| [^|]+ \| ([^|]+?) \| [^|]+ \|$", readme, re.M):
        index[int(m.group(1))] = (m.group(2), m.group(3).strip())
    for num, p in sorted(files.items()):
        if num not in index:
            errors.append(f"README.md: ADR {num:04d} is not in the index")
            continue
        link, st = index[num]
        if link != p.name:
            errors.append(f"README.md: ADR {num:04d} links to {link}, expected {p.name}")
        if statuses.get(num) and st != statuses[num]:
            errors.append(f"README.md: ADR {num:04d} listed as '{st}', the ADR says '{statuses[num]}'")
    for num, (link, _) in sorted(index.items()):
        if not (adr_dir / link).exists():
            errors.append(f"README.md: index entry {num:04d} links to missing file {link}")
    issue_refs += [("README.md", int(n)) for n in ISSUE_REF.findall(readme)]
    lab_paths += [("README.md", x) for x in LAB_PATH.findall(readme)]

    # 5. issue references
    published = set(json.loads((root / "plan" / "published.json").read_text()).values())
    for name, n in issue_refs:
        if n not in published:
            errors.append(f"{name}: #{n} is not an issue in plan/published.json")

    # 6. lab paths
    for name, raw in lab_paths:
        path = raw.rstrip(".,;:")
        hits = glob.glob(str(root / path)) if "*" in path else [p for p in [root / path] if p.exists()]
        if not hits:
            errors.append(f"{name}: cited path {path} does not exist")

    linked = len(re.findall(r"^\| \[D\d+\]\(adr/", design, re.M))
    summary = (f"{len(files)} ADRs, {linked} decision rows linked from DESIGN.md, "
               f"{len(index)} index entries, {len(issue_refs)} issue references, "
               f"{len(lab_paths)} lab paths")
    return errors, summary


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", type=pathlib.Path,
                    default=pathlib.Path(__file__).resolve().parents[2],
                    help="repository root (default: two levels above this script)")
    args = ap.parse_args()
    errors, summary = check(args.root.resolve())
    for e in errors:
        print(f"FAIL {e}")
    if errors:
        print(f"{len(errors)} problem(s); checked {summary}")
        return 1
    print(f"OK: {summary}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
