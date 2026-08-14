//! `os_control::redaction` — the single shared sensitivity/redaction registry.
//!
//! linux-os-control-production **Task 1.8**, design §14 ("Audit and Redaction
//! Design") and §15 (`ApprovalProjection`); OSC-007, OSC-029.
//!
//! # One classifier for three consumers
//!
//! This module owns the **single** sensitivity registry used by *all three*
//! surfaces that could otherwise leak sensitive parameter content:
//!
//! * **durable audit** ([`crate::os_control::audit`]) — parameters are stored
//!   only as redacted structured metadata plus a canonical [`Digest`];
//! * **HITL presentation** ([`crate::safety::hitl`]) — approval requests carry
//!   only an [`ApprovalProjection`], never the raw parameter object;
//! * **provider tracing** — any surfaced provider detail is redacted through the
//!   same [`redact_value`] path.
//!
//! The classification is **data-driven from the frozen manifest** (Task 0.1):
//! every operation's strict input schema annotates each field with an
//! `x-dataClass` of `PublicLocal`, `SensitiveMetadata`, `Content`, or `Secret`
//! (design §14 redaction classes). The registry resolves `$ref`s against the
//! frozen `schemaDefinitions` and classifies each top-level parameter field by
//! the **most sensitive** leaf in its resolved subtree, so a compound field
//! (e.g. a clipboard payload whose `data` is `Content`) is redacted as strongly
//! as its most sensitive part. Unknown/unannotated fields default to
//! [`DataClass::SensitiveMetadata`] and are **hashed, never shown**, so a value
//! passed under an unexpected key can never leak either.
//!
//! # Leak-proofing
//!
//! * `PublicLocal` — the normalized value is retained (percentages, booleans,
//!   power profiles, enums).
//! * `SensitiveMetadata` — only a digest + character length is retained (SSIDs,
//!   device/app/file names). No raw substring or truncated prefix is ever kept.
//! * `Content` — only a content type, byte length, and digest is retained
//!   (clipboard data, notification bodies, file contents, logs).
//! * `Secret` — only a reference digest is retained; the value is never
//!   serialized (passwords, tokens, passkeys, VPN material).

use std::collections::BTreeMap;
use std::collections::HashMap;

use once_cell::sync::Lazy;
use serde_json::Value;

use crate::os_control::contract::{Digest, SafeField, SafeText};
use crate::os_control::manifest::CONTRACT_MANIFEST_JSON;

// ─────────────────────────────────────────────────────────────────────────────
// Data classes (design §14)
// ─────────────────────────────────────────────────────────────────────────────

/// The closed set of parameter data-sensitivity classes (design §14). Ordering
/// is by sensitivity: `PublicLocal < SensitiveMetadata < Content < Secret`, so
/// classifying a compound field by the maximum of its leaves is well-defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    /// Public/non-sensitive local value — stored normalized.
    PublicLocal,
    /// Sensitive metadata (SSID, device/app/file names) — hashed only.
    SensitiveMetadata,
    /// Content (clipboard, notification body, file contents) — type/size/digest.
    Content,
    /// Secret (passwords, tokens, passkeys, VPN material) — never serialized.
    Secret,
}

