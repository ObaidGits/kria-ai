//! `GoalIntent` — the parsed, embedded representation of a user's goal
//! (design §7.2).
//!
//! # Ownership / coordination seam (tasks 5.1 ↔ 5.2)
//!
//! This module defines the data model [`GoalIntent`] exactly as design §7.2
//! specifies, plus **generic** helpers. The *real* goal derivation
//! ([`derive_goal_intent_llm`]) — `Embedder::embed` + **one** structured LLM
//! call producing `required`/`composite`/`max_risk` with NO keyword tables and
//! NO per-category rules (design §7.1 anti-hardcoding, §Runtime step 1) — is
//! **task 5.1**, implemented here. The deterministic JSON→capabilities mapping
//! stays factored into the pure [`parse_required`] so it is unit-testable
//! without a live LLM, and the simpler [`derive_goal_intent`] helper (embed +
//! caller-supplied `required`) is retained for tests/callers that construct a
//! `GoalIntent` directly. Task 5.2 (`CapabilityRanker`, [`super::rank`]) only
//! needs the struct *shape* + a way to construct a `GoalIntent`, so the
//! canonical single definition lives here (one definition, not a duplicate).
//!
//! # No-hardcoding
//!
//! `required` is a list of open-vocabulary [`CapabilityTag`]s with confidences —
//! never an enum of known capabilities. A never-before-seen capability id flows
//! through as just another string, so ranking and planning treat it identically
//! to any built-in (design anti-hardcoding proof, §7.1).

use super::embed::Embedder;
use super::profile::CapabilityTag;
use super::CilError;
use crate::llm::{ChatMessage, ModelRouter};
use crate::openclaw::arg_gen::extract_json_object;
use crate::safety::RiskLevel;

/// The parsed, embedded representation of a user's goal (design §7.2).
///
/// Produced generically via the configured LLM + embedder — NO keyword tables,
/// NO category rules. The full derivation is task 5.1; this is the shared data
/// model consumed by the discovery index (task 3.3), the ranker (task 5.2), and
/// the planner (later phases).
#[derive(Debug, Clone)]
pub struct GoalIntent {
    /// The raw goal text as the user expressed it.
    pub raw: String,
    /// Dense embedding of the goal (semantic query for discovery).
    pub goal_embedding: Vec<f32>,
    /// Capabilities the goal appears to require, each with a confidence in
    /// `0.0..=1.0`. Open vocabulary — new domains are new tag ids, zero code.
    pub required: Vec<(CapabilityTag, f32)>,
    /// Whether the goal likely needs composition (multiple capabilities).
    pub composite: bool,
    /// The maximum risk the goal is permitted to reach.
    pub max_risk: crate::safety::RiskLevel,
}

/// Parse a structured LLM output value into the open-vocabulary `required`
/// capability list — generic, no keyword tables, no per-category branch.
///
/// Accepts a JSON array of objects shaped `{"id": "<tag>", "confidence": <f32>}`
/// (also tolerates `"capability"`/`"tag"` as the id key and a bare string
/// element). Anything unparseable is skipped. Task 5.1 wires the structured LLM
/// call that produces this value; this helper is the deterministic, testable
/// parse step it reuses.
pub fn parse_required(value: &serde_json::Value) -> Vec<(CapabilityTag, f32)> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        // A bare string is a tag id with full confidence.
        if let Some(id) = item.as_str() {
            if !id.is_empty() {
                out.push((CapabilityTag::new(id), 1.0));
            }
            continue;
        }
        let Some(obj) = item.as_object() else {
            continue;
        };
        let id = obj
            .get("id")
            .or_else(|| obj.get("capability"))
            .or_else(|| obj.get("tag"))
            .and_then(|v| v.as_str());
        let Some(id) = id.filter(|s| !s.is_empty()) else {
            continue;
        };
        let confidence = obj
            .get("confidence")
            .or_else(|| obj.get("score"))
            .and_then(|v| v.as_f64())
            .map(|f| (f as f32).clamp(0.0, 1.0))
            .unwrap_or(1.0);
        out.push((CapabilityTag::new(id), confidence));
    }
    out
}

/// Derive a [`GoalIntent`] for `raw` by embedding it with the configured
/// [`Embedder`] and attaching the already-parsed `required` capabilities.
///
/// This is the direct-construction helper (embed + caller-supplied `required`).
/// The full generic derivation — embed + one structured LLM call — is
/// [`derive_goal_intent_llm`]. This helper stays for tests/callers that already
/// hold `required`, and is honest: an embedder failure surfaces as
/// [`CilError::Embed`], never a silent default.
pub async fn derive_goal_intent(
    raw: &str,
    embedder: &dyn Embedder,
    required: Vec<(CapabilityTag, f32)>,
    composite: bool,
    max_risk: RiskLevel,
) -> Result<GoalIntent, CilError> {
    let goal_embedding = embedder
        .embed(raw)
        .await
        .map_err(|e| CilError::Embed(format!("goal embed failed: {e}")))?;
    Ok(GoalIntent {
        raw: raw.to_string(),
        goal_embedding,
        required,
        composite,
        max_risk,
    })
}

