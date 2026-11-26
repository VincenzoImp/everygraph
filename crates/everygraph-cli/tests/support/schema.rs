// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The documents of the command checked against the schemas as the specifications
//! write them: every ```` ```json ```` block with a `$schema` key in
//! `docs/ux-contract.md` and `docs/api-sketch.md`, closed as they are, each found by its
//! `$id` (`urn:everygraph:schema:result:1`, …), with the others in the registry for
//! their references.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;

fn schemas() -> &'static BTreeMap<String, Value> {
    static SCHEMAS: OnceLock<BTreeMap<String, Value>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        let mut found = BTreeMap::new();
        for name in ["ux-contract.md", "api-sketch.md"] {
            let text = std::fs::read_to_string(docs.join(name))
                .unwrap()
                .replace("\r\n", "\n");
            for block in text.split("```json\n").skip(1) {
                let body = block.split("\n```").next().unwrap();
                let value: Value = serde_json::from_str(body).unwrap();
                if let Some(id) = value.get("$id").and_then(Value::as_str) {
                    if value.get("$schema").is_some() {
                        found.insert(id.to_owned(), value.clone());
                    }
                }
            }
        }
        found
    })
}

/// Checks `doc` against the schema `urn:everygraph:schema:NAME:1`.
pub fn check(name: &str, doc: &Value) -> Result<(), String> {
    let id = format!("urn:everygraph:schema:{name}:1");
    let all = schemas();
    let schema = all.get(&id).ok_or(format!("no schema {id} in the docs"))?;
    let mut registry = jsonschema::Registry::new();
    for (uri, s) in all {
        registry = registry.add(uri, s.clone()).map_err(|e| e.to_string())?;
    }
    let registry = registry.prepare().map_err(|e| e.to_string())?;
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(schema)
        .map_err(|e| e.to_string())?;
    let errors: Vec<String> = validator
        .iter_errors(doc)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("{id}: {}", errors.join("; ")))
    }
}