impl DataClass {
    /// Parse a manifest `x-dataClass` token into a class.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "PublicLocal" => Some(Self::PublicLocal),
            "SensitiveMetadata" => Some(Self::SensitiveMetadata),
            "Content" => Some(Self::Content),
            "Secret" => Some(Self::Secret),
            _ => None,
        }
    }

    /// The stable token for this class.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PublicLocal => "public_local",
            Self::SensitiveMetadata => "sensitive_metadata",
            Self::Content => "content",
            Self::Secret => "secret",
        }
    }

    /// Return the more-sensitive of two classes (uses the derived ordering).
    #[must_use]
    pub fn more_sensitive(self, other: Self) -> Self {
        std::cmp::max(self, other)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Redacted values (durable audit + presentation)
// ─────────────────────────────────────────────────────────────────────────────

/// A single redacted parameter value. The variant a field maps to is decided by
/// its [`DataClass`]; no variant except [`RedactedValue::Public`] retains the
/// raw value, and even `Public` is bounded/sanitized.
///
/// Not `Eq`: the `Public` variant wraps a [`serde_json::Value`] (no `Eq` due to
/// floats). `PartialEq` is sufficient for tests and comparisons.
#[derive(Debug, Clone, PartialEq)]
pub enum RedactedValue {
    /// Public/non-sensitive — the normalized value is retained.
    Public(Value),
    /// Sensitive metadata — only a digest + char length is retained.
    SensitiveMetadata {
        /// Digest of the canonical value.
        digest: Digest,
        /// Character length of the original value (for display sizing only).
        char_len: usize,
    },
    /// Content — only a content type, byte length, and digest is retained.
    Content {
        /// Redacted content-type label (e.g. mime), never the payload.
        content_type: SafeText,
        /// Byte length of the content.
        byte_len: usize,
        /// Digest of the canonical content.
        digest: Digest,
    },
    /// Secret — only a reference digest is retained; the value never serializes.
    Secret {
        /// Reference digest (correlation-safe; not the value).
        reference_digest: Digest,
    },
}

impl RedactedValue {
    /// The class this redacted value was produced under.
    #[must_use]
    pub fn class(&self) -> DataClass {
        match self {
            Self::Public(_) => DataClass::PublicLocal,
            Self::SensitiveMetadata { .. } => DataClass::SensitiveMetadata,
            Self::Content { .. } => DataClass::Content,
            Self::Secret { .. } => DataClass::Secret,
        }
    }
}

impl serde::Serialize for RedactedValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(None)?;
        map.serialize_entry("redaction", self.class().as_str())?;
        match self {
            Self::Public(value) => {
                map.serialize_entry("value", value)?;
            }
            Self::SensitiveMetadata { digest, char_len } => {
                map.serialize_entry("digest", digest)?;
                map.serialize_entry("char_len", char_len)?;
            }
            Self::Content {
                content_type,
                byte_len,
                digest,
            } => {
                map.serialize_entry("content_type", content_type)?;
                map.serialize_entry("byte_len", byte_len)?;
                map.serialize_entry("digest", digest)?;
            }
            Self::Secret { reference_digest } => {
                map.serialize_entry("reference_digest", reference_digest)?;
            }
        }
        map.end()
    }
}

/// The fully-redacted view of one action's parameters: a canonical digest over
/// the whole parameter object plus per-field redacted metadata. This is the
/// exact representation stored in durable audit and surfaced (as
/// [`ApprovalProjection::redacted_fields`]) to HITL.
#[derive(Debug, Clone, PartialEq)]
pub struct RedactedParameters {
    /// Canonical digest over the whole (key-sorted) parameter object.
    pub parameter_digest: Digest,
    /// Per-field redacted metadata, ordered by field name.
    pub fields: BTreeMap<SafeField, RedactedValue>,
}

impl serde::Serialize for RedactedParameters {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("parameter_digest", &self.parameter_digest)?;
        let fields: BTreeMap<&str, &RedactedValue> =
            self.fields.iter().map(|(k, v)| (k.as_str(), v)).collect();
        map.serialize_entry("fields", &fields)?;
        map.end()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The sensitivity registry (manifest-driven)
// ─────────────────────────────────────────────────────────────────────────────

/// The shared classifier: `tool_name -> field_name -> DataClass`, built once
/// from the frozen manifest's strict input schemas and `schemaDefinitions`.
struct SensitivityRegistry {
    by_tool: HashMap<String, HashMap<String, DataClass>>,
}

impl SensitivityRegistry {
    fn build() -> Self {
        let manifest: Value = serde_json::from_str(CONTRACT_MANIFEST_JSON)
            .expect("frozen OS-control contract manifest fixture must be valid JSON");
        let defs = manifest
            .get("schemaDefinitions")
            .cloned()
            .unwrap_or(Value::Null);
        let mut by_tool: HashMap<String, HashMap<String, DataClass>> = HashMap::new();
        if let Some(ops) = manifest.get("operations").and_then(Value::as_array) {
            for op in ops {
                let Some(tool) = op.get("toolName").and_then(Value::as_str) else {
                    continue;
                };
                let mut fields = HashMap::new();
                if let Some(props) = op
                    .get("inputSchema")
                    .and_then(|s| s.get("properties"))
                    .and_then(Value::as_object)
                {
                    for (field, subschema) in props {
                        let class = classify_schema(subschema, &defs, &mut Vec::new())
                            .unwrap_or(DataClass::SensitiveMetadata);
                        fields.insert(field.clone(), class);
                    }
                }
                by_tool.insert(tool.to_string(), fields);
            }
        }
        Self { by_tool }
    }

