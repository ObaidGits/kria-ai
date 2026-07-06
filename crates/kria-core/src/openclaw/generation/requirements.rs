//! A9.2 Requirement Extraction — turn a user goal into a structured requirement.
//!
//! Backend-agnostic: extraction is performed by a `SkillGenerator` (LLM-backed in
//! production, mock in tests). The extracted `SkillRequirement` drives design +
//! capability inference downstream.

use serde::{Deserialize, Serialize};

/// A structured requirement extracted from a user goal (A9.2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillRequirement {
    /// Concise intent (verb-noun).
    pub intent: String,
    /// Named inputs the skill needs.
    pub inputs: Vec<RequirementField>,
    /// Named outputs the skill produces.
    pub outputs: Vec<RequirementField>,
    /// Free-form constraints (e.g. "must preserve EXIF").
    pub constraints: Vec<String>,
    /// Capabilities the goal implies (filesystem_read, network, subprocess, ...).
    pub implied_capabilities: Vec<String>,
    /// Suggested category.
    pub category: String,
    /// Suggested tags.
    pub tags: Vec<String>,
    /// Declared external dependencies (package names) if any.
    pub dependencies: Vec<String>,
    /// Enumerated failure cases to handle.
    pub failure_cases: Vec<String>,
    /// Enumerated edge cases to handle.
    pub edge_cases: Vec<String>,
    /// Extractor confidence in [0.0, 1.0].
    pub confidence: f64,
}

impl SkillRequirement {
    /// A minimal requirement (used by tests / fallbacks).
    pub fn minimal(intent: impl Into<String>, category: impl Into<String>) -> Self {
        Self {
            intent: intent.into(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            constraints: Vec::new(),
            implied_capabilities: Vec::new(),
            category: category.into(),
            tags: Vec::new(),
            dependencies: Vec::new(),
            failure_cases: Vec::new(),
            edge_cases: Vec::new(),
            confidence: 0.5,
        }
    }
}

/// A named input/output field with a JSON-schema-ish type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequirementField {
    pub name: String,
    /// One of: string, number, integer, boolean, array, object.
    pub ty: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
}

impl RequirementField {
    pub fn new(name: &str, ty: &str, description: &str, required: bool) -> Self {
        Self {
            name: name.to_string(),
            ty: ty.to_string(),
            description: description.to_string(),
            required,
        }
    }
}
