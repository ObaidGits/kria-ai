//! Neutral, safe capability **primitive** vocabulary (Wave 9 foundation).
//!
//! A small, whitelisted set of pure, deterministic text transforms that both the
//! filesystem provider and the synthesis provider execute. Being *pure*
//! (no I/O, no host code, no network), a capability built from a primitive is
//! safe to run in-process AND trivially satisfies the "synthesized code never
//! runs unsandboxed on the host" invariant (spec R11.4) — there is no generated
//! host code, only a declared composition of these audited primitives.
//!
//! This is the anti-fake boundary for synthesis: KRIA can only synthesize
//! capabilities expressible from this audited set, and **honestly declines**
//! otherwise (spec R7.4) rather than emitting unverified arbitrary code.

use base64::Engine;

/// The audited primitive operations. Open at the edges via [`apply_primitive`]
/// returning `None` for unknown ops (honest-decline, never a fabricated result).
pub const KNOWN_PRIMITIVES: &[&str] = &[
    "reverse",
    "upper",
    "lower",
    "trim",
    "length",
    "word_count",
    "base64_encode",
    "base64_decode",
    "hex_encode",
    "json_pretty",
    "json_minify",
];

/// Apply a primitive transform to `text`. Returns `Ok(None)` for an unknown
/// operation (caller declines honestly); `Err` for a real execution error
/// (e.g. malformed input for a decoder).
pub fn apply_primitive(op: &str, text: &str) -> Result<Option<String>, String> {
    let out = match op {
        "reverse" => text.chars().rev().collect::<String>(),
        "upper" => text.to_uppercase(),
        "lower" => text.to_lowercase(),
        "trim" => text.trim().to_string(),
        "length" => text.chars().count().to_string(),
        "word_count" => text.split_whitespace().count().to_string(),
        "base64_encode" => base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
        "base64_decode" => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(text.as_bytes())
                .map_err(|e| format!("base64 decode failed: {e}"))?;
            String::from_utf8(bytes).map_err(|e| format!("base64 utf8: {e}"))?
        }
        "hex_encode" => text.as_bytes().iter().map(|b| format!("{b:02x}")).collect(),
        "json_pretty" => {
            let v: serde_json::Value =
                serde_json::from_str(text).map_err(|e| format!("invalid json: {e}"))?;
            serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
        }
        "json_minify" => {
            let v: serde_json::Value =
                serde_json::from_str(text).map_err(|e| format!("invalid json: {e}"))?;
            serde_json::to_string(&v).map_err(|e| e.to_string())?
        }
        _ => return Ok(None),
    };
    Ok(Some(out))
}

/// Audited **multi-input reducer** operations (Wave 9, W9-R9 / BLOCKER 4): pure,
/// deterministic functions of SEVERAL named text inputs → one text output. They
/// are the typed-multi-input boundary node of a synthesized capability (e.g.
/// "concatenate two strings", "merge two JSON objects"). Like primitives they are
/// pure (no I/O), so a multi-input capability needs no host code / sandbox.
pub const KNOWN_REDUCERS: &[&str] = &["concat", "json_merge", "join_lines"];

/// The declared named input keys of a reducer (typed multi-input schema). Order
/// is the argument order for goldens/UX.
pub fn reducer_inputs(op: &str) -> Option<&'static [&'static str]> {
    match op {
        "concat" => Some(&["a", "b"]),
        "json_merge" => Some(&["a", "b"]),
        "join_lines" => Some(&["items", "separator"]),
        _ => None,
    }
}

/// Apply a multi-input reducer to a map of named string arguments. Returns
/// `Ok(None)` for an unknown reducer (honest-decline); `Err` for a real input
/// error (e.g. malformed JSON for `json_merge`). Never fabricates a result.
pub fn apply_reducer(
    op: &str,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<String>, String> {
    let get = |k: &str| -> Result<String, String> {
        args.get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| format!("reducer '{op}' missing required string argument '{k}'"))
    };
    let out = match op {
        "concat" => format!("{}{}", get("a")?, get("b")?),
        "join_lines" => {
            // `items` may be a JSON array of strings OR a newline-delimited string.
            let sep = args
                .get("separator")
                .and_then(|v| v.as_str())
                .unwrap_or("\n")
                .to_string();
            let items = args
                .get("items")
                .ok_or_else(|| "reducer 'join_lines' missing 'items'".to_string())?;
            let parts: Vec<String> = if let Some(arr) = items.as_array() {
                arr.iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            } else if let Some(s) = items.as_str() {
                s.lines().map(|l| l.to_string()).collect()
            } else {
                return Err("reducer 'join_lines' 'items' must be array or string".into());
            };
            parts.join(&sep)
        }
        "json_merge" => {
            let a: serde_json::Value =
                serde_json::from_str(&get("a")?).map_err(|e| format!("json_merge 'a': {e}"))?;
            let b: serde_json::Value =
                serde_json::from_str(&get("b")?).map_err(|e| format!("json_merge 'b': {e}"))?;
            let (mut ao, bo) = match (a, b) {
                (serde_json::Value::Object(ao), serde_json::Value::Object(bo)) => (ao, bo),
                _ => return Err("json_merge requires two JSON objects".into()),
            };
            for (k, v) in bo {
                ao.insert(k, v); // b overrides a on key collisions
            }
            serde_json::to_string(&serde_json::Value::Object(ao)).map_err(|e| e.to_string())?
        }
        _ => return Ok(None),
    };
    Ok(Some(out))
}