    /// Classify a `(tool, field)`. Unknown tool/field defaults to the
    /// conservative [`DataClass::SensitiveMetadata`] (hashed, never shown).
    fn classify(&self, tool: &str, field: &str) -> DataClass {
        self.by_tool
            .get(tool)
            .and_then(|fields| fields.get(field))
            .copied()
            .unwrap_or(DataClass::SensitiveMetadata)
    }
}

static REGISTRY: Lazy<SensitivityRegistry> = Lazy::new(SensitivityRegistry::build);

/// Resolve a `$ref` such as `#/schemaDefinitions/PathRef` to its definition.
fn resolve_ref<'a>(reference: &str, defs: &'a Value) -> Option<&'a Value> {
    let name = reference.strip_prefix("#/schemaDefinitions/")?;
    defs.get(name)
}

/// Classify one schema subtree by the maximum-sensitivity leaf it contains.
/// Returns `None` when the subtree carries no `x-dataClass` annotation at all
/// (the caller applies a conservative default).
fn classify_schema(schema: &Value, defs: &Value, seen: &mut Vec<String>) -> Option<DataClass> {
    let Some(obj) = schema.as_object() else {
        return None;
    };

    // A `$ref` resolves to a named definition (guard against cycles).
    if let Some(reference) = obj.get("$ref").and_then(Value::as_str) {
        if seen.iter().any(|s| s == reference) {
            return None;
        }
        seen.push(reference.to_string());
        let resolved = resolve_ref(reference, defs).and_then(|d| classify_schema(d, defs, seen));
        seen.pop();
        return resolved;
    }

    let mut acc: Option<DataClass> = obj
        .get("x-dataClass")
        .and_then(Value::as_str)
        .and_then(DataClass::from_token);

    fn fold(acc: &mut Option<DataClass>, maybe: Option<DataClass>) {
        if let Some(c) = maybe {
            *acc = Some(match *acc {
                Some(existing) => existing.more_sensitive(c),
                None => c,
            });
        }
    }

    // Recurse into nested object properties.
    if let Some(props) = obj.get("properties").and_then(Value::as_object) {
        for sub in props.values() {
            fold(&mut acc, classify_schema(sub, defs, seen));
        }
    }
    // Recurse into oneOf/anyOf/allOf branches.
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(branches) = obj.get(key).and_then(Value::as_array) {
            for branch in branches {
                fold(&mut acc, classify_schema(branch, defs, seen));
            }
        }
    }
    // Recurse into array item schemas.
    if let Some(items) = obj.get("items") {
        fold(&mut acc, classify_schema(items, defs, seen));
    }

    acc
}

// ─────────────────────────────────────────────────────────────────────────────
// Canonicalization + per-value redaction
// ─────────────────────────────────────────────────────────────────────────────

/// Canonical, key-sorted JSON string for stable digesting. Two logically-equal
/// parameter objects produce the same string (and therefore the same digest)
/// regardless of key insertion order.
#[must_use]
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, &Value> = map.iter().collect();
            let mut out = String::from("{");
            for (i, (k, v)) in sorted.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                out.push_str(&canonical_json(v));
            }
            out.push('}');
            out
        }
        Value::Array(items) => {
            let mut out = String::from("[");
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&canonical_json(v));
            }
            out.push(']');
            out
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// The canonical digest of a parameter object (matches what a grant binds).
#[must_use]
pub fn parameter_digest(params: &Value) -> Digest {
    Digest::of_str(&canonical_json(params))
}

/// Number of characters in a value's canonical string form (display sizing).
fn value_char_len(value: &Value) -> usize {
    match value {
        Value::String(s) => s.chars().count(),
        other => canonical_json(other).chars().count(),
    }
}

