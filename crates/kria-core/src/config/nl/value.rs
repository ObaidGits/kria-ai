//! Universal, schema-driven value extraction (settings-nl-intelligence Wave 1).
//!
//! The field's TYPE is inferred from the real `KriaConfig` serde shape (zero
//! per-field hardcoding — a new field is typed automatically); enums + their
//! allowed values come from `FieldMeta`. This replaces the old hand-coded
//! `resolve_value` hints and generalizes to numbers, floats, booleans, and enums
//! phrased naturally (spaces/underscores/case), for ANY field.
//!
//! Scope of this increment: bool/int/float/enum/string extraction. Range bounds,
//! URL/path/duration/lang aliases, and LLM-constrained fallback land with
//! `FieldMeta` v2 (Task 1/2) — this module is the seam they slot into.

use crate::config::{schema, KriaConfig};

/// Inferred value type for a config field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Bool,
    Int,
    Float,
    /// A closed set (`FieldMeta.valid_values`) — includes boolean-as-enum.
    Enum,
    Str,
    List,
    Unknown,
}

/// Infer a field's value kind. Enum wins when the schema declares a closed set;
/// otherwise the kind comes from the field's default serde value (its real type).
pub fn value_kind(section: &str, field: &str) -> ValueKind {
    if schema::field_meta(section, field).valid_values.is_some() {
        return ValueKind::Enum;
    }
    if let Ok(root) = serde_json::to_value(KriaConfig::default()) {
        if let Some(v) = root.get(section).and_then(|s| s.get(field)) {
            return match v {
                serde_json::Value::Bool(_) => ValueKind::Bool,
                serde_json::Value::Number(n) => {
                    if n.is_f64() && n.as_i64().is_none() && n.as_u64().is_none() {
                        ValueKind::Float
                    } else {
                        ValueKind::Int
                    }
                }
                serde_json::Value::String(_) => ValueKind::Str,
                serde_json::Value::Array(_) => ValueKind::List,
                _ => ValueKind::Unknown,
            };
        }
    }
    ValueKind::Unknown
}

/// Extract a value for `(section, field)` from natural text, driven by the field
/// type + schema. Returns `None` when no value is present (the caller then asks a
/// clarifying question). For an enum with an unmatched token it returns the raw
/// word so the handler can reject it with the allowed-values list (grounded reask).
pub fn extract(section: &str, field: &str, text: &str) -> Option<serde_json::Value> {
    let norm = text.to_ascii_lowercase();
    let meta = schema::field_meta(section, field);
    match value_kind(section, field) {
        ValueKind::Enum => extract_enum(meta.valid_values.unwrap_or(&[]), &norm),
        ValueKind::Bool => extract_bool(&norm).map(|b| serde_json::json!(b)),
        ValueKind::Int => extract_number(&norm).map(|n| serde_json::json!(n.round() as i64)),
        ValueKind::Float => extract_number(&norm).map(|n| serde_json::json!(n)),
        ValueKind::Str => extract_string(&norm).map(|s| serde_json::json!(s)),
        ValueKind::List | ValueKind::Unknown => None,
    }
}

/// Transitional value-alias data (natural phrase → canonical enum value). This is
/// DATA, not control flow, and migrates into `FieldMeta.aliases` in Task 1 so it is
/// declared next to the field (fully schema-driven, no module-level table).
const VALUE_ALIASES: &[(&str, &str)] = &[
    ("night", "dark"),
    ("local", "local_only"),
    ("cloud", "cloud_only"),
];

/// Value-word tokens too generic to distinguish an enum variant on their own.
const VALUE_STOPWORDS: &[&str] = &["mode", "only", "with", "auto", "true", "false"];

fn extract_enum(vals: &[&str], norm: &str) -> Option<serde_json::Value> {
    if vals.is_empty() {
        return None;
    }
    // Boolean-as-enum: map on/off/enable/disable/yes/no/true/false.
    if vals == ["true", "false"] {
        return extract_bool(norm).map(|b| serde_json::json!(b));
    }
    // Strong: the canonical value (or its spaced form) appears verbatim.
    for v in vals {
        let lv = v.to_ascii_lowercase();
        if norm.contains(&lv) || norm.contains(&lv.replace('_', " ")) {
            return Some(serde_json::json!(*v));
        }
    }
    // Alias data (transitional): natural phrase → canonical, only if allowed here.
    for (needle, canon) in VALUE_ALIASES {
        if vals.contains(canon) && norm.contains(needle) {
            return Some(serde_json::json!(*canon));
        }
    }
    // Distinctive token: a meaningful word of the value appears as a whole word.
    for v in vals {
        for tok in v.split('_') {
            if tok.len() >= 4
                && !VALUE_STOPWORDS.contains(&tok)
                && norm.split(|c: char| !c.is_alphanumeric()).any(|w| w == tok)
            {
                return Some(serde_json::json!(*v));
            }
        }
    }
    // Unmatched value word → raw, so the handler rejects with allowed values.
    raw_value_after_connector(norm).map(|w| serde_json::json!(w))
}