/// Infer a multi-input reducer from a goal, or `None`. Specific → general.
pub fn infer_reducer_from_goal(goal: &str) -> Option<&'static str> {
    let g = goal.to_lowercase();
    let table: &[(&[&str], &str)] = &[
        (&["merge", "json"], "json_merge"),
        (&["combine", "json"], "json_merge"),
        (&["concatenate"], "concat"),
        (&["concat"], "concat"),
        (&["join", "lines"], "join_lines"),
        (&["join", "with"], "join_lines"),
    ];
    for (needles, op) in table {
        if needles.iter().all(|n| g.contains(n)) {
            return Some(op);
        }
    }
    None
}

/// Apply an ordered **pipeline** of primitives (capability composition): the
/// output of each stage feeds the next. Empty pipeline is an error; an unknown
/// op in the pipeline is an honest error (never a fabricated result).
pub fn apply_pipeline(ops: &[String], text: &str) -> Result<String, String> {
    if ops.is_empty() {
        return Err("empty capability pipeline".into());
    }
    let mut cur = text.to_string();
    for op in ops {
        match apply_primitive(op, &cur)? {
            Some(next) => cur = next,
            None => return Err(format!("pipeline stage '{op}' is not a known primitive")),
        }
    }
    Ok(cur)
}

/// Infer an ordered pipeline of primitives from a composed goal (e.g.
/// "trim then uppercase then reverse"). Splits on sequence connectives and infers
/// each segment; returns `None` if ANY segment is not expressible (honest-decline
/// — a partially-synthesizable composition is still a decline, never fabricated).
pub fn infer_pipeline_from_goal(goal: &str) -> Option<Vec<&'static str>> {
    let g = goal.to_lowercase();
    // Sequence connectives, longest first so "and then" wins over "and"/"then".
    let has_seq = [
        " then ",
        " and then ",
        ", then ",
        " followed by ",
        " -> ",
        " => ",
    ]
    .iter()
    .any(|s| g.contains(s));
    if !has_seq {
        return infer_primitive_from_goal(goal).map(|p| vec![p]);
    }
    // Normalize all connectives to a single delimiter, then split.
    let mut normalized = g.clone();
    for sep in [
        " and then ",
        ", then ",
        " followed by ",
        " -> ",
        " => ",
        " then ",
    ] {
        normalized = normalized.replace(sep, "\u{1}");
    }
    let mut ops = Vec::new();
    for segment in normalized.split('\u{1}') {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        // Each segment must map to a primitive, else the whole composition declines.
        let op = infer_primitive_from_goal(seg)?;
        ops.push(op);
    }
    if ops.is_empty() {
        None
    } else {
        Some(ops)
    }
}

