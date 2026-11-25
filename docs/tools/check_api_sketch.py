#!/usr/bin/env python3
"""Check the structure of docs/api-sketch.md against the plan.

usage: python3 docs/tools/check_api_sketch.py [FILE]   (from the repository root;
                                                       FILE defaults to the sketch)

  * there are ten stories, each with Python code, a command-line session
    and a line naming its issues;
  * the Python blocks parse, and the JSON blocks parse;
  * every issue number cited exists in plan/published.json;
  * in the table "story -> milestone", every issue sits under the milestone
    its file declares, and every story has at least one issue in M2-M7;
  * every function, method and attribute of the stubs in section 3 is named
    in the tables of section 6 by its qualified name (`g.top(`,
    `Table.top(`, `c.of(`, `Hierarchy.of(`), every exception class by its
    name, and every keyword-only parameter as `name=`;
  * the rule of section 2.1 holds for every stub: at most two positional
    parameters, each of them a subject of the call (a name in SUBJECTS), and
    no two methods of one name take them in opposite orders;
  * every name of the stubs that the approved previews do not show is listed
    in section 9 B or 9 C.

Exit status 0 when all hold. No third-party imports.
"""
import ast
import glob
import html
import json
import re
import sys

DOC = "docs/api-sketch.md"
PREVIEWS = ["design/preview/api.md", "design/preview/cli.md",
            "design/preview/choices.md", "design/preview/style/notebook.html",
            "design/preview/style/terminal.html"]

# How section 6 writes a member of each class of the stubs.
PREFIX = {
    None: "eg.", "Graph": "g.", "View": "view.", "Table": "Table.",
    "Communities": "c.", "Community": "Community.", "Hierarchy": "Hierarchy.",
    "Path": "p.", "Map": "m.", "Selection": "Selection.", "Plan": "Plan.",
    "EverygraphError": "EverygraphError.",
}
# Operators and protocols, which section 6 spells in their own way.
SPELLED = {
    ("View", "__and__"): "view & view",
    ("Communities", "__getitem__"): "c[i]",
    ("Communities", "__len__"): "len(c)",
    ("Community", "__len__"): "len(Community)",
    ("Table", "__len__"): "len(t)",
    ("Table", "__arrow_c_schema__"): "the Arrow interface of `Table`",
    ("Table", "__arrow_c_stream__"): "the Arrow interface of `Table`",
    ("Communities", "__arrow_c_schema__"): "and `Communities`",
    ("Communities", "__arrow_c_stream__"): "and `Communities`",
}
# Classes whose fields are parameters, written `name=` in section 6.
PARAMETER_CLASSES = {"Run"}
# The subjects of a call (section 2.1): the only names a stub may take
# positionally.
SUBJECTS = {"data", "path", "source", "target", "matrix", "by", "n", "operation",
            "nodes", "node", "a", "b", "start", "end", "level", "community",
            "other", "requested_schema"}


def section(text, start, end):
    return text[text.index(start):text.index(end)]


def preview_text():
    parts = []
    for path in PREVIEWS:
        t = open(path, encoding="utf-8").read()
        if path.endswith(".html"):
            t = html.unescape(re.sub(r"<[^>]+>", "", t))
        parts.append(t)
    return "\n".join(parts)


def shown(name, previews):
    """Whether a preview shows the name: as `.name`, `name=` or `name`."""
    return bool(re.search(r"\." + re.escape(name) + r"\b", previews)
                or re.search(r"(?<![\w-])" + re.escape(name) + "=", previews)
                or "`" + name + "`" in previews)


def stub_items(stubs):
    """Yield (class or None, name, kind, node) for everything the stubs define."""
    tree = ast.parse(stubs)
    for node in tree.body:
        if isinstance(node, ast.FunctionDef):
            yield None, node.name, "function", node
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            yield None, node.target.id, "attribute", node
        elif isinstance(node, ast.ClassDef):
            yield node.name, node.name, "class", node
            for item in node.body:
                if isinstance(item, ast.FunctionDef):
                    yield node.name, item.name, "method", item
                elif (isinstance(item, ast.AnnAssign)
                      and isinstance(item.target, ast.Name)):
                    yield node.name, item.target.id, "attribute", item


