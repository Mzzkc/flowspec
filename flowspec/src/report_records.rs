//! Versioned, lossless sharing of repeated report records on the structured wire.
use serde_json::{json, Value};
use std::collections::BTreeMap;

const ENCODING: &str = "flowspec.report.records.v1";
const AUTHORITY_FIELDS: &[&str] = &["losses", "pattern_state", "provenance"];

const ROOT_FIELDS: &[&str] = &[
    "diagnostics",
    "entities",
    "summary",
    "flows",
    "type_flows",
    "boundaries",
    "dependency_graph",
];

#[derive(Default)]
struct Records {
    nodes: Vec<Value>,
    shapes: Vec<Vec<String>>,
    values: BTreeMap<String, usize>,
    shape_ids: BTreeMap<Vec<String>, usize>,
}
impl Records {
    fn intern(&mut self, value: &Value) -> usize {
        let key = serde_json::to_string(value).expect("report JSON value");
        if let Some(index) = self.values.get(&key) {
            return *index;
        }
        let node = match value {
            Value::Object(fields) => {
                let shape: Vec<_> = fields.keys().cloned().collect();
                let shape_id = if let Some(id) = self.shape_ids.get(&shape) {
                    *id
                } else {
                    let id = self.shapes.len();
                    self.shapes.push(shape.clone());
                    self.shape_ids.insert(shape.clone(), id);
                    id
                };
                let mut node = vec![json!("o"), json!(shape_id)];
                node.extend(shape.iter().map(|key| json!(self.intern(&fields[key]))));
                Value::Array(node)
            }
            Value::Array(items) => {
                let mut node = vec![json!("a")];
                node.extend(items.iter().map(|item| json!(self.intern(item))));
                Value::Array(node)
            }
            scalar => scalar.clone(),
        };
        let index = self.nodes.len();
        self.nodes.push(node);
        self.values.insert(key, index);
        index
    }
}

pub(crate) fn pack_report_records(mut report: Value) -> Value {
    if !report["authority"].is_object() {
        return report;
    }
    let original = report.clone();
    let mut records = Records::default();
    for field in AUTHORITY_FIELDS {
        if report["authority"][field] != json!([]) {
            report["authority"][field] =
                json!({"$record": records.intern(&report["authority"][field])});
        }
    }
    for (parent, field) in [("inventory", "entries"), ("findings", "all")] {
        if report["authority"][parent][field] != json!([]) {
            report["authority"][parent][field] =
                json!({"$record": records.intern(&report["authority"][parent][field])});
        }
    }
    for field in ROOT_FIELDS {
        if report[field] != json!([]) {
            report[field] = json!({"$record": records.intern(&report[field])});
        }
    }
    report["report_records"] =
        json!({"encoding":ENCODING,"shapes":records.shapes,"nodes":records.nodes});
    // Never enlarge small or legacy reports. Expansion equality is a producer
    // precondition, independent of whether the byte saving is worthwhile.
    if expand_report_records(&report).as_ref() != Ok(&original)
        || serde_json::to_vec(&report).expect("report JSON").len()
            >= serde_json::to_vec(&original).expect("report JSON").len()
    {
        return original;
    }
    report
}

fn index(value: &Value) -> Result<usize, String> {
    value
        .as_u64()
        .and_then(|i| usize::try_from(i).ok())
        .ok_or_else(|| "invalid record index".into())
}

fn report_slot<'a>(report: &'a mut Value, path: &[&str]) -> Result<&'a mut Value, String> {
    let mut slot = report;
    for key in path {
        slot = slot
            .as_object_mut()
            .and_then(|fields| fields.get_mut(*key))
            .ok_or_else(|| format!("missing or invalid report field {}", path.join(".")))?;
    }
    Ok(slot)
}

