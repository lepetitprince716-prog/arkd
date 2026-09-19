use std::sync::LazyLock;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

pub const FILE_PATH_FIELDS: &[&str] = &["filename"];

#[derive(Debug, Deserialize, Clone)]
pub struct TaskSpec {
    pub task_type: String,
    pub summary: String,
    #[serde(default)]
    pub notes: String,
    pub schema: Value,
    pub example: Value,
}

#[derive(Debug, Serialize)]
pub struct TaskSummary {
    pub task_type: String,
    pub summary: String,
    pub required: String,
}

static CATALOG: LazyLock<IndexMap<String, TaskSpec>> = LazyLock::new(|| {
    let specs: Vec<TaskSpec> = serde_json::from_str(include_str!("../../catalog/tasks.json"))
        .expect("catalog/tasks.json is valid JSON");
    specs
        .into_iter()
        .map(|s| (s.task_type.clone(), s))
        .collect()
});

pub fn list() -> Vec<TaskSummary> {
    CATALOG
        .values()
        .map(|s| {
            let required = s
                .schema
                .get("required")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "none".to_string());
            TaskSummary {
                task_type: s.task_type.clone(),
                summary: s.summary.clone(),
                required,
            }
        })
        .collect()
}

pub fn get(name: &str) -> Option<&'static TaskSpec> {
    CATALOG.get(name)
}

pub fn names() -> impl Iterator<Item = &'static str> {
    CATALOG.keys().map(String::as_str)
}

pub fn resolve(task_type: &str) -> Result<&'static str> {
    let lowered = task_type.trim().to_lowercase();
    if let Some(name) = CATALOG.keys().find(|n| n.to_lowercase() == lowered) {
        return Ok(name.as_str());
    }
    let mut scored: Vec<(&str, f64)> = CATALOG
        .keys()
        .map(|n| {
            (
                n.as_str(),
                strsim::normalized_levenshtein(n, task_type)
                    .max(strsim::jaro_winkler(&n.to_lowercase(), &lowered) * 0.95),
            )
        })
        .filter(|(_, s)| *s >= 0.6)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scored.truncate(3);
    let hint = if scored.is_empty() {
        String::new()
    } else {
        format!(
            " Did you mean {}?",
            scored
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join(" or ")
        )
    };
    Err(Error::UnknownTaskType(format!(
        "Unknown task type {task_type:?}.{hint} Known types: {}.",
        CATALOG
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

fn matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn repr(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{s}'"),
        other => other.to_string(),
    }
}

fn check_value(field: &str, value: &Value, schema: &Value, errors: &mut Vec<String>) {
    let expected = schema.get("type");
    match expected {
        Some(Value::Array(types)) => {
            let ok = types
                .iter()
                .filter_map(|t| t.as_str())
                .any(|t| matches_type(value, t));
            if !ok {
                errors.push(format!(
                    "'{field}' must be one of types {}, got {}",
                    serde_json::to_string(types).unwrap_or_default(),
                    type_name(value)
                ));
                return;
            }
        }
        Some(Value::String(t)) if !matches_type(value, t) => {
            errors.push(format!("'{field}' must be a {t}, got {}", type_name(value)));
            return;
        }
        _ => {}
    }

    if let Some(allowed) = schema.get("enum").and_then(|v| v.as_array())
        && !allowed.contains(value)
    {
        let strs: Vec<String> = allowed.iter().map(repr).collect();
        let suggestion = allowed
            .iter()
            .map(|a| {
                (
                    a,
                    strsim::normalized_levenshtein(&a.to_string(), &value.to_string()),
                )
            })
            .filter(|(_, s)| *s >= 0.6)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(a, _)| format!(" Did you mean {}?", repr(a)))
            .unwrap_or_default();
        errors.push(format!(
            "'{field}' must be one of [{}], got {}.{suggestion}",
            strs.join(", "),
            repr(value)
        ));
        return;
    }

    if let Some(n) = value.as_f64()
        && value.is_number()
        && !value.is_boolean()
    {
        if let Some(min) = schema.get("minimum")
            && n < min.as_f64().unwrap()
        {
            errors.push(format!("'{field}' must be >= {min}, got {}", repr(value)));
        }
        if let Some(max) = schema.get("maximum")
            && n > max.as_f64().unwrap()
        {
            errors.push(format!("'{field}' must be <= {max}, got {}", repr(value)));
        }
    }

    if let Value::Array(items) = value {
        if let Some(min_items) = schema.get("minItems").and_then(|v| v.as_u64())
            && (items.len() as u64) < min_items
        {
            errors.push(format!(
                "'{field}' needs at least {min_items} item(s), got {}",
                items.len()
            ));
        }
        if let Some(item_schema) = schema.get("items")
            && item_schema.is_object()
            && item_schema.get("anyOf").is_none()
        {
            for (index, item) in items.iter().enumerate() {
                check_value(&format!("{field}[{index}]"), item, item_schema, errors);
            }
        }
    }
}

pub fn validate(task_type: &str, params: Value) -> Result<(String, Value)> {
    let canonical = resolve(task_type)?;
    let spec = CATALOG.get(canonical).unwrap();

    if !params.is_object() {
        return Err(Error::Validation(format!(
            "Parameters for {canonical} must be a JSON object, got {}.",
            type_name(&params)
        )));
    }
    let params = params.as_object().unwrap();

    let properties = spec
        .schema
        .get("properties")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut errors: Vec<String> = Vec::new();

    if let Some(required) = spec.schema.get("required").and_then(|v| v.as_array()) {
        for field in required.iter().filter_map(|v| v.as_str()) {
            if !params.contains_key(field) {
                errors.push(format!("'{field}' is required for {canonical}"));
            }
        }
    }

    for (field, value) in params {
        let Some(field_schema) = properties.get(field) else {
            let suggestion = properties
                .keys()
                .map(|k| (k, strsim::normalized_levenshtein(k, field)))
                .filter(|(_, s)| *s >= 0.6)
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(k, _)| format!(" Did you mean '{k}'?"))
                .unwrap_or_default();
            errors.push(format!(
                "'{field}' is not a parameter of {canonical}.{suggestion} Valid parameters: {}.",
                properties
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            continue;
        };
        check_value(field, value, field_schema, &mut errors);
    }

    if !errors.is_empty() {
        return Err(Error::Validation(format!(
            "{} problem(s) with {canonical} parameters:\n- {}",
            errors.len(),
            errors.join("\n- ")
        )));
    }

    Ok((canonical.to_string(), Value::Object(params.clone())))
}
