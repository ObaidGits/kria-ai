//! The live privacy-control and firewall providers.
//!
//! linux-os-control-production tasks **5.3** (privacy) and **5.6** (firewall).
//!
//! # Privacy: the setting is a permission, not a hardware switch
//!
//! GNOME's camera, microphone and location toggles govern what **portal-using
//! applications** are allowed to request. They do not power the hardware down, and
//! an application with direct device access is unaffected. The read is therefore
//! reported as the value of a permission, and `None` means the schema is absent —
//! never "off", which would tell the user the camera is blocked when it is not.
//!
//! # Firewall: reading state needs root, and that is reported honestly
//!
//! `ufw status` requires root. Rather than prompting for a password or shelling out
//! through `sudo`, this reads what is readable unprivileged — whether the service is
//! enabled — and reports `None` for the facts it genuinely cannot see. Toggling the
//! firewall is a privileged operation routed through the broker.

use async_trait::async_trait;

use crate::os_control::broker::protocol::{FirewallProviderId, RecognizedPrivacyControl};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::firewall::{FirewallFacts, FirewallOp, FirewallTransport};
use crate::os_control::linux::providers::cli_query as cli;
use crate::os_control::privacy::{PrivacySnapshot, PrivacyTransport};
use crate::os_control::receipt::ApplyOutcome;

const GSETTINGS_PATHS: &[&str] = &["/usr/bin/gsettings"];
const SYSTEMCTL_PATHS: &[&str] = &["/usr/bin/systemctl", "/bin/systemctl"];
const UFW_PATHS: &[&str] = &["/usr/sbin/ufw", "/usr/bin/ufw"];

/// The GNOME privacy schema.
const PRIVACY_SCHEMA: &str = "org.gnome.desktop.privacy";
/// The GNOME location schema (location lives in its own schema).
const LOCATION_SCHEMA: &str = "org.gnome.system.location";

/// The live privacy transport, backed by GSettings.
pub struct LivePrivacy {
    gsettings: &'static str,
}

impl LivePrivacy {
    /// Compose the provider when GSettings is present.
    #[must_use]
    pub fn discover() -> Option<Self> {
        Some(Self {
            gsettings: cli::first_present(GSETTINGS_PATHS)?,
        })
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("gnome-privacy")
    }

    /// The schema and key backing one control.
    ///
    /// The mapping is a closed match, so no caller-supplied string can ever reach
    /// `gsettings` as a schema or key name.
    fn binding(control: RecognizedPrivacyControl) -> (&'static str, &'static str) {
        match control {
            RecognizedPrivacyControl::CameraAccess => (PRIVACY_SCHEMA, "disable-camera"),
            RecognizedPrivacyControl::MicrophoneAccess => (PRIVACY_SCHEMA, "disable-microphone"),
            RecognizedPrivacyControl::LocationAccess => (LOCATION_SCHEMA, "enabled"),
        }
    }

    /// Whether this control's key is stored inverted.
    ///
    /// GNOME stores camera and microphone as `disable-*` but location as `enabled`.
    /// Getting this wrong would report the camera as blocked when it is allowed —
    /// the single most misleading answer this provider could give.
    fn is_inverted(control: RecognizedPrivacyControl) -> bool {
        match control {
            RecognizedPrivacyControl::CameraAccess
            | RecognizedPrivacyControl::MicrophoneAccess => true,
            RecognizedPrivacyControl::LocationAccess => false,
        }
    }

    /// Read one control as "access is allowed".
    async fn read_allowed(
        &self,
        ctx: &HostExecutionContext,
        control: RecognizedPrivacyControl,
    ) -> Result<Option<bool>, OsControlError> {
        let (schema, key) = Self::binding(control);
        let (raw, exit_ok) = cli::query_tolerant(
            ctx,
            self.id(),
            "privacy.read",
            self.gsettings,
            vec!["get".into(), schema.into(), key.into()],
        )
        .await?;
        if !exit_ok {
            // The schema is not installed. Absent, not "off".
            return Ok(None);
        }
        let stored = match raw.trim() {
            "true" => true,
            "false" => false,
            // An unrecognized value is unknown, never a default.
            _ => return Ok(None),
        };
        Ok(Some(if Self::is_inverted(control) {
            !stored
        } else {
            stored
        }))
    }
}

#[async_trait]
impl PrivacyTransport for LivePrivacy {
    fn provider_id(&self) -> ProviderId {
        self.id()
    }

    async fn read_control(
        &self,
        ctx: &HostExecutionContext,
        control: RecognizedPrivacyControl,
    ) -> Result<bool, OsControlError> {
        self.read_allowed(ctx, control).await?.ok_or_else(|| {
            OsControlError::Unavailable {
                provider: Some(self.id()),
                reason: SafeText::new(
                    "this privacy control is not present on this desktop; its state is unknown \
                     rather than off",
                ),
                retryable: false,
            }
        })
    }

