//! Capability profile data models (design §7.1) and column (de)serialization
//! for the `capability_profiles` derived-view table (design §7.4).
//!
//! [`CapabilityProfile`] is a **derived VIEW over `SkillMetadata`** — never an
//! authoritative store. The authoritative source of truth remains the
//! `ProductionSkillRegistry`; `capability_profiles` is rebuildable from it and
//! keyed by `skill_id`. The extractor that derives a profile from
//! `SkillMetadata` is task 2.3; this module defines only the models plus the
//! helpers that (de)serialize a profile to/from the table columns.
//!
//! # No-hardcoding primitive
//!
//! [`CapabilityTag::id`] is an **open-vocabulary** reverse-DNS-style string
//! (e.g. `"media.image.ocr"`, `"io.file.read"`). It is deliberately NOT an enum:
//! new capability domains are new strings supplied by skill metadata, requiring
//! zero code changes and zero per-category branches anywhere.
//!
//! # `Eq` deviation from design §7.1
//!
//! Design §7.1 lists `#[derive(... PartialEq, Eq ...)]` for [`CapabilityTag`],
//! but the struct contains `Option<Vec<f32>>` (`f32` is not `Eq`) and
//! `serde_json::Map<String, serde_json::Value>` (`serde_json::Value` is not `Eq`
//! because it can hold an `f64`). Deriving `Eq` therefore does not compile.
//! We keep `PartialEq` (equality by tag identity + qualifiers + embedding) and
//! drop `Eq`. This preserves the design intent — tags remain comparable — while
//! satisfying the type checker.

use serde::{Deserialize, Serialize};

use super::CilError;

/// A semantic capability a skill PROVIDES or a goal REQUIRES (design §7.1).
///
/// `id` is a namespaced, open string (e.g. `"io.file.read"`, `"media.image.ocr"`,
/// `"doc.pdf.render"`, `"net.email.send"`). New domains = new strings = zero code.
///
/// See the module docs for why `Eq` is not derived (design §7.1 lists it, but
/// the `f32` embedding and `serde_json::Value` qualifiers are not `Eq`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityTag {
    /// Reverse-DNS-style capability id. Open vocabulary; NOT an enum.
    pub id: String,
    /// Optional structured qualifiers (e.g. `{"format":"pdf"}`), matched structurally.
    #[serde(default)]
    pub qualifiers: serde_json::Map<String, serde_json::Value>,
    /// Optional dense embedding of the tag (lazily computed, cached).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

impl CapabilityTag {
    /// Construct a bare tag from its open-vocabulary id (no qualifiers, no embedding).
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            qualifiers: serde_json::Map::new(),
            embedding: None,
        }
    }
}

/// A skill's advertised capability profile — derived from its manifest/metadata
/// (design §7.1). This is a VIEW over `SkillMetadata`; it is never the
/// authoritative store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityProfile {
    pub skill_id: String,
    /// What this skill provides (semantic tags).
    pub provides: Vec<CapabilityTag>,
    /// What this skill needs from other skills to be useful (composition edges).
    pub consumes: Vec<CapabilityTag>,
    /// Permission capabilities it will request at runtime (frozen `capability::Capability`).
    pub permissions: Vec<crate::openclaw::capability::Capability>,
    /// I/O contract for composition: MIME/type tags in and out (open strings).
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

/// The raw column values of one `capability_profiles` row (design §7.4).
///
/// The JSON columns hold serialized `Vec<CapabilityTag>` / `Vec<String>`; the
/// `embedding` BLOB holds the profile embedding as little-endian `f32` bytes (see
/// [`encode_embedding`]); `profile_epoch` is the model/version epoch. `permissions`
/// are NOT a column — they are derived at extraction time (task 2.3), so they
/// round-trip only through [`CapabilityProfile`] itself, not this row.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileColumns {
    pub skill_id: String,
    pub provides_json: String,
    pub consumes_json: String,
    pub inputs_json: String,
    pub outputs_json: String,
    pub embedding: Option<Vec<u8>>,
    pub profile_epoch: i64,
}

/// A fully materialized `capability_profiles` row: the derived [`CapabilityProfile`]
/// plus the row-level `embedding` and `profile_epoch` columns (design §7.4).
///
/// `embedding` is the profile-level dense vector (typically the embedding of the
/// `provides` tags); `profile_epoch` versions the derived view so a model change
/// can trigger a background reindex. Both are row columns rather than fields of
/// [`CapabilityProfile`], which is why they live here.
#[derive(Debug, Clone)]
pub struct CapabilityProfileRow {
    pub profile: CapabilityProfile,
    pub embedding: Option<Vec<f32>>,
    pub profile_epoch: i64,
}

