use super::types::N8nWorkflowConfig;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum N8nSchemaValidationError {
    #[error("n8n schema reference is empty for workflow {workflow_id}")]
    EmptySchemaRef { workflow_id: String },
    #[error("failed to read n8n schema {path}: {source}")]
    SchemaRead {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse n8n schema {path}: {source}")]
    SchemaParse {
        path: String,
        source: serde_json::Error,
    },
    #[error("n8n payload does not satisfy {schema_ref}: {issues}")]
    Payload { schema_ref: String, issues: String },
}

pub fn validate_n8n_input_payload(
    workflow: &N8nWorkflowConfig,
    payload: &Value,
) -> Result<(), N8nSchemaValidationError> {
    validate_against_schema_ref(workflow, &workflow.input_schema_ref, payload)
}

pub fn validate_n8n_output_evidence(
    workflow: &N8nWorkflowConfig,
    evidence_log: &[Value],
) -> Result<(), N8nSchemaValidationError> {
    let schema = workflow.output_schema_ref.trim();
    if schema.is_empty() {
        return Err(N8nSchemaValidationError::EmptySchemaRef {
            workflow_id: workflow.workflow_id.clone(),
        });
    }

    if evidence_log.is_empty() {
        return Err(N8nSchemaValidationError::Payload {
            schema_ref: schema.to_string(),
            issues: "no callback evidence was recorded".into(),
        });
    }

    let loaded = load_schema(schema)?;
    let mut failures = Vec::new();
    for evidence in evidence_log.iter().rev() {
        let issues = validate_value(&loaded, evidence, "$");
        if issues.is_empty() {
            return Ok(());
        }
        failures.push(issues.join("; "));
    }

    Err(N8nSchemaValidationError::Payload {
        schema_ref: schema.to_string(),
        issues: failures
            .into_iter()
            .next()
            .unwrap_or_else(|| "callback evidence did not match output schema".into()),
    })
}

pub fn input_payload_validation_issues(
    workflow: &N8nWorkflowConfig,
    payload: &Value,
) -> Vec<String> {
    validate_n8n_input_payload(workflow, payload)
        .err()
        .map(schema_error_issues)
        .unwrap_or_default()
}

pub fn schema_error_issues(error: N8nSchemaValidationError) -> Vec<String> {
    match error {
        N8nSchemaValidationError::Payload { issues, .. } => issues
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        other => vec![other.to_string()],
    }
}

fn validate_against_schema_ref(
    workflow: &N8nWorkflowConfig,
    schema_ref: &str,
    payload: &Value,
) -> Result<(), N8nSchemaValidationError> {
    let schema_ref = schema_ref.trim();
    if schema_ref.is_empty() {
        return Err(N8nSchemaValidationError::EmptySchemaRef {
            workflow_id: workflow.workflow_id.clone(),
        });
    }

    let schema = load_schema(schema_ref)?;
    let issues = validate_value(&schema, payload, "$");
    if issues.is_empty() {
        Ok(())
    } else {
        Err(N8nSchemaValidationError::Payload {
            schema_ref: schema_ref.to_string(),
            issues: issues.join("; "),
        })
    }
}

fn load_schema(schema_ref: &str) -> Result<Value, N8nSchemaValidationError> {
    let path = resolve_schema_path(schema_ref).unwrap_or_else(|| PathBuf::from(schema_ref));
    let body =
        std::fs::read_to_string(&path).map_err(|source| N8nSchemaValidationError::SchemaRead {
            path: path.display().to_string(),
            source,
        })?;
    serde_json::from_str(&body).map_err(|source| N8nSchemaValidationError::SchemaParse {
        path: path.display().to_string(),
        source,
    })
}

fn resolve_schema_path(schema_ref: &str) -> Option<PathBuf> {
    let path = Path::new(schema_ref);
    if path.is_absolute() && path.exists() {
        return Some(path.to_path_buf());
    }

    if let Ok(cwd) = std::env::current_dir() {
        let direct = cwd.join(schema_ref);
        if direct.exists() {
            return Some(direct);
        }
        for ancestor in cwd.ancestors().take(8) {
            let candidate = ancestor.join(schema_ref);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for base in [
        manifest_dir.as_path(),
        manifest_dir.parent().unwrap_or(manifest_dir.as_path()),
        manifest_dir
            .parent()
            .and_then(Path::parent)
            .unwrap_or(manifest_dir.as_path()),
    ] {
        let candidate = base.join(schema_ref);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn validate_value(schema: &Value, value: &Value, path: &str) -> Vec<String> {
    let mut issues = Vec::new();

    if let Some(expected_type) = schema.get("type").and_then(Value::as_str) {
        if !value_matches_type(value, expected_type) {
            issues.push(format!("{path} must be {expected_type}"));
            return issues;
        }
    }

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        if let Some(object) = value.as_object() {
            for field in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(field) {
                    issues.push(format!("missing required field: {field}"));
                }
            }
        }
    }

    if let (Some(properties), Some(object)) = (
        schema.get("properties").and_then(Value::as_object),
        value.as_object(),
    ) {
        if schema
            .get("additionalProperties")
            .and_then(Value::as_bool)
            .is_some_and(|allowed| !allowed)
        {
            for key in object.keys() {
                if !properties.contains_key(key) {
                    issues.push(format!("unexpected field: {key}"));
                }
            }
        }

        for (key, property_schema) in properties {
            if let Some(child) = object.get(key) {
                issues.extend(validate_value(
                    property_schema,
                    child,
                    &format!("{path}.{key}"),
                ));
            }
        }
    }

    if let Some(items) = value.as_array() {
        if let Some(min_items) = schema.get("minItems").and_then(Value::as_u64) {
            if items.len() < min_items as usize {
                issues.push(format!("{path} must contain at least {min_items} item(s)"));
            }
        }
        if let Some(max_items) = schema.get("maxItems").and_then(Value::as_u64) {
            if items.len() > max_items as usize {
                issues.push(format!("{path} must contain at most {max_items} item(s)"));
            }
        }
        if let Some(item_schema) = schema.get("items") {
            for (index, item) in items.iter().enumerate() {
                issues.extend(validate_value(
                    item_schema,
                    item,
                    &format!("{path}[{index}]"),
                ));
            }
        }
    }

    if let Some(number) = value.as_i64().or_else(|| value.as_u64().map(|v| v as i64)) {
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_i64) {
            if number < minimum {
                issues.push(format!("{path} must be >= {minimum}"));
            }
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_i64) {
            if number > maximum {
                issues.push(format!("{path} must be <= {maximum}"));
            }
        }
    }

    issues
}

fn value_matches_type(value: &Value, expected_type: &str) -> bool {
    match expected_type {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "null" => value.is_null(),
        _ => true,
    }
}
