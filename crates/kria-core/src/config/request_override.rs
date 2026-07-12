//! Turn-scoped configuration overrides (settings-config-revamp Task 14).
//!
//! A [`RequestOverride`] is a per-turn overlay applied at the TOP of precedence
//! for a small whitelist of cheap, safe fields (e.g. `image_generation.image_mode`
//! for "generate this one using local AI"). It is NEVER persisted and is dropped
//! at turn end — so it auto-reverts on success, error, or crash (by construction,
//! since it lives only in the turn's in-memory context). Non-whitelisted fields
//! (auth/network/safety/secrets and anything not `temp_overridable`) are refused.

use crate::config::{schema, KriaConfig};

/// A collection of turn-scoped field overrides.
#[derive(Clone, Debug, Default)]
pub struct RequestOverride {
    fields: Vec<(String, String, serde_json::Value)>,
}

/// Error when a field is not eligible for a temporary override.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OverrideError {
    #[error("field '{0}.{1}' is not allowed as a temporary override")]
    NotAllowed(String, String),
    #[error("invalid override value for '{0}.{1}': {2}")]
    Invalid(String, String, String),
}

impl RequestOverride {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Add a whitelisted field override for this turn. Rejects any field that is
    /// not `temp_overridable` in the schema (auth/network/safety/secrets etc.).
    pub fn set(
        &mut self,
        section: &str,
        field: &str,
        value: serde_json::Value,
    ) -> Result<(), OverrideError> {
        // Must be a valid, temp-overridable field with an allowed value.
        match schema::validate_change(section, field, &value, true) {
            Ok(_) => {
                // De-dupe: last write wins per (section, field).
                self.fields
                    .retain(|(s, f, _)| !(s == section && f == field));
                self.fields
                    .push((section.to_string(), field.to_string(), value));
                Ok(())
            }
            Err(schema::SchemaError::NotTempOverridable(s, f)) => {
                Err(OverrideError::NotAllowed(s, f))
            }
            Err(schema::SchemaError::NotPromptChangeable(s, f)) => {
                Err(OverrideError::NotAllowed(s, f))
            }
            Err(schema::SchemaError::UnknownField(s, f)) => Err(OverrideError::NotAllowed(s, f)),
            Err(e) => Err(OverrideError::Invalid(
                section.to_string(),
                field.to_string(),
                e.to_string(),
            )),
        }
    }

    /// Overlay these overrides onto a config clone (top of precedence). Applied
    /// to a per-turn copy — the persisted config is never touched.
    pub fn apply_to(&self, cfg: &mut KriaConfig) {
        if self.fields.is_empty() {
            return;
        }
        if let Ok(mut root) = serde_json::to_value(&*cfg) {
            if let Some(obj) = root.as_object_mut() {
                for (section, field, value) in &self.fields {
                    if let Some(sect) = obj.get_mut(section).and_then(|s| s.as_object_mut()) {
                        sect.insert(field.clone(), value.clone());
                    }
                }
            }
            if let Ok(applied) = serde_json::from_value::<KriaConfig>(root) {
                *cfg = applied;
            }
        }
    }

    /// Produce a turn-scoped config = `base` with the overrides applied.
    pub fn overlay(&self, base: &KriaConfig) -> KriaConfig {
        let mut cfg = base.clone();
        self.apply_to(&mut cfg);
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelisted_field_can_be_set_and_overlaid() {
        let mut ov = RequestOverride::new();
        assert!(ov.is_empty());
        ov.set(
            "image_generation",
            "image_mode",
            serde_json::json!("local_only"),
        )
        .expect("image_mode is temp-overridable");
        assert!(!ov.is_empty());

        let base = KriaConfig::default();
        assert_eq!(base.image_generation.image_mode, "auto");
        let turn = ov.overlay(&base);
        assert_eq!(turn.image_generation.image_mode, "local_only");
        // base untouched (no persistence / no leak)
        assert_eq!(base.image_generation.image_mode, "auto");
    }

    #[test]
    fn non_whitelisted_field_is_refused() {
        let mut ov = RequestOverride::new();
        // ui.theme is prompt-changeable but NOT temp-overridable.
        let err = ov
            .set("ui", "theme", serde_json::json!("dark"))
            .unwrap_err();
        assert!(matches!(err, OverrideError::NotAllowed(_, _)));
        assert!(ov.is_empty());
    }

    #[test]
    fn secret_and_auth_fields_are_refused() {
        let mut ov = RequestOverride::new();
        assert!(ov
            .set("server", "enable_auth", serde_json::json!(false))
            .is_err());
        assert!(ov
            .set("llm", "cloud_api_key", serde_json::json!("sk-x"))
            .is_err());
    }

    #[test]
    fn empty_override_is_noop() {
        let ov = RequestOverride::new();
        let base = KriaConfig::default();
        let turn = ov.overlay(&base);
        assert_eq!(
            turn.image_generation.image_mode,
            base.image_generation.image_mode
        );
    }

    #[test]
    fn multiple_overrides_all_apply() {
        let mut ov = RequestOverride::new();
        ov.set(
            "image_generation",
            "image_mode",
            serde_json::json!("cloud_only"),
        )
        .unwrap();
        ov.set(
            "image_generation",
            "tier_override",
            serde_json::json!("a_standard"),
        )
        .unwrap();
        let turn = ov.overlay(&KriaConfig::default());
        assert_eq!(turn.image_generation.image_mode, "cloud_only");
        assert_eq!(turn.image_generation.tier_override, "a_standard");
    }
}