impl CapabilityProfileRow {
    /// Serialize this row into the `capability_profiles` column values.
    ///
    /// `provides`/`consumes`/`inputs`/`outputs` become their `_json` TEXT columns;
    /// `embedding` becomes the little-endian `f32` BLOB (NULL when absent).
    pub fn to_columns(&self) -> Result<ProfileColumns, CilError> {
        let provides_json = serde_json::to_string(&self.profile.provides)
            .map_err(|e| CilError::Io(format!("serialize provides_json: {e}")))?;
        let consumes_json = serde_json::to_string(&self.profile.consumes)
            .map_err(|e| CilError::Io(format!("serialize consumes_json: {e}")))?;
        let inputs_json = serde_json::to_string(&self.profile.inputs)
            .map_err(|e| CilError::Io(format!("serialize inputs_json: {e}")))?;
        let outputs_json = serde_json::to_string(&self.profile.outputs)
            .map_err(|e| CilError::Io(format!("serialize outputs_json: {e}")))?;

        Ok(ProfileColumns {
            skill_id: self.profile.skill_id.clone(),
            provides_json,
            consumes_json,
            inputs_json,
            outputs_json,
            embedding: self.embedding.as_deref().map(encode_embedding),
            profile_epoch: self.profile_epoch,
        })
    }

    /// Reconstruct a row from its `capability_profiles` column values.
    ///
    /// `permissions` are not stored as a column (they are re-derived by the
    /// extractor in task 2.3), so the reconstructed [`CapabilityProfile::permissions`]
    /// is empty here.
    pub fn from_columns(cols: &ProfileColumns) -> Result<Self, CilError> {
        let provides: Vec<CapabilityTag> = serde_json::from_str(&cols.provides_json)
            .map_err(|e| CilError::Io(format!("deserialize provides_json: {e}")))?;
        let consumes: Vec<CapabilityTag> = serde_json::from_str(&cols.consumes_json)
            .map_err(|e| CilError::Io(format!("deserialize consumes_json: {e}")))?;
        let inputs: Vec<String> = serde_json::from_str(&cols.inputs_json)
            .map_err(|e| CilError::Io(format!("deserialize inputs_json: {e}")))?;
        let outputs: Vec<String> = serde_json::from_str(&cols.outputs_json)
            .map_err(|e| CilError::Io(format!("deserialize outputs_json: {e}")))?;
        let embedding = match &cols.embedding {
            Some(bytes) => Some(decode_embedding(bytes)?),
            None => None,
        };
        Ok(Self {
            profile: CapabilityProfile {
                skill_id: cols.skill_id.clone(),
                provides,
                consumes,
                permissions: Vec::new(),
                inputs,
                outputs,
            },
            embedding,
            profile_epoch: cols.profile_epoch,
        })
    }
}

