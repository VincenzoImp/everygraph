#!/usr/bin/env python3
"""Check docs/spec/graph-semantics.md against the API sketch and the UX contract.

usage: python3 docs/tools/check_graph_semantics.py   (from the repository root)

  * every `g.NAME` of the table "Python: functions and `Graph`" of
    docs/api-sketch.md section 6 that reads the graph's edges appears in the
    spec's kernel table (section 6), and the table names nothing the sketch
    does not;
  * the option values of the spec (section 2.1) are those of the stubs of
    `eg.read` and `eg.from_edges` (`Literal[...]`), and the printed values are
    the enumerations of the plan schema of docs/ux-contract.md section 3.6;
  * every exception the spec names with an exit status, in the kernel table
    and in the refusals table, has that exit status in the sketch's table of
    section 2.7.

Exit status 0 when all hold. No third-party imports.
"""
import re
import sys

SPEC = "docs/spec/graph-semantics.md"
SKETCH = "docs/api-sketch.md"
CONTRACT = "docs/ux-contract.md"

# Members of `Graph` in section 6 that do not read edges, so have no row.
NO_ROW = {
    "info": "metadata: prints what the file holds",
    "explain": "returns a plan, computes nothing",
    "columns": "metadata: the list of columns",
    "directed": "metadata: the graph's kind",
    "file": "metadata: the path",
}
# Python's own exceptions, which section 2.7 lists outside its table.
PYTHON_ERRORS = {"ValueError": 2, "TypeError": 2}


def block(text, name):
    m = re.search(rf"<!-- check: {name} -->\n(.*?)<!-- /check: {name} -->", text, re.S)
    if not m:
        sys.exit(f"{SPEC}: no block '{name}'")
    return m.group(1)


def main():
    spec = open(SPEC, encoding="utf-8").read()
    sketch = open(SKETCH, encoding="utf-8").read()
    contract = open(CONTRACT, encoding="utf-8").read()
    errors = []

    # 1. Section 6 of the sketch against the kernel table.
    m = re.search(r"\*\*Python: functions and `Graph`\*\*(.*?)\*\*Python: the other objects\*\*",
                  sketch, re.S)
    if not m:
        sys.exit(f"{SKETCH}: section 6 table not found")
    named = set(re.findall(r"`g\.(\w+)", m.group(1)))
    table = block(spec, "kernel-table")
    rows = set(re.findall(r"`g\.(\w+)", table))
    for name in sorted(named - rows - set(NO_ROW)):
        errors.append(f"g.{name} is in {SKETCH} § 6 but not in the kernel table")
    for name in sorted(rows - named):
        errors.append(f"g.{name} is in the kernel table but not in {SKETCH} § 6")
    for name in sorted(set(NO_ROW) - named):
        errors.append(f"NO_ROW names g.{name}, which {SKETCH} § 6 does not")

    # 2. Options and printed values.
    options = block(spec, "options")
    for opt in ("self_loops", "duplicates"):
        stubs = re.findall(rf"{opt}: Literal\[([^\]]*)\]", sketch)
        if len(stubs) < 2:
            errors.append(f"{SKETCH}: fewer than two stubs take {opt}=")
        values = {tuple(re.findall(r'"([^"]+)"', s)) for s in stubs}
        if len(values) != 1:
            errors.append(f"{SKETCH}: the stubs disagree on {opt}: {values}")
            continue
        (want,) = values
        row = next((r for r in options.splitlines() if r.startswith(f"| `{opt}=`")), "")
        cells = row.split("|")
        got = tuple(re.findall(r'`"([^"]+)"`', cells[2])) if len(cells) > 2 else ()
        if got != want:
            errors.append(f"{opt}: spec values {got}, stubs {want}")
        enum = re.search(rf'"{opt}": \{{\s*"enum": \[([^\]]*)\]', contract)
        if not enum:
            errors.append(f"{CONTRACT}: no enum for {opt}")
            continue
        printed = re.findall(r'"([^"]+)"', enum.group(1))
        tail = cells[4] if len(cells) > 4 else ""
        listed = re.findall(r"`([^`]+)`", tail.split("values", 1)[-1])
        if listed != printed:
            errors.append(f"{opt}: spec printed values {listed}, contract enum {printed}")

    # 3. Exceptions and exit statuses.
    t27 = re.search(r"\| Exception \| Also a \|.*?\n\n", sketch, re.S)
    if not t27:
        sys.exit(f"{SKETCH}: table of § 2.7 not found")
    status = dict(PYTHON_ERRORS)
    for line in t27.group(0).splitlines():
        cells = [c.strip() for c in line.split("|")]
        if len(cells) > 6 and re.fullmatch(r"`\w+`", cells[1]) and cells[-2].isdigit():
            status[cells[1].strip("`")] = int(cells[-2])
    pairs = re.findall(r"`(\w+)` \((\d)\)", table)
    for line in block(spec, "refusals").splitlines():
        cells = [c.strip() for c in line.split("|")]
        if len(cells) > 3 and re.fullmatch(r"`\w+`", cells[2]):
            pairs.append((cells[2].strip("`"), cells[3]))
    if not pairs:
        errors.append("no exception with an exit status found in the spec")
    for exc, code in pairs:
        if exc not in status:
            errors.append(f"{exc} is not an exception of {SKETCH} § 2.7")
        elif status[exc] != int(code):
            errors.append(f"{exc}: exit status {code} in the spec, {status[exc]} in {SKETCH}")

    for e in errors:
        print(f"error: {e}")
    if errors:
        return 1
    print(f"ok: {len(named)} names of § 6, {len(pairs)} refusals checked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
