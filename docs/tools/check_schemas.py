#!/usr/bin/env python3
"""Check the JSON Schemas written out in docs/ux-contract.md.

usage: uv run --no-project --with jsonschema \
           python docs/tools/check_schemas.py docs/ux-contract.md

Every ```json block that carries "$schema" must be a valid draft 2020-12
schema (plan, error, event and stop documents); the schemas refer to each
other by their "$id", and those references must resolve. Every other ```json
block is an example and must validate against the schema it belongs to: a
plan, an error document (also the form of a refusal inside a plan), an event,
or a stop document. A refusal is also checked inside a plan with
"fits": false, through the plan's reference to the error schema. Negative
controls show that the schemas reject a missing key, an unknown error class,
a misspelt key, a way forward without its "python" key, an unknown progress
unit and an unknown stop reason. Exit status 0 when everything holds.
"""
import copy
import json
import re
import sys

from jsonschema import Draft202012Validator
from referencing import Registry, Resource
from referencing.jsonschema import DRAFT202012

PLAN = "urn:everygraph:schema:plan:1"
ERROR = "urn:everygraph:schema:error:1"
EVENT = "urn:everygraph:schema:event:1"
STOP = "urn:everygraph:schema:stop:1"


def kind_of(doc):
    """Which schema an example belongs to, from its keys."""
    if "event" in doc:
        return EVENT
    if "stopped" in doc:
        return STOP
    if "operation" in doc and "fits" in doc:
        return PLAN
    if "error" in doc:
        return ERROR
    return None


def main(path):
    text = open(path, encoding="utf-8").read()
    docs = [json.loads(b) for b in re.findall(r"```json\n(.*?)\n```", text, re.S)]
    schemas = {d["$id"]: d for d in docs if "$schema" in d}
    examples = [d for d in docs if "$schema" not in d]
    for sid in (PLAN, ERROR, EVENT, STOP):
        if sid not in schemas:
            raise SystemExit(f"schema not found: {sid}")
    registry = Registry().with_resources(
        (sid, Resource(contents=s, specification=DRAFT202012))
        for sid, s in schemas.items())
    validators = {}
    for sid, schema in schemas.items():
        Draft202012Validator.check_schema(schema)
        validators[sid] = Draft202012Validator(schema, registry=registry)
        print("valid schema:", sid)

    def errors(sid, doc):
        return list(validators[sid].iter_errors(doc))

    failed = False
    by_kind = {}
    for d in examples:
        sid = kind_of(d)
        if sid is None:
            raise SystemExit(f"example matches no schema: {list(d)[:4]}")
        problems = errors(sid, d)
        for e in problems:
            print(f"  {sid}: {e.json_path}: {e.message}")
        failed = failed or bool(problems)
        name = d.get("event") or d.get("operation") or d.get("error")
        print(f"{sid.split(':')[3]} example validates: {name}:", not problems)
        by_kind.setdefault(sid, []).append(d)

    plan = by_kind[PLAN][0]
    refusal = next(d for d in by_kind[ERROR] if d["error"] == "InsufficientBudget")
    refused = dict(plan, fits=False, refusal=refusal)
    ok = not errors(PLAN, refused)
    print("refusal inside a plan validates through the reference:", ok)
    failed = failed or not ok

    controls = []
    bad = dict(plan)
    bad.pop("fits")
    controls.append(("plan without 'fits'", PLAN, bad))
    bad = dict(refusal, error="Oops")
    controls.append(("unknown error class", ERROR, bad))
    bad = dict(refusal, budget_byte=refusal["budget_bytes"])
    controls.append(("misspelt key 'budget_byte'", ERROR, bad))
    bad = copy.deepcopy(refusal)
    bad["ways_forward"][0].pop("python")
    controls.append(("way forward without 'python'", ERROR, bad))
    bad = dict(plan, fits=False, refusal=dict(refusal, available_bytes="4GB"))
    controls.append(("refusal with a string size, inside a plan", PLAN, bad))
    event = by_kind[EVENT][0]
    controls.append(("progress unit 'groups'", EVENT, dict(event, unit="groups")))
    stop = copy.deepcopy(by_kind[STOP][0])
    stop["stopped"]["reason"] = "callback"
    controls.append(("stop reason 'callback'", STOP, stop))
    for label, sid, doc in controls:
        rejected = bool(errors(sid, doc))
        print(f"negative control, {label} rejected:", rejected)
        failed = failed or not rejected
    return 1 if failed else 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    sys.exit(main(sys.argv[1]))