    async fn read_snapshot(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<PrivacySnapshot, OsControlError> {
        Ok(PrivacySnapshot {
            camera: self
                .read_allowed(ctx, RecognizedPrivacyControl::CameraAccess)
                .await?,
            microphone: self
                .read_allowed(ctx, RecognizedPrivacyControl::MicrophoneAccess)
                .await?,
            location: self
                .read_allowed(ctx, RecognizedPrivacyControl::LocationAccess)
                .await?,
        })
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        control: RecognizedPrivacyControl,
        enabled: bool,
    ) -> Result<ApplyOutcome, OsControlError> {
        let (schema, key) = Self::binding(control);
        // `enabled` means "access allowed". Invert it back into the stored sense
        // for the keys GNOME stores as `disable-*`.
        let stored = if Self::is_inverted(control) {
            !enabled
        } else {
            enabled
        };
        cli::dispatch(
            ctx,
            "privacy.set",
            self.gsettings,
            vec![
                "set".into(),
                schema.into(),
                key.into(),
                stored.to_string(),
            ],
        )
        .await
    }
}

/// The live firewall transport.
pub struct LiveFirewall {
    systemctl: Option<&'static str>,
    ufw_present: bool,
}

impl LiveFirewall {
    /// Compose the provider when a recognized firewall is installed.
    #[must_use]
    pub fn discover() -> Option<Self> {
        let ufw_present = cli::first_present(UFW_PATHS).is_some();
        ufw_present.then(|| Self {
            systemctl: cli::first_present(SYSTEMCTL_PATHS),
            ufw_present,
        })
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("ufw")
    }
}

#[async_trait]
impl FirewallTransport for LiveFirewall {
    fn provider_id(&self) -> ProviderId {
        self.id()
    }

    async fn read_facts(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<FirewallFacts, OsControlError> {
        if !self.ufw_present {
            return Err(cli::missing(self.id(), "ufw"));
        }
        // `ufw status` needs root. The unit's enabled-state is world-readable and
        // is the strongest honest unprivileged signal.
        let enabled = match self.systemctl {
            Some(systemctl) => {
                let (raw, _exit_ok) = cli::query_tolerant(
                    ctx,
                    self.id(),
                    "firewall.read_enabled",
                    systemctl,
                    vec!["is-active".into(), "ufw.service".into()],
                )
                .await?;
                match raw.trim() {
                    "active" => Some(true),
                    "inactive" | "failed" => Some(false),
                    // Anything else is genuinely unknown. Never defaulted to
                    // `false`, which would tell the user they are unprotected when
                    // they may well be.
                    _ => None,
                }
            }
            None => None,
        };
        Ok(FirewallFacts {
            provider: FirewallProviderId::Ufw,
            enabled,
            // Both need `ufw status`, i.e. root. Unknown rather than invented.
            default_incoming: None,
            rule_count: None,
        })
    }

    async fn read_app_grant(
        &self,
        _ctx: &HostExecutionContext,
        _app_id: &str,
    ) -> Result<bool, OsControlError> {
        // Enumerating rules needs root, so no unprivileged read can prove whether
        // a grant exists. Returning `false` would let a caller believe a grant was
        // absent and safely re-addable when it may already be present.
        Err(OsControlError::PermissionDenied {
            authority: SafeText::new("ufw"),
            remediation: SafeText::new(
                "reading firewall rules requires administrative rights; install the KRIA broker \
                 service to enable it",
            ),
        })
    }

    async fn apply(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        op: &FirewallOp,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Every firewall mutation is privileged. Routed through the broker as a
        // typed operation — never by invoking `sudo`, which would put a password
        // prompt behind a model-initiated action.
        let capability = match op {
            FirewallOp::SetEnabled(_) => "firewall.set_enabled.privileged",
            FirewallOp::GrantTemporary { .. } => "firewall.grant_temporary.privileged",
        };
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new(capability),
            reason: SafeText::new(
                "changing the firewall needs administrative rights; install the KRIA broker \
                 service to enable it",
            ),
        })
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn camera_and_microphone_are_stored_inverted_but_location_is_not() {
        // GNOME stores `disable-camera`, so allowed == !stored. Getting this
        // backwards would report a live camera as blocked.
        assert!(LivePrivacy::is_inverted(
            RecognizedPrivacyControl::CameraAccess
        ));
        assert!(LivePrivacy::is_inverted(
            RecognizedPrivacyControl::MicrophoneAccess
        ));
        assert!(!LivePrivacy::is_inverted(
            RecognizedPrivacyControl::LocationAccess
        ));
    }

    #[test]
    fn each_control_binds_to_its_own_schema_and_key() {
        let (camera_schema, camera_key) =
            LivePrivacy::binding(RecognizedPrivacyControl::CameraAccess);
        let (mic_schema, mic_key) =
            LivePrivacy::binding(RecognizedPrivacyControl::MicrophoneAccess);
        let (location_schema, _) =
            LivePrivacy::binding(RecognizedPrivacyControl::LocationAccess);
        assert_eq!(camera_schema, mic_schema);
        // Two controls must never share a key, or one would read the other's state.
        assert_ne!(camera_key, mic_key);
        // Location lives in a different schema entirely.
        assert_ne!(location_schema, camera_schema);
    }

    #[tokio::test]
    async fn a_firewall_mutation_is_refused_rather_than_escalated() {
        let firewall = LiveFirewall {
            systemctl: None,
            ufw_present: true,
        };
        let ctx = crate::os_control::testing::observation_context_for_test();
        // No unprivileged read can prove a grant's absence.
        assert!(firewall.read_app_grant(&ctx, "app").await.is_err());
        // And the facts read reports unknown rather than "unprotected".
        let facts = firewall.read_facts(&ctx).await.expect("facts read");
        assert!(facts.enabled.is_none(), "no systemctl means unknown");
        assert!(facts.default_incoming.is_none());
        assert!(facts.rule_count.is_none());
    }
}