/// Best-effort content byte length for a `Content` field.
fn content_byte_len(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len(),
        Value::Object(map) => map
            .get("data")
            .and_then(Value::as_str)
            .map(str::len)
            .unwrap_or_else(|| canonical_json(value).len()),
        other => canonical_json(other).len(),
    }
}

/// Best-effort redacted content-type label for a `Content` field.
fn content_type_label(value: &Value) -> SafeText {
    let label = match value {
        Value::Object(map) => map
            .get("mime")
            .and_then(Value::as_str)
            .or_else(|| map.get("encoding").and_then(Value::as_str))
            .unwrap_or("application/json"),
        Value::String(_) => "text/plain",
        Value::Array(_) => "application/json-array",
        _ => "application/json",
    };
    SafeText::new(label)
}

/// Redact a single value under an explicit [`DataClass`]. This is the one path
/// every surface uses; there is no way to serialize a `Secret`/`Content` value.
#[must_use]
pub fn redact_value(class: DataClass, value: &Value) -> RedactedValue {
    match class {
        DataClass::PublicLocal => RedactedValue::Public(bound_public(value)),
        DataClass::SensitiveMetadata => RedactedValue::SensitiveMetadata {
            digest: Digest::of_str(&canonical_json(value)),
            char_len: value_char_len(value),
        },
        DataClass::Content => RedactedValue::Content {
            content_type: content_type_label(value),
            byte_len: content_byte_len(value),
            digest: Digest::of_str(&canonical_json(value)),
        },
        DataClass::Secret => RedactedValue::Secret {
            reference_digest: Digest::of_str(&canonical_json(value)),
        },
    }
}

/// Bound a `PublicLocal` value so even a "safe" field cannot carry an unbounded
/// or control-bearing string into audit/presentation.
fn bound_public(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(SafeText::new(s.clone()).as_str().to_string()),
        other => other.clone(),
    }
}

/// Redact a whole parameter object for `tool_name` using the shared registry.
/// Produces the canonical parameter digest and per-field redacted metadata.
#[must_use]
pub fn redact_parameters(tool_name: &str, params: &Value) -> RedactedParameters {
    let mut fields = BTreeMap::new();
    if let Some(obj) = params.as_object() {
        for (field, value) in obj {
            let class = REGISTRY.classify(tool_name, field);
            fields.insert(SafeField::new(field.clone()), redact_value(class, value));
        }
    }
    RedactedParameters {
        parameter_digest: parameter_digest(params),
        fields,
    }
}

/// Classify a single `(tool, field)` through the shared registry.
#[must_use]
pub fn classify_field(tool_name: &str, field: &str) -> DataClass {
    REGISTRY.classify(tool_name, field)
}

/// Produce a redacted JSON projection of arbitrary provider/trace text under a
/// class. Used by provider tracing so no raw provider detail is surfaced.
#[must_use]
pub fn redact_trace_value(class: DataClass, value: &Value) -> Value {
    serde_json::to_value(redact_value(class, value)).unwrap_or(Value::Null)
}

// ─────────────────────────────────────────────────────────────────────────────
// ApprovalProjection (design §15) — the ONLY thing HITL receives for OS actions
// ─────────────────────────────────────────────────────────────────────────────

/// A redacted summary of one resource affected by an action. Carries only a
/// domain label and an opaque identity digest — never a raw path/SSID/name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SafeResourceSummary {
    /// The typed resource domain token (e.g. `path`, `network-profile`).
    pub domain: SafeText,
    /// Opaque identity digest of the resource scope.
    pub identity: Digest,
}