/// The `schema_name` posted with the structured intent call (used by the
/// `json_schema` `response_format` envelope, mirroring `arg_gen`).
const INTENT_SCHEMA_NAME: &str = "openclaw_goal_intent";

/// The routing intent hint handed to [`ModelRouter::route`] for the intent call.
const INTENT_ROUTE_HINT: &str = "openclaw.cil.intent";

/// The JSON schema for the single structured intent call (design §Runtime
/// step 1). The model returns an object with a `capabilities` array; each item
/// is `{ "id": <string>, "confidence": <number> }`.
///
/// The tag `id` is an **open string** — deliberately NOT an `enum`. A
/// never-before-seen capability id flows straight through [`parse_required`] as
/// just another string, so no code, table, or schema change is needed to
/// support a new domain (design anti-hardcoding proof, §7.1).
pub fn goal_intent_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "capabilities": {
                "type": "array",
                "description": "Capabilities the goal requires, most-relevant first.",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Open-vocabulary reverse-DNS-style capability id, e.g. net.file.download"
                        },
                        "confidence": {
                            "type": "number",
                            "description": "Confidence in 0.0..=1.0 that the goal needs this capability"
                        }
                    },
                    "required": ["id"]
                }
            }
        },
        "required": ["capabilities"]
    })
}

/// Build the two-message prompt for the structured intent call, reusing the
/// frozen `arg_gen` discipline: a system message that forbids prose/markdown and
/// asks for ONLY a schema-conforming JSON object, plus the schema and the user
/// goal. Kept pure (no I/O) so prompt construction is unit-testable.
///
/// The instruction is deliberately generic: it asks the model to infer the
/// capabilities an arbitrary goal needs from an OPEN vocabulary. It contains no
/// keyword list, no category enumeration, and no per-domain hint — the required
/// tags come entirely from the model's open-vocabulary output.
pub fn build_intent_messages(query: &str) -> Vec<ChatMessage> {
    let schema_pretty =
        serde_json::to_string_pretty(&goal_intent_schema()).unwrap_or_else(|_| "{}".into());
    let system = "You analyze a user's goal and infer the abstract capabilities it requires. \
         Respond with ONLY a single JSON object that conforms to the provided JSON Schema — \
         no prose, no markdown, no code fences. Each capability `id` is an open-vocabulary, \
         reverse-DNS-style string you choose to best describe a needed capability (e.g. \
         net.file.download, data.csv.parse, viz.chart.render). Do NOT restrict yourself to a \
         fixed list; invent a precise id when needed. Set `confidence` in 0.0..=1.0. Return an \
         empty `capabilities` array if the goal needs no external capability."
        .to_string();
    let user = format!("JSON Schema for your answer:\n{schema_pretty}\n\nUser goal:\n{query}");
    vec![
        ChatMessage {
            role: "system".into(),
            content: system,
            name: None,
            images: None,
        },
        ChatMessage {
            role: "user".into(),
            content: user,
            name: None,
            images: None,
        },
    ]
}

/// Extract the open-vocabulary `required` capabilities from a parsed structured
/// intent object. Reuses the pure [`parse_required`] over the `capabilities`
/// array so the JSON→capabilities mapping stays testable without a live LLM.
///
/// Tolerant of the model omitting the wrapper: if `capabilities` is absent the
/// whole object/array is passed through (a model may answer with a bare array).
fn required_from_intent_object(obj: &serde_json::Value) -> Vec<(CapabilityTag, f32)> {
    match obj.get("capabilities") {
        Some(caps) => parse_required(caps),
        None => parse_required(obj),
    }
}

