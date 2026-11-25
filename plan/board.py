#!/usr/bin/env python3
"""Keep the GitHub project board in step with the work.

usage: plan/board.py todo|doing|done ISSUE [ISSUE ...]
       plan/board.py sync        # closed issues -> Done; open issues with an open PR -> In Progress

The board is user project 7 of VincenzoImp. Status options: Todo, In Progress, Done.
"""
import json, subprocess, sys, time

OWNER, NUMBER, REPO = "VincenzoImp", "7", "VincenzoImp/everygraph"
PROJECT_ID = "PVT_kwHOA6w0f84BtBx7"
FIELD = "PVTSSF_lAHOA6w0f84BtBx7zhrNIZo"
OPTION = {"todo": "f75ad846", "doing": "47fc9ee4", "done": "98236657"}
NAME = {"Todo": "todo", "In Progress": "doing", "Done": "done"}


def gh(*args):
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr)
    return r.stdout


def items():
    data = json.loads(gh("project", "item-list", NUMBER, "--owner", OWNER, "--limit", "500", "--format", "json"))
    return {i["content"]["number"]: (i["id"], NAME.get(i.get("status"), "todo")) for i in data["items"] if i.get("content", {}).get("number")}


def set_status(item_id, state):
    gh("project", "item-edit", "--project-id", PROJECT_ID, "--id", item_id, "--field-id", FIELD,
       "--single-select-option-id", OPTION[state])


cmd = sys.argv[1]
board = items()
if cmd in OPTION:
    for n in map(int, sys.argv[2:]):
        if n not in board:
            gh("project", "item-add", NUMBER, "--owner", OWNER, "--url", f"https://github.com/{REPO}/issues/{n}")
            for _ in range(6):          # the new item can take a moment to appear in the list
                board = items()
                if n in board:
                    break
                time.sleep(2)
        if board[n][1] != cmd:
            set_status(board[n][0], cmd)
        print(f"#{n} -> {cmd}")
elif cmd == "sync":
    issues = json.loads(gh("issue", "list", "-R", REPO, "--state", "all", "--limit", "500", "--json", "number,state"))
    prs = json.loads(gh("pr", "list", "-R", REPO, "--state", "open", "--limit", "200", "--json", "closingIssuesReferences"))
    in_pr = {r["number"] for p in prs for r in p.get("closingIssuesReferences", [])}
    for i in issues:
        n = i["number"]
        if n not in board:
            continue
        want = "done" if i["state"] == "CLOSED" else ("doing" if n in in_pr else None)
        if want and board[n][1] != want:
            set_status(board[n][0], want)
            print(f"#{n} -> {want}")
else:
    raise SystemExit(__doc__)