def main(doc):
    text = open(doc, encoding="utf-8").read()
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
    json_blocks = re.findall(r"```json\n(.*?)\n```", text, re.S)
    for i, block in enumerate(json_blocks):
        try:
            json.loads(block)
        except ValueError as e:
            problems.append(f"json block {i + 1} does not parse: {e}")

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
    # Only the rows of the tables count, not the prose around them.
    names = "\n".join(line for line in section(
        text, "## 6. Every public name", "## 7. Gaps").split("\n")
        if line.startswith("|"))
    decided = text[text.index("**B. Names taken from issue files**"):]
    previews = preview_text()

    missing, positional, undecided = [], [], []
    orders = {}
    checked = 0
    for cls, name, kind, node in stub_items(stubs):
        if cls in PARAMETER_CLASSES and kind == "attribute":
            spelled = name + "="
        elif kind == "class":
            if cls in PARAMETER_CLASSES:
                continue
            spelled = name
        elif (cls, name) in SPELLED:
            spelled = SPELLED[(cls, name)]
        elif cls is None:
            spelled = PREFIX[cls] + name
        elif cls in PREFIX:
            spelled = PREFIX[cls] + name + ("(" if kind != "attribute" else "")
        else:
            continue
        checked += 1
        if spelled.endswith("="):
            present = re.search(r"(?<![\w-])" + re.escape(spelled), names)
        elif spelled.endswith("(") or " " in spelled:
            present = spelled in names
        else:
            present = re.search(r"(?<![\w.])" + re.escape(spelled) + r"(?![\w(])",
                                names)
        if not present:
            missing.append(spelled)

        if kind in ("function", "method"):
            args = node.args
            pos = [a.arg for a in args.posonlyargs + args.args if a.arg != "self"]
            if not name.startswith("__"):
                if len(pos) > 2 or any(a not in SUBJECTS for a in pos):
                    positional.append(f"{cls or 'eg'}.{name}({', '.join(pos)})")
                # Two methods of one name may not take the same positional
                # parameters in another order (g.top(by, n) and t.top(n, by)).
                for other_cls, other in orders.get(name, []):
                    if sorted(other) == sorted(pos) and other != pos:
                        positional.append(
                            f"{other_cls}.{name}({', '.join(other)}) and "
                            f"{cls}.{name}({', '.join(pos)}): opposite order")
                orders.setdefault(name, []).append((cls or "eg", pos))
            for a in args.kwonlyargs:
                checked += 1
                if not re.search(r"(?<![\w-])" + re.escape(a.arg) + "=", names):
                    missing.append(a.arg + "=")
                if not (shown(a.arg, previews)
                        or re.search(r"\b" + re.escape(a.arg) + r"\b", decided)):
                    undecided.append(a.arg + "=")
        if kind in ("function", "method", "attribute") and not name.startswith("__"):
            if cls in PARAMETER_CLASSES:
                continue
            listed = re.search(r"\b" + re.escape(name) + r"\b", decided)
            if not shown(name, previews) and not listed:
                undecided.append((PREFIX.get(cls) or "") + name)

    if missing:
        problems.append(f"in the stubs but not in section 6: {sorted(set(missing))}")
    if positional:
        problems.append(f"the rule of section 2.1 is broken: "
                        f"{positional}")
    if undecided:
        problems.append(f"in no preview and not in section 9 B or C: "
                        f"{sorted(set(undecided))}")

    for p in problems:
        print(p)
    print(f"{'FAIL' if problems else 'ok'}  {doc}: {len(stories)} stories, "
          f"{len(blocks)} Python blocks, {len(json_blocks)} JSON blocks, "
          f"{len(cited)} issues cited, {len(rows)} stories mapped to milestones, "
          f"{checked} names checked against section 6, "
          f"{len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else DOC))