/// Infer the best-matching primitive for a natural-language goal, or `None` when
/// the goal is not expressible from the audited set (honest-decline, R7.4).
/// Keyword hints map to a primitive; this is *synthesis input inference*, not
/// provider routing (it lives in the neutral Brain layer).
pub fn infer_primitive_from_goal(goal: &str) -> Option<&'static str> {
    let g = goal.to_lowercase();
    // Ordered specific→general; "decode"/"encode" before generic base64.
    // Each entry is an AND-group of needles (all must be present). Synonyms are
    // separate entries (OR by iteration order). Specific → general.
    let table: &[(&[&str], &str)] = &[
        (&["base64", "decode"], "base64_decode"),
        (&["base64", "encode"], "base64_encode"),
        (&["base64"], "base64_encode"),
        (&["json", "minify"], "json_minify"),
        (&["json", "compact"], "json_minify"),
        (&["json", "pretty"], "json_pretty"),
        (&["json", "prettify"], "json_pretty"),
        (&["json", "beautify"], "json_pretty"),
        (&["json", "format"], "json_pretty"),
        (&["reverse"], "reverse"),
        (&["uppercase"], "upper"),
        (&["upper case"], "upper"),
        (&["lowercase"], "lower"),
        (&["lower case"], "lower"),
        (&["trim"], "trim"),
        (&["whitespace"], "trim"),
        (&["word count"], "word_count"),
        (&["count words"], "word_count"),
        (&["char count"], "length"),
        (&["count characters"], "length"),
        (&["length"], "length"),
        (&["hex"], "hex_encode"),
    ];
    for (needles, op) in table {
        if needles.iter().all(|n| g.contains(n)) {
            return Some(op);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_apply_correctly() {
        assert_eq!(
            apply_primitive("reverse", "abc").unwrap(),
            Some("cba".into())
        );
        assert_eq!(apply_primitive("upper", "ab").unwrap(), Some("AB".into()));
        assert_eq!(
            apply_primitive("base64_encode", "hi").unwrap(),
            Some("aGk=".into())
        );
        assert_eq!(
            apply_primitive("base64_decode", "aGk=").unwrap(),
            Some("hi".into())
        );
        assert_eq!(apply_primitive("length", "abc").unwrap(), Some("3".into()));
    }

    #[test]
    fn unknown_primitive_declines_not_fabricates() {
        assert_eq!(apply_primitive("mine_bitcoin", "x").unwrap(), None);
    }

    #[test]
    fn bad_input_is_honest_error() {
        assert!(apply_primitive("base64_decode", "!!!not-base64!!!").is_err());
        assert!(apply_primitive("json_pretty", "not json").is_err());
    }

    #[test]
    fn pipeline_applies_in_sequence() {
        // trim → upper → reverse
        let ops = vec![
            "trim".to_string(),
            "upper".to_string(),
            "reverse".to_string(),
        ];
        assert_eq!(apply_pipeline(&ops, "  hi  ").unwrap(), "IH");
        assert!(apply_pipeline(&[], "x").is_err());
        assert!(apply_pipeline(&["bogus".to_string()], "x").is_err());
    }

    #[test]
    fn pipeline_inference_composes_or_declines() {
        assert_eq!(
            infer_pipeline_from_goal("trim then uppercase then reverse"),
            Some(vec!["trim", "upper", "reverse"])
        );
        assert_eq!(
            infer_pipeline_from_goal("base64 encode then reverse"),
            Some(vec!["base64_encode", "reverse"])
        );
        // Single op (no connective) → pipeline of one.
        assert_eq!(
            infer_pipeline_from_goal("reverse text"),
            Some(vec!["reverse"])
        );
        // A composition where one stage is unsynthesizable → whole thing declines.
        assert_eq!(
            infer_pipeline_from_goal("uppercase then deploy to kubernetes"),
            None
        );
    }

    #[test]
    fn reducers_apply_and_infer_or_decline() {
        let mut m = serde_json::Map::new();
        m.insert("a".into(), serde_json::json!("foo"));
        m.insert("b".into(), serde_json::json!("bar"));
        assert_eq!(apply_reducer("concat", &m).unwrap(), Some("foobar".into()));

        let mut jm = serde_json::Map::new();
        jm.insert("a".into(), serde_json::json!("{\"x\":1}"));
        jm.insert("b".into(), serde_json::json!("{\"y\":2}"));
        let merged = apply_reducer("json_merge", &jm).unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v.get("x").and_then(|x| x.as_i64()), Some(1));
        assert_eq!(v.get("y").and_then(|x| x.as_i64()), Some(2));

        // Unknown reducer → honest None (never fabricated).
        assert_eq!(apply_reducer("mine", &m).unwrap(), None);
        // Missing arg → honest error.
        let empty = serde_json::Map::new();
        assert!(apply_reducer("concat", &empty).is_err());

        assert_eq!(
            infer_reducer_from_goal("concatenate two strings"),
            Some("concat")
        );
        assert_eq!(
            infer_reducer_from_goal("merge two json objects"),
            Some("json_merge")
        );
        assert_eq!(infer_reducer_from_goal("reverse a string"), None);
    }

    #[test]
    fn goal_inference_maps_or_declines() {
        assert_eq!(
            infer_primitive_from_goal("reverse a string"),
            Some("reverse")
        );
        assert_eq!(
            infer_primitive_from_goal("base64 encode this text"),
            Some("base64_encode")
        );
        assert_eq!(
            infer_primitive_from_goal("prettify this json output"),
            Some("json_pretty")
        );
        // Not expressible from the audited set → honest decline.
        assert_eq!(infer_primitive_from_goal("train a neural network"), None);
    }
}