fn extract_bool(norm: &str) -> Option<bool> {
    // Check OFF first ("turn off" contains the "on" substring otherwise).
    let off = [
        "disable",
        "turn off",
        "deactivate",
        "switch off",
        " off",
        "false",
        " no ",
        "stop",
        "mute",
        "unset",
    ];
    let on = [
        "enable",
        "turn on",
        "activate",
        "switch on",
        " on",
        "true",
        " yes",
        "start",
        "unmute",
    ];
    if off.iter().any(|k| norm.contains(k)) {
        return Some(false);
    }
    if on.iter().any(|k| norm.contains(k)) {
        return Some(true);
    }
    None
}

/// Extract the first parseable number in the text (int or float).
fn extract_number(norm: &str) -> Option<f64> {
    for tok in norm.split(|c: char| c.is_whitespace() || c == '=' || c == ':') {
        let cleaned: String = tok
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect();
        if cleaned.is_empty() || cleaned == "-" || cleaned == "." {
            continue;
        }
        if let Ok(n) = cleaned.parse::<f64>() {
            return Some(n);
        }
    }
    None
}

/// The value word following a connector (`to`/`=`/`:`), for enum grounding + strings.
fn extract_string(norm: &str) -> Option<String> {
    raw_value_after_connector(norm)
}

fn raw_value_after_connector(norm: &str) -> Option<String> {
    for sep in [" to ", " = ", ": ", "="] {
        if let Some(idx) = norm.find(sep) {
            let rest = norm[idx + sep.len()..].trim();
            let word = rest
                .split(|c: char| c.is_whitespace())
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| {
                    !c.is_alphanumeric() && c != '_' && c != '.' && c != '/' && c != ':'
                });
            if !word.is_empty() {
                return Some(word.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_kind_from_real_config_shape() {
        // Enum via valid_values.
        assert_eq!(value_kind("ui", "theme"), ValueKind::Enum);
        // Booleans are declared as valid_values ["true","false"] → Enum.
        assert_eq!(value_kind("voice", "enabled"), ValueKind::Enum);
        // Free numeric field (no valid_values) → inferred Int from the struct type.
        assert_eq!(value_kind("agent", "max_tool_rounds"), ValueKind::Int);
        // Free string field → Str.
        assert_eq!(value_kind("search", "searxng_url"), ValueKind::Str);
    }

    #[test]
    fn enum_matches_verbatim_and_alias_and_spacing() {
        assert_eq!(
            extract("ui", "theme", "change theme to dark"),
            Some(serde_json::json!("dark"))
        );
        // Alias: "night" → dark (transitional data).
        assert_eq!(
            extract("ui", "theme", "switch to night mode"),
            Some(serde_json::json!("dark"))
        );
        // Underscore/spacing normalization for a multi-word enum value.
        assert_eq!(
            extract("voice", "mode", "use push to talk"),
            Some(serde_json::json!("push_to_talk"))
        );
        // image_mode alias "local" → local_only.
        assert_eq!(
            extract("image_generation", "image_mode", "generate image locally"),
            Some(serde_json::json!("local_only"))
        );
    }

    #[test]
    fn invalid_enum_returns_raw_for_grounded_reject() {
        assert_eq!(
            extract("ui", "theme", "set theme to rainbow"),
            Some(serde_json::json!("rainbow"))
        );
    }

    #[test]
    fn boolean_on_off_mapping() {
        assert_eq!(
            extract("voice", "enabled", "turn off voice"),
            Some(serde_json::json!(false))
        );
        assert_eq!(
            extract("voice", "enabled", "enable voice"),
            Some(serde_json::json!(true))
        );
    }

    #[test]
    fn numeric_extraction_generalizes_without_per_field_code() {
        // The KEY fix (B1): a free numeric field is now settable from natural text.
        assert_eq!(
            extract("agent", "max_tool_rounds", "set max tool rounds to 8"),
            Some(serde_json::json!(8))
        );
    }

    #[test]
    fn no_value_present_returns_none() {
        assert_eq!(
            extract("agent", "max_tool_rounds", "what are my tool rounds"),
            None
        );
        assert_eq!(extract("ui", "theme", "tell me about themes"), None);
    }
}