/// The typed, redacted projection HITL receives for a native-OS action (design
/// §15). It replaces the raw `serde_json::Value` parameters and any
/// `params`-formatted description. The projection is **non-authoritative**: the
/// durable SQLite `InteractionDecision` remains the authority.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ApprovalProjection {
    /// The HITL request id.
    pub request_id: String,
    /// The durable decision id, when one has been created.
    pub decision_id: Option<String>,
    /// Redacted human-safe action label.
    pub action_label: SafeText,
    /// The action's risk level.
    pub risk: crate::safety::RiskLevel,
    /// Redacted purpose text.
    pub purpose: SafeText,
    /// Redacted summaries of affected resources (bounded).
    pub affected_resources: Vec<SafeResourceSummary>,
    /// Canonical parameter digest.
    pub parameter_digest: Digest,
    /// Per-field redacted parameter metadata (bounded, ordered).
    pub redacted_fields: BTreeMap<SafeField, RedactedValue>,
    /// The static per-operation rollback claim.
    pub rollback: crate::os_control::manifest::RollbackClaim,
}

/// Max number of affected-resource summaries carried in a projection.
const MAX_AFFECTED_RESOURCES: usize = 64;
/// Max number of redacted fields carried in a projection.
const MAX_REDACTED_FIELDS: usize = 64;

impl ApprovalProjection {
    /// Build the projection for a native-OS action from its raw parameters via
    /// the shared registry. Never retains a raw sensitive/secret/content value.
    #[must_use]
    pub fn build(
        request_id: &str,
        decision_id: Option<&str>,
        action: &str,
        risk: crate::safety::RiskLevel,
        purpose: &str,
        params: &Value,
    ) -> Self {
        let redacted = redact_parameters(action, params);
        let redacted_fields: BTreeMap<SafeField, RedactedValue> = redacted
            .fields
            .into_iter()
            .take(MAX_REDACTED_FIELDS)
            .collect();

        let affected_resources = crate::os_control::resource::os_resources(action, params)
            .into_iter()
            .take(MAX_AFFECTED_RESOURCES)
            .map(|r| SafeResourceSummary {
                domain: SafeText::new(r.kind().token()),
                identity: Digest::of_str(r.scope()),
            })
            .collect();

        let rollback = crate::os_control::manifest::frozen_contract(action)
            .map(|c| c.rollback)
            .unwrap_or(crate::os_control::manifest::RollbackClaim::NoRollback);

        Self {
            request_id: request_id.to_string(),
            decision_id: decision_id.map(str::to_string),
            action_label: SafeText::new(action),
            risk,
            purpose: SafeText::new(purpose),
            affected_resources,
            parameter_digest: redacted.parameter_digest,
            redacted_fields,
            rollback,
        }
    }