/// Derive a [`GoalIntent`] for `query` **generically** — the real task-5.1 path
/// (design §Runtime step 1). Performs exactly two backend interactions and no
/// more: one embedding and **one** structured LLM call (bounded cognition — no
/// keyword tables, no per-category rules, no recursive re-planning loop).
///
/// Steps:
/// 1. `goal_embedding = embedder.embed(query)` — a backend failure surfaces as
///    [`CilError::Embed`] (honest degraded, never a silent default).
/// 2. Route to a backend via the frozen [`ModelRouter`]; no backend →
///    [`CilError::Degraded`].
/// 3. ONE `chat_structured` call constrained by [`goal_intent_schema`] (reusing
///    the frozen structured-output discipline). A call error or a response that
///    is not extractable JSON → [`CilError::Degraded`] (we do NOT fabricate
///    capabilities).
/// 4. Map the `capabilities` array through the pure [`parse_required`];
///    `composite = required.len() > 1`; `max_risk` is the caller-provided ceiling.
///
/// `max_risk` is threaded in from the request policy rather than inferred here,
/// keeping risk authority with the frozen safety layer (subsystem boundary).
pub async fn derive_goal_intent_llm(
    query: &str,
    embedder: &dyn Embedder,
    llm: &ModelRouter,
    max_risk: RiskLevel,
) -> Result<GoalIntent, CilError> {
    // 1. Embed the goal (semantic query for discovery). Honest on failure.
    let goal_embedding = embedder
        .embed(query)
        .await
        .map_err(|e| CilError::Embed(format!("goal embed failed: {e}")))?;

    // 2. Select a backend. No backend available → honest degraded mode.
    let backend = llm.route(INTENT_ROUTE_HINT).await.ok_or_else(|| {
        CilError::Degraded("no LLM backend available for goal-intent derivation".to_string())
    })?;

    // 3. ONE structured call, schema-constrained (frozen arg_gen discipline).
    let messages = build_intent_messages(query);
    let resp = backend
        .chat_structured(
            &messages,
            goal_intent_schema(),
            INTENT_SCHEMA_NAME,
            0.0,
            512,
        )
        .await
        .map_err(|e| CilError::Degraded(format!("goal-intent structured call failed: {e}")))?;

    // 4. Parse the structured object. Unparseable → degraded, never fabricated.
    let obj = extract_json_object(&resp.content).ok_or_else(|| {
        CilError::Degraded("goal-intent response was not a JSON object".to_string())
    })?;
    let required = required_from_intent_object(&obj);
    let composite = required.len() > 1;

    Ok(GoalIntent {
        raw: query.to_string(),
        goal_embedding,
        required,
        composite,
        max_risk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_required_reads_id_and_confidence() {
        let v = json!([{ "id": "net.file.download", "confidence": 0.8 }]);
        let out = parse_required(&v);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.id, "net.file.download");
        assert!((out[0].1 - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_required_accepts_bare_string_tag() {
        let v = json!(["data.csv.parse"]);
        let out = parse_required(&v);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.id, "data.csv.parse");
        assert!(
            (out[0].1 - 1.0).abs() < f32::EPSILON,
            "bare string → full confidence"
        );
    }

    #[test]
    fn parse_required_defaults_missing_confidence_to_full() {
        let v = json!([{ "id": "viz.chart.render" }]);
        let out = parse_required(&v);
        assert_eq!(out.len(), 1);
        assert!((out[0].1 - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_required_accepts_alias_keys_and_score() {
        // `capability`/`tag` id aliases and `score` confidence alias.
        let v = json!([
            { "capability": "a.b", "score": 0.5 },
            { "tag": "c.d" },
        ]);
        let out = parse_required(&v);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0.id, "a.b");
        assert!((out[0].1 - 0.5).abs() < f32::EPSILON);
        assert_eq!(out[1].0.id, "c.d");
    }

    #[test]
    fn parse_required_skips_empty_and_invalid_items() {
        let v = json!([
            { "id": "" },          // empty id → skipped
            "",                     // empty bare string → skipped
            42,                     // not a string/object → skipped
            { "confidence": 0.9 }, // missing id → skipped
            { "id": "keep.this" },
        ]);
        let out = parse_required(&v);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.id, "keep.this");
    }

    #[test]
    fn parse_required_clamps_confidence_to_unit_range() {
        let v = json!([
            { "id": "hi", "confidence": 5.0 },
            { "id": "lo", "confidence": -3.0 },
        ]);
        let out = parse_required(&v);
        assert!((out[0].1 - 1.0).abs() < f32::EPSILON);
        assert!((out[1].1 - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_required_non_array_yields_empty() {
        assert!(parse_required(&json!({ "id": "x" })).is_empty());
        assert!(parse_required(&json!("x")).is_empty());
        assert!(parse_required(&json!(null)).is_empty());
    }

    #[test]
    fn open_vocabulary_novel_tag_is_not_constrained() {
        // A tag id never seen before flows through unchanged — proof the schema
        // is open (no enum). The schema's `id` property must be a plain string.
        let novel = "com.acme.quantum.teleport.v7";
        let out = parse_required(&json!([{ "id": novel, "confidence": 0.42 }]));
        assert_eq!(out[0].0.id, novel);

        let schema = goal_intent_schema();
        let id_schema = &schema["properties"]["capabilities"]["items"]["properties"]["id"];
        assert_eq!(id_schema["type"], "string");
        assert!(
            id_schema.get("enum").is_none(),
            "tag id must be open, not an enum"
        );
    }

    #[test]
    fn composite_derivation_from_required_count() {
        // composite is derived purely from the count of required capabilities.
        let single = required_from_intent_object(&json!({ "capabilities": [{ "id": "one" }] }));
        assert!(!(single.len() > 1));

        let multi = required_from_intent_object(&json!({
            "capabilities": [{ "id": "one" }, { "id": "two" }]
        }));
        assert!(multi.len() > 1);
    }

    #[test]
    fn required_from_intent_object_reads_capabilities_wrapper() {
        let obj = json!({ "capabilities": [{ "id": "x.y", "confidence": 0.3 }] });
        let out = required_from_intent_object(&obj);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.id, "x.y");
    }

    #[test]
    fn required_from_intent_object_tolerates_bare_array() {
        // Model omitted the wrapper and answered with a bare array element list.
        let obj = json!([{ "id": "bare.array" }]);
        let out = required_from_intent_object(&obj);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.id, "bare.array");
    }

    #[test]
    fn goal_intent_schema_is_object_with_required_capabilities() {
        let schema = goal_intent_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "capabilities");
        assert_eq!(schema["properties"]["capabilities"]["type"], "array");
    }

    #[test]
    fn build_intent_messages_has_system_and_embeds_query() {
        let msgs = build_intent_messages("download a file and chart it");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].role, "user");
        // User message carries the goal + the schema, no keyword tables.
        assert!(msgs[1].content.contains("download a file and chart it"));
        assert!(msgs[1].content.contains("capabilities"));
    }

    /// A realistic structured LLM output — the exact shape `goal_intent_schema`
    /// describes, `{"capabilities":[{"id":..,"confidence":..}, ...]}` — parses
    /// into the open-vocabulary `Vec<(CapabilityTag, f32)>` with NO keyword
    /// table and NO per-category mapping. (R1.3, task 5.5)
    #[test]
    fn structured_llm_output_parses_to_open_vocabulary_required() {
        // Shaped exactly as the schema instructs the model to answer.
        let llm_output = json!({
            "capabilities": [
                { "id": "net.file.download", "confidence": 0.95 },
                { "id": "data.csv.parse",    "confidence": 0.80 },
                { "id": "viz.chart.render",  "confidence": 0.60 }
            ]
        });
        let out = required_from_intent_object(&llm_output);
        // Every capability flows through as an open string tag with its
        // confidence — order preserved, nothing dropped, nothing remapped.
        let ids: Vec<&str> = out.iter().map(|(t, _)| t.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["net.file.download", "data.csv.parse", "viz.chart.render"]
        );
        assert!((out[0].1 - 0.95).abs() < f32::EPSILON);
        assert!((out[1].1 - 0.80).abs() < f32::EPSILON);
        assert!((out[2].1 - 0.60).abs() < f32::EPSILON);
    }

    /// A novel/unseen capability id in the structured output flows through
    /// verbatim — open vocabulary, no enum, no per-category branch. (R1.3)
    #[test]
    fn structured_output_passes_novel_tag_through_unchanged() {
        let novel = "io.holographic.render.v11";
        let llm_output = json!({ "capabilities": [{ "id": novel, "confidence": 0.5 }] });
        let out = required_from_intent_object(&llm_output);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.id, novel, "unseen tag must pass through as-is");
        assert!((out[0].1 - 0.5).abs() < f32::EPSILON);
    }

    /// `composite` (derived as `required.len() > 1`) is false for a single
    /// capability output and true for a multi-capability output — computed from
    /// the structured object alone, no keyword logic. (R1.3)
    #[test]
    fn structured_output_drives_composite_from_capability_count() {
        let single = required_from_intent_object(&json!({
            "capabilities": [{ "id": "single.cap", "confidence": 1.0 }]
        }));
        assert_eq!(single.len(), 1);
        assert!(!(single.len() > 1), "single capability → not composite");

        let multi = required_from_intent_object(&json!({
            "capabilities": [
                { "id": "first.cap",  "confidence": 0.9 },
                { "id": "second.cap", "confidence": 0.7 }
            ]
        }));
        assert_eq!(multi.len(), 2);
        assert!(multi.len() > 1, "multiple capabilities → composite");
    }

    /// The schema's capability `id` field is an OPEN string with no `enum`
    /// constraint, proving there is no hardcoded/closed capability set. (R1.3)
    #[test]
    fn schema_capability_id_is_open_string_no_enum() {
        let schema = goal_intent_schema();
        let items = &schema["properties"]["capabilities"]["items"];
        // items describe an object whose `id` property is a bare string.
        assert_eq!(items["type"], "object");
        let id_schema = &items["properties"]["id"];
        assert_eq!(id_schema["type"], "string");
        assert!(
            id_schema.get("enum").is_none(),
            "capability id must not be a closed enum"
        );
        // No sibling closed enumeration of categories anywhere on the item.
        assert!(
            items["properties"].get("category").is_none(),
            "no per-category field allowed"
        );
    }
}