/// Expand `flowspec.report.records.v1` to the complete ordinary report value.
/// Unencoded reports pass through unchanged. Malformed tables fail closed;
/// no partial report, missing finding, or apparently clean result is returned.
pub fn expand_report_records(report: &Value) -> Result<Value, String> {
    let Some(table) = report.get("report_records") else {
        return Ok(report.clone());
    };
    if table["encoding"] != ENCODING || table.as_object().is_none_or(|m| m.len() != 3) {
        return Err("unsupported report record encoding".into());
    }
    let shapes = table["shapes"].as_array().ok_or("missing record shapes")?;
    let nodes = table["nodes"].as_array().ok_or("missing record nodes")?;
    let mut decoded = Vec::<Value>::new();
    let mut sizes = Vec::<usize>::new();
    // A hostile DAG must not expand exponentially without a bound. This is a
    // reader resource limit, not an analysis scope/filter or emitted-byte waiver.
    const MAX_EXPANSION: usize = 64 * 1024 * 1024;
    let mut total_expansion = 0usize;
    for node in nodes {
        let mut size = 0usize;
        let value = if let Some(row) = node.as_array() {
            let get = |reference: &Value| -> Result<(Value, usize), String> {
                let id = index(reference)?;
                Ok((
                    decoded
                        .get(id)
                        .ok_or("forward or missing record reference")?
                        .clone(),
                    *sizes.get(id).ok_or("missing record size")?,
                ))
            };
            match row.first().and_then(Value::as_str) {
                Some("a") => {
                    let mut values = Vec::new();
                    for reference in &row[1..] {
                        let (value, bytes) = get(reference)?;
                        size = size
                            .checked_add(bytes + 1)
                            .ok_or("record expansion overflow")?;
                        if size > MAX_EXPANSION {
                            return Err("report record expansion limit".into());
                        }
                        values.push(value);
                    }
                    Value::Array(values)
                }
                Some("o") if row.len() >= 2 => {
                    let shape = shapes
                        .get(index(&row[1])?)
                        .and_then(Value::as_array)
                        .ok_or("invalid record shape")?;
                    if shape.len() != row.len() - 2 {
                        return Err("record shape arity mismatch".into());
                    }
                    let mut fields = serde_json::Map::new();
                    for (key, reference) in shape.iter().zip(&row[2..]) {
                        let key = key.as_str().ok_or("invalid record key")?;
                        let (value, bytes) = get(reference)?;
                        size = size
                            .checked_add(bytes + key.len() + 6)
                            .ok_or("record expansion overflow")?;
                        if size > MAX_EXPANSION {
                            return Err("report record expansion limit".into());
                        }
                        if fields.insert(key.to_string(), value).is_some() {
                            return Err("duplicate record key".into());
                        }
                    }
                    Value::Object(fields)
                }
                _ => return Err("invalid record node".into()),
            }
        } else if node.is_object() {
            return Err("invalid scalar record node".into());
        } else {
            size = serde_json::to_vec(node).map_err(|e| e.to_string())?.len();
            node.clone()
        };
        total_expansion = total_expansion
            .checked_add(size + 2)
            .ok_or("record expansion overflow")?;
        if total_expansion > MAX_EXPANSION {
            return Err("report record expansion limit".into());
        }
        decoded.push(value);
        sizes.push(size + 2);
    }
    let mut result = report.clone();
    result
        .as_object_mut()
        .ok_or("report must be object")?
        .remove("report_records");
    for (parent, fields) in [("authority", AUTHORITY_FIELDS), ("", ROOT_FIELDS)] {
        for field in fields {
            let slot = if parent.is_empty() {
                report_slot(&mut result, &[*field])?
            } else {
                report_slot(&mut result, &[parent, *field])?
            };
            if *slot == json!([]) {
                continue;
            }
            let wrapper = slot.as_object().ok_or("missing record wrapper")?;
            if wrapper.len() != 1 {
                return Err("invalid record wrapper".into());
            }
            let id = index(wrapper.get("$record").ok_or("missing record reference")?)?;
            *slot = decoded.get(id).ok_or("missing root record")?.clone();
        }
    }
    for (parent, field) in [("inventory", "entries"), ("findings", "all")] {
        let slot = report_slot(&mut result, &["authority", parent, field])?;
        if *slot == json!([]) {
            continue;
        }
        let wrapper = slot.as_object().ok_or("missing record wrapper")?;
        if wrapper.len() != 1 {
            return Err("invalid record wrapper".into());
        }
        let id = index(wrapper.get("$record").ok_or("missing record reference")?)?;
        *slot = decoded.get(id).ok_or("missing root record")?.clone();
    }
    Ok(result)
}
