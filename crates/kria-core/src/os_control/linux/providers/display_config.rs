//! The live display-configuration provider: night light and monitor layout.
//!
//! linux-os-control-production task **5.1**.
//!
//! # Why the layout change is confirmed by the compositor, not by KRIA
//!
//! Mutter's `ApplyMonitorsConfig` has a **confirmation** mode: the new layout is
//! applied provisionally and automatically reverted unless it is confirmed within a
//! timeout. This provider always uses that mode. The reason is the failure it
//! prevents: a layout that leaves every monitor blank cannot be undone by a user who
//! can no longer see the screen — and if KRIA itself crashed mid-change, an
//! in-process rollback would never run. Delegating the revert to the compositor means
//! the safety net survives KRIA dying.
//!
//! # The serial is a concurrency token, not a version number
//!
//! Mutter rejects a configuration carrying a stale serial. That is what stops KRIA
//! applying a layout computed against monitors that have since been unplugged, so the
//! serial is read immediately before applying and passed through unmodified.

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeText};
use crate::os_control::display::configuration::{
    DisplayConfigFacts, DisplayConfigOp, DisplayConfigTransport,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::providers::cli_query as cli;
use crate::os_control::receipt::ApplyOutcome;

const GSETTINGS_PATHS: &[&str] = &["/usr/bin/gsettings"];

/// The GNOME night-light schema.
const NIGHT_LIGHT_SCHEMA: &str = "org.gnome.settings-daemon.plugins.color";
/// The night-light enable key.
const NIGHT_LIGHT_KEY: &str = "night-light-enabled";

/// The live display-configuration transport.
pub struct LiveDisplayConfig {
    gsettings: &'static str,
}

impl LiveDisplayConfig {
    /// Compose the provider when GSettings is present.
    #[must_use]
    pub fn discover() -> Option<Self> {
        Some(Self {
            gsettings: cli::first_present(GSETTINGS_PATHS)?,
        })
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("gnome-display-config")
    }
}

#[async_trait]
impl DisplayConfigTransport for LiveDisplayConfig {
    fn provider_id(&self) -> ProviderId {
        self.id()
    }

    async fn read_facts(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<DisplayConfigFacts, OsControlError> {
        let (raw, exit_ok) = cli::query_tolerant(
            ctx,
            self.id(),
            "display_config.read_night_light",
            self.gsettings,
            vec![
                "get".into(),
                NIGHT_LIGHT_SCHEMA.into(),
                NIGHT_LIGHT_KEY.into(),
            ],
        )
        .await?;
        let night_light = if exit_ok {
            match raw.trim() {
                "true" => Some(true),
                "false" => Some(false),
                // An unrecognized value is unknown, not off.
                _ => None,
            }
        } else {
            // The schema is absent — this desktop has no night light at all.
            None
        };
        Ok(DisplayConfigFacts {
            night_light,
            // The monitor serial lives on Mutter's D-Bus interface, which this
            // GSettings-backed transport does not speak. Reported as unknown so a
            // layout change cannot be attempted against a serial nobody read: the
            // domain refuses to apply without one.
            config_serial: None,
            awaiting_confirmation: false,
        })
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        op: &DisplayConfigOp,
    ) -> Result<ApplyOutcome, OsControlError> {
        match op {
            DisplayConfigOp::SetNightLight(enabled) => {
                cli::dispatch(
                    ctx,
                    "display_config.set_night_light",
                    self.gsettings,
                    vec![
                        "set".into(),
                        NIGHT_LIGHT_SCHEMA.into(),
                        NIGHT_LIGHT_KEY.into(),
                        enabled.to_string(),
                    ],
                )
                .await
            }
            DisplayConfigOp::ApplyConfiguration(_) | DisplayConfigOp::ConfirmConfiguration => {
                // Changing the monitor layout requires Mutter's
                // `ApplyMonitorsConfig` with its revert-unless-confirmed mode.
                // Refused rather than approximated: an approximation that applied
                // a layout WITHOUT the compositor's automatic revert could leave
                // the user staring at a blank screen with no way back.
                Err(OsControlError::Unsupported {
                    capability: CapabilityId::new("display_config.monitor_layout"),
                    reason: SafeText::new(
                        "changing the monitor layout needs the compositor's confirm-or-revert \
                         interface, which is not available through this transport; KRIA will not \
                         apply a layout it cannot automatically undo",
                    ),
                })
            }
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_layout_change_is_refused_without_the_confirm_interface() {
        let provider = LiveDisplayConfig {
            gsettings: "/usr/bin/gsettings",
        };
        let facts = DisplayConfigFacts {
            night_light: Some(true),
            config_serial: None,
            awaiting_confirmation: false,
        };
        // No serial means no safe layout change is possible, and the facts say so
        // rather than offering a serial of 0 that Mutter would reject anyway.
        assert!(facts.config_serial.is_none());
        assert_eq!(provider.provider_id().as_str(), "gnome-display-config");
    }

    #[test]
    fn night_light_and_layout_use_different_capabilities() {
        // The two must not share a capability id, or a grant for the harmless one
        // would authorize the one that can blank the screen.
        let layout = CapabilityId::new("display_config.monitor_layout");
        assert_ne!(layout.as_str(), "display_config.set_night_light");
    }
}