/// Encode an `f32` embedding into the `embedding` BLOB byte layout: contiguous
/// little-endian IEEE-754 `f32` values, 4 bytes each (total length `4 * dim`).
///
/// This little-endian layout is the stable on-disk encoding for the
/// `capability_profiles.embedding` and `market_catalog.embedding` BLOB columns.
pub fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for f in embedding {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// Decode an `embedding` BLOB (little-endian `f32`, 4 bytes each) back into a
/// vector. Returns [`CilError::Io`] if the byte length is not a multiple of 4.
pub fn decode_embedding(bytes: &[u8]) -> Result<Vec<f32>, CilError> {
    if bytes.len() % 4 != 0 {
        return Err(CilError::Io(format!(
            "embedding BLOB length {} is not a multiple of 4 (little-endian f32)",
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::capability::{
        Capability, CapabilityKind, CapabilityMode, CapabilityScope,
    };

    fn sample_profile() -> CapabilityProfile {
        let mut qualifiers = serde_json::Map::new();
        qualifiers.insert("format".into(), serde_json::Value::String("pdf".into()));
        CapabilityProfile {
            skill_id: "acme.pdf.render".into(),
            provides: vec![
                CapabilityTag::new("doc.pdf.render"),
                CapabilityTag {
                    id: "media.image.ocr".into(),
                    qualifiers,
                    embedding: Some(vec![0.1, -0.2, 0.3]),
                },
            ],
            consumes: vec![CapabilityTag::new("io.file.read")],
            permissions: vec![Capability {
                kind: CapabilityKind::Filesystem,
                mode: CapabilityMode::ReadOnly,
                scope: CapabilityScope::Workspace,
            }],
            inputs: vec!["application/pdf".into()],
            outputs: vec!["image/png".into(), "text/plain".into()],
        }
    }

    #[test]
    fn capability_tag_id_is_open_string_not_enum() {
        // A never-before-seen domain is just a string — no code change needed.
        let novel = CapabilityTag::new("quantum.entangle.route");
        assert_eq!(novel.id, "quantum.entangle.route");
        assert!(novel.qualifiers.is_empty());
        assert!(novel.embedding.is_none());
    }

    #[test]
    fn capability_tag_json_roundtrip_omits_absent_embedding() {
        let tag = CapabilityTag::new("io.file.read");
        let json = serde_json::to_string(&tag).unwrap();
        assert!(
            !json.contains("embedding"),
            "absent embedding must be skipped: {json}"
        );
        let back: CapabilityTag = serde_json::from_str(&json).unwrap();
        assert_eq!(tag, back);
    }

    #[test]
    fn capability_tag_qualifiers_default_when_missing() {
        let tag: CapabilityTag = serde_json::from_str(r#"{"id":"net.email.send"}"#).unwrap();
        assert_eq!(tag.id, "net.email.send");
        assert!(tag.qualifiers.is_empty());
        assert!(tag.embedding.is_none());
    }

    #[test]
    fn embedding_encode_decode_roundtrip() {
        let v = vec![0.0_f32, 1.5, -2.25, f32::MIN_POSITIVE, 123456.0];
        let bytes = encode_embedding(&v);
        assert_eq!(bytes.len(), v.len() * 4);
        let back = decode_embedding(&bytes).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn embedding_empty_roundtrip() {
        let bytes = encode_embedding(&[]);
        assert!(bytes.is_empty());
        assert_eq!(decode_embedding(&bytes).unwrap(), Vec::<f32>::new());
    }

    #[test]
    fn decode_embedding_rejects_misaligned_length() {
        let err = decode_embedding(&[0, 1, 2]).unwrap_err();
        assert!(matches!(err, CilError::Io(_)));
    }

    #[test]
    fn profile_row_columns_roundtrip() {
        let row = CapabilityProfileRow {
            profile: sample_profile(),
            embedding: Some(vec![0.5, -0.5, 1.0]),
            profile_epoch: 7,
        };
        let cols = row.to_columns().unwrap();
        assert_eq!(cols.skill_id, "acme.pdf.render");
        assert_eq!(cols.profile_epoch, 7);
        assert!(cols.embedding.is_some());

        let back = CapabilityProfileRow::from_columns(&cols).unwrap();
        assert_eq!(back.profile.skill_id, row.profile.skill_id);
        assert_eq!(back.profile.provides, row.profile.provides);
        assert_eq!(back.profile.consumes, row.profile.consumes);
        assert_eq!(back.profile.inputs, row.profile.inputs);
        assert_eq!(back.profile.outputs, row.profile.outputs);
        assert_eq!(back.embedding, row.embedding);
        assert_eq!(back.profile_epoch, row.profile_epoch);
        // permissions are not a column; they are re-derived by the extractor (task 2.3).
        assert!(back.profile.permissions.is_empty());
    }

    #[test]
    fn profile_row_null_embedding_roundtrip() {
        let row = CapabilityProfileRow {
            profile: sample_profile(),
            embedding: None,
            profile_epoch: 0,
        };
        let cols = row.to_columns().unwrap();
        assert!(cols.embedding.is_none());
        let back = CapabilityProfileRow::from_columns(&cols).unwrap();
        assert!(back.embedding.is_none());
    }

    #[test]
    fn empty_json_columns_deserialize_to_empty_vecs() {
        let cols = ProfileColumns {
            skill_id: "empty.skill".into(),
            provides_json: "[]".into(),
            consumes_json: "[]".into(),
            inputs_json: "[]".into(),
            outputs_json: "[]".into(),
            embedding: None,
            profile_epoch: 0,
        };
        let row = CapabilityProfileRow::from_columns(&cols).unwrap();
        assert!(row.profile.provides.is_empty());
        assert!(row.profile.consumes.is_empty());
        assert!(row.profile.inputs.is_empty());
        assert!(row.profile.outputs.is_empty());
    }
}
