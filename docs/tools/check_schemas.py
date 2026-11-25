#!/usr/bin/env python3
"""Check the JSON Schemas written out in docs/ux-contract.md.

usage: uv run --no-project --with jsonschema \
           python docs/tools/check_schemas.py docs/ux-contract.md

Every ```json block that carries "$schema" must be a valid draft 2020-12
schema; every other ```json block is an example and must validate against
the schema it belongs to (a plan, an error document, or the refusal part of
a plan). Two negative controls show the schemas can reject a document.
Exit status 0 when everything holds.
"""
import json
import re
import sys

from jsonschema import Draft202012Validator

PLAN = "urn:everygraph:schema:plan:1"
ERROR = "urn:everygraph:schema:error:1"


def main(path):
    text = open(path, encoding="utf-8").read()
    docs = [json.loads(b) for b in re.findall(r"```json\n(.*?)\n```", text, re.S)]
    schemas = {d["$id"]: d for d in docs if "$schema" in d}
    examples = [d for d in docs if "$schema" not in d]
    for sid, schema in schemas.items():
        Draft202012Validator.check_schema(schema)
        print("valid schema:", sid)
    plan, error = schemas[PLAN], schemas[ERROR]
    refusal = {"$schema": plan["$schema"], "$ref": "#/$defs/refusal",
               "$defs": plan["$defs"]}
    for d in examples:
        if d.get("schema") == 1 and "operation" in d:
            Draft202012Validator(plan).validate(d)
            print("plan example validates:", d["operation"])
        elif d.get("schema") == 1 and "error" in d:
            Draft202012Validator(error).validate(d)
            print("error example validates:", d["error"])
        elif "error" in d and "available_bytes" in d:
            Draft202012Validator(refusal).validate(d)
            print("refusal example validates:", d["error"])
        else:
            raise SystemExit(f"example matches no schema: {list(d)[:4]}")

    bad = dict(next(d for d in examples if "operation" in d))
    bad.pop("fits")
    rejected = bool(list(Draft202012Validator(plan).iter_errors(bad)))
    print("negative control, plan without 'fits' rejected:", rejected)
    bad2 = dict(next(d for d in examples if d.get("schema") == 1 and "error" in d))
    bad2["error"] = "Oops"
    rejected2 = bool(list(Draft202012Validator(error).iter_errors(bad2)))
    print("negative control, unknown error class rejected:", rejected2)
    return 0 if rejected and rejected2 else 1


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    sys.exit(main(sys.argv[1]))
