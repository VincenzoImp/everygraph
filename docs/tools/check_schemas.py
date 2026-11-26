#!/usr/bin/env python3
"""Check the JSON Schemas written out in the specifications.

usage: uv run --no-project --with jsonschema \
           python docs/tools/check_schemas.py FILE.md [FILE.md ...]

For example `docs/ux-contract.md` (plan, error, event, stop and file
documents) and `docs/api-sketch.md` (the result and verify documents); given together,
the schemas of both files are checked as one set.

Every ```json block that carries "$schema" must be a valid draft 2020-12
schema; the schemas refer to each other by their "$id", and those references
must resolve. Every other ```json block is an example and must validate
against the schema it belongs to, recognized by its keys: a plan, an error
document (also the form of a refusal inside a plan), an event, a stop
document, a file document, a result document or a verify report. A refusal is also checked
inside a plan with "fits": false, through the plan's reference to the error
schema. Negative controls, for each schema that is present, show that the
schemas reject a missing key, an unknown error class, a misspelt key, a way
forward without its "python" key, an `OutputExists` without "holds", an
unknown progress unit, an unknown stop reason, and a misspelt key in a file
document, in a result document and in a verify report. Exit status 0 when everything holds.
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
FILE = "urn:everygraph:schema:file:1"
RESULT = "urn:everygraph:schema:result:1"
VERIFY = "urn:everygraph:schema:verify:1"
KNOWN = (PLAN, ERROR, EVENT, STOP, FILE, RESULT, VERIFY)


def kind_of(doc):
    """Which schema an example belongs to, from its keys."""
    if "verify" in doc and "intact" in doc:
        return VERIFY
    if "event" in doc:
        return EVENT
    if "stopped" in doc:
        return STOP
    if "operation" in doc and "fits" in doc:
        return PLAN
    if "error" in doc:
        return ERROR
    if "made" in doc:
        return FILE
    if "result" in doc and "columns" in doc:
        return RESULT
    return None


def main(paths):
    docs = []
    for path in paths:
        text = open(path, encoding="utf-8").read()
        docs += [json.loads(b)
                 for b in re.findall(r"```json\n(.*?)\n```", text, re.S)]
    schemas = {d["$id"]: d for d in docs if "$schema" in d}
    examples = [d for d in docs if "$schema" not in d]
    if not schemas:
        raise SystemExit(f"no schema found in {paths}")
    unknown = sorted(set(schemas) - set(KNOWN))
    if unknown:
        raise SystemExit(f"schemas this checker does not know: {unknown}")
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
        if sid is None or sid not in validators:
            print(f"example matches no schema given: {list(d)[:4]}")
            failed = True
            continue
        problems = errors(sid, d)
        for e in problems:
            print(f"  {sid}: {e.json_path}: {e.message}")
        failed = failed or bool(problems)
        name = (d.get("event") or d.get("operation") or d.get("error")
                or d.get("result") or d.get("verify"))
        print(f"{sid.split(':')[3]} example validates: {name}:", not problems)
        by_kind.setdefault(sid, []).append(d)
    for sid in schemas:
        if sid not in by_kind:
            print(f"schema without an example: {sid}")
            failed = True

    controls = []
    if PLAN in by_kind and ERROR in by_kind:
        plan = by_kind[PLAN][0]
        refusal = next(d for d in by_kind[ERROR]
                       if d["error"] == "InsufficientBudget")
        refused = dict(plan, fits=False, refusal=refusal)
        ok = not errors(PLAN, refused)
        print("refusal inside a plan validates through the reference:", ok)
        failed = failed or not ok
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
        exists = [d for d in by_kind[ERROR] if d["error"] == "OutputExists"]
        if exists:
            bad = dict(exists[0])
            bad.pop("holds")
            controls.append(("OutputExists without 'holds'", ERROR, bad))
    if EVENT in by_kind:
        event = by_kind[EVENT][0]
        controls.append(("progress unit 'groups'", EVENT,
                         dict(event, unit="groups")))
    if STOP in by_kind:
        stop = copy.deepcopy(by_kind[STOP][0])
        stop["stopped"]["reason"] = "callback"
        controls.append(("stop reason 'callback'", STOP, stop))
    if FILE in by_kind:
        made = dict(by_kind[FILE][0])
        made["elapsed_second"] = made.pop("elapsed_seconds")
        controls.append(("file document with 'elapsed_second'", FILE, made))
    if RESULT in by_kind:
        result = dict(by_kind[RESULT][0])
        result["rows_totl"] = result.pop("rows_total")
        controls.append(("result document with 'rows_totl'", RESULT, result))
        result = dict(by_kind[RESULT][0], extra=1)
        controls.append(("result document with an unknown key", RESULT, result))
    if VERIFY in by_kind:
        report = dict(by_kind[VERIFY][0])
        report["problem"] = report.pop("problems")
        controls.append(("verify report with 'problem'", VERIFY, report))
    for label, sid, doc in controls:
        rejected = bool(errors(sid, doc))
        print(f"negative control, {label} rejected:", rejected)
        failed = failed or not rejected
    print(f"{'FAIL' if failed else 'ok'}  {len(schemas)} schemas, "
          f"{len(examples)} examples, {len(controls)} negative controls")
    return 1 if failed else 0


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    sys.exit(main(sys.argv[1:]))
