#!/usr/bin/env python3
"""Check the structure of docs/api-sketch.md against the plan.

usage: python3 docs/tools/check_api_sketch.py        (from the repository root)

  * there are ten stories, each with Python code, a command-line session
    and a line naming its issues;
  * the Python blocks parse;
  * every issue number cited exists in plan/published.json;
  * in the table "story -> milestone", every issue sits under the milestone
    its file declares, and every story has at least one issue in M2-M7;
  * every method and attribute of the stubs in section 3 is named in the
    table of section 6.

Exit status 0 when all hold. No third-party imports.
"""
import ast
import glob
import json
import re
import sys

DOC = "docs/api-sketch.md"
# Spelled differently in section 6 than in the stubs: operators, the Arrow
# interface, and the fields of two small classes that are listed as a group.
GROUPED = {
    "__and__": "view & view", "__getitem__": "c[i]", "__len__": "len()",
    "__arrow_c_schema__": "the Arrow interface",
    "__arrow_c_stream__": "the Arrow interface",
    "message": "`EverygraphError` and its attributes",
    "ways_forward": "`EverygraphError` and its attributes",
    "changed": "`EverygraphError` and its attributes",
    "id": "`Community`", "level": "`Community`",
}


def section(text, start, end):
    return text[text.index(start):text.index(end)]


def main():
    text = open(DOC, encoding="utf-8").read()
    published = json.load(open("plan/published.json"))
    milestone = {}
    for path in glob.glob("plan/issues/*.md"):
        key = path.split("/")[-1][:-3]
        m = re.search(r"^milestone: (M\d)", open(path, encoding="utf-8").read(),
                      re.M)
        if key in published and m:
            milestone[published[key]] = m.group(1)
    problems = []

    stories = re.split(r"^### Story ", section(
        text, "## 1. Ten user stories", "## 2. Conventions"), flags=re.M)[1:]
    if len(stories) != 10:
        problems.append(f"{len(stories)} stories, not 10")
    for story in stories:
        title = story.split("\n")[0]
        for need, what in (("```python", "Python code"),
                           ("```console", "a command-line session"),
                           ("\nIssues: ", "its issues")):
            if need not in story:
                problems.append(f"story {title!r} lacks {what}")

    blocks = re.findall(r"```python\n(.*?)\n```", text, re.S)
    for i, block in enumerate(blocks):
        try:
            ast.parse(block)
        except SyntaxError as e:
            problems.append(f"python block {i + 1} does not parse: {e.msg}")

    cited = sorted({int(n) for n in re.findall(r"#(\d+)", text)})
    unknown = [n for n in cited if n not in published.values()]
    if unknown:
        problems.append(f"issues cited but not in the plan: {unknown}")

    table = text[text.index("| Story | M2 | M3 | M4 | M5 | M6 | M7 |"):]
    table = table[:table.index("\n\n")]
    rows = table.split("\n")[2:]
    for row in rows:
        cells = [c.strip() for c in row.split("|")[1:-1]]
        found = 0
        for column, cell in zip(("M2", "M3", "M4", "M5", "M6", "M7"), cells[1:]):
            for n in re.findall(r"#(\d+)", cell):
                found += 1
                if milestone.get(int(n)) != column:
                    problems.append(f"story {cells[0]!r}: #{n} is in "
                                    f"{milestone.get(int(n))}, not {column}")
        if not found:
            problems.append(f"story {cells[0]!r} maps to no issue")

    stubs = "\n".join(re.findall(r"```python\n(.*?)\n```", section(
        text, "## 3. The object model", "## 4. The Rust"), re.S))
    members = set(re.findall(r"^\s*def ([a-z_]+)\(", stubs, re.M))
    members |= set(re.findall(r"^    ([a-z_]+): ", stubs, re.M))
    names = section(text, "## 6. Every public name", "## 7. Gaps")
    missing = []
    for member in sorted(members):
        spelled = GROUPED.get(member, member)
        if spelled in GROUPED.values():
            present = spelled in names
        else:
            present = bool(re.search(r"\b" + re.escape(spelled) + r"\b", names))
        if not present:
            missing.append(member)
    if missing:
        problems.append(f"in the stubs but not in section 6: {missing}")

    for p in problems:
        print(p)
    print(f"{'FAIL' if problems else 'ok'}  {DOC}: {len(stories)} stories, "
          f"{len(blocks)} Python blocks, {len(cited)} issues cited, "
          f"{len(rows)} stories mapped to milestones, "
          f"{len(members)} members checked against section 6, "
          f"{len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