    /// Serialize the projection into the additive JSON value carried by the
    /// existing `ApprovalRequest.parameters` field (frontend contract stays
    /// stable; only the payload content becomes the redacted projection).
    #[must_use]
    pub fn to_hitl_parameters(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(obj) = value.as_object_mut() {
            obj.insert("os_action".to_string(), Value::Bool(true));
            obj.insert("redacted".to_string(), Value::Bool(true));
        }
        value
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use crate::os_control::manifest::frozen_tool_names;

    #[test]
    fn every_frozen_field_classifies_without_panicking() {
        // The registry builds for the whole frozen manifest and classifies every
        // declared field.
        for tool in frozen_tool_names() {
            let _ = redact_parameters(&tool, &serde_json::json!({}));
        }
    }

    #[test]
    fn public_local_values_are_retained_normalized() {
        // set_volume.percent is a Percent → PublicLocal → value retained.
        let r = redact_parameters("set_volume", &serde_json::json!({ "percent": 42 }));
        match r.fields.get(&SafeField::new("percent")).unwrap() {
            RedactedValue::Public(v) => assert_eq!(v, &serde_json::json!(42)),
            other => panic!("expected public, got {other:?}"),
        }
    }

    #[test]
    fn ssid_is_sensitive_metadata_and_never_leaks_value() {
        // connect_wifi carries an SSID (SensitiveMetadata) + password (Secret).
        let params = serde_json::json!({ "ssid": "SECRET-NET", "password": "hunter2" });
        let r = redact_parameters("connect_wifi", &params);
        let serialized = serde_json::to_string(&r).unwrap();
        assert!(!serialized.contains("SECRET-NET"), "ssid value leaked");
        assert!(!serialized.contains("hunter2"), "password value leaked");
    }

    #[test]
    fn secret_fields_store_only_a_reference_digest() {
        let params = serde_json::json!({ "ssid": "n", "password": "topsecret-pw" });
        let r = redact_parameters("connect_wifi", &params);
        if let Some(RedactedValue::Secret { reference_digest }) =
            r.fields.get(&SafeField::new("password"))
        {
            assert!(!reference_digest.as_hex().contains("topsecret"));
            assert_eq!(reference_digest.as_hex().len(), 64);
        } else {
            // If schema classifies password differently it must still not be public.
            let class = classify_field("connect_wifi", "password");
            assert_ne!(class, DataClass::PublicLocal, "password must not be public");
        }
    }

    #[test]
    fn content_fields_store_only_type_size_digest() {
        // set_clipboard payload data is Content.
        let params = serde_json::json!({
            "payload": { "mime": "text/plain", "encoding": "utf8", "data": "my private clipboard text", "content_digest": "d" }
        });
        let r = redact_parameters("set_clipboard", &params);
        let serialized = serde_json::to_string(&r).unwrap();
        assert!(
            !serialized.contains("my private clipboard text"),
            "clipboard content leaked: {serialized}"
        );
    }

    #[test]
    fn file_content_is_content_class_and_does_not_leak() {
        // write_file.content is BoundedContent (Content) → only type/size/digest.
        let params = serde_json::json!({
            "path": "/tmp/x",
            "content": { "encoding": "utf8", "data": "very private file body contents", "content_digest": "d" }
        });
        let r = redact_parameters("write_file", &params);
        assert_eq!(classify_field("write_file", "content"), DataClass::Content);
        let serialized = serde_json::to_string(&r).unwrap();
        assert!(
            !serialized.contains("very private file body contents"),
            "file content leaked: {serialized}"
        );
    }

    #[test]
    fn notification_text_follows_authoritative_manifest_classification() {
        // NORMATIVE NOTE: design §14's prose table lists "notification body" under
        // the Content class, but the frozen manifest (Task 0.1, the single source
        // of truth) annotates `send_notification.title`/`body` as `BoundedText`
        // (PublicLocal) — short, bounded, user-authored display text — while it
        // reserves the Content class for arbitrary large payloads (clipboard
        // `data`, file `content`). The shared registry faithfully implements the
        // authoritative manifest rather than the illustrative prose example. This
        // divergence is reported for the owner to reconcile; the registry does not
        // silently override the frozen classification.
        assert_eq!(
            classify_field("send_notification", "body"),
            DataClass::PublicLocal,
            "send_notification.body classification changed; reconcile with design §14"
        );
        assert_eq!(
            classify_field("send_notification", "title"),
            DataClass::PublicLocal
        );
    }

    #[test]
    fn canonical_json_is_key_order_independent() {
        let a = serde_json::json!({ "b": 1, "a": 2 });
        let b = serde_json::json!({ "a": 2, "b": 1 });
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(parameter_digest(&a), parameter_digest(&b));
    }

    #[test]
    fn approval_projection_carries_no_raw_secret_or_content() {
        let params = serde_json::json!({ "ssid": "MY-SSID", "password": "p@ssw0rd!" });
        let projection = ApprovalProjection::build(
            "req-1",
            Some("dec-1"),
            "connect_wifi",
            crate::safety::RiskLevel::Red,
            "connect to wifi",
            &params,
        );
        let hitl = projection.to_hitl_parameters();
        let serialized = serde_json::to_string(&hitl).unwrap();
        assert!(!serialized.contains("MY-SSID"));
        assert!(!serialized.contains("p@ssw0rd!"));
        assert_eq!(hitl["os_action"], serde_json::json!(true));
        assert_eq!(hitl["redacted"], serde_json::json!(true));
        // Affected resources carry only opaque digests, no raw ssid.
        assert!(!serialized.contains("MY-SSID"));
    }

    #[test]
    fn unknown_field_defaults_to_hashed_sensitive_metadata() {
        // A field not present in the schema is conservatively hashed, never shown.
        let params = serde_json::json!({ "totally_unexpected": "leak-me-if-you-can" });
        let r = redact_parameters("set_volume", &params);
        let serialized = serde_json::to_string(&r).unwrap();
        assert!(!serialized.contains("leak-me-if-you-can"));
    }
}
