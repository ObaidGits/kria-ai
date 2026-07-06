//! Bundle lifecycle events (A2.9).
//!
//! Distinct from the execution `SkillEvent` stream (event-contract, whose `Stage` set is frozen to
//! *execution* stages). This is the install/registry lifecycle stream: one enum, one bus, no
//! duplicate logging.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BundleLifecycleEvent {
    Installing {
        slug: String,
        version: String,
    },
    Installed {
        slug: String,
        version: String,
    },
    Updated {
        slug: String,
        from: String,
        to: String,
    },
    Removed {
        slug: String,
    },
    Enabled {
        slug: String,
    },
    Disabled {
        slug: String,
    },
    RolledBack {
        slug: String,
        reason: String,
    },
    Failed {
        slug: String,
        reason: String,
    },
}

impl BundleLifecycleEvent {
    pub fn slug(&self) -> &str {
        match self {
            Self::Installing { slug, .. }
            | Self::Installed { slug, .. }
            | Self::Updated { slug, .. }
            | Self::Removed { slug }
            | Self::Enabled { slug }
            | Self::Disabled { slug }
            | Self::RolledBack { slug, .. }
            | Self::Failed { slug, .. } => slug,
        }
    }
}

static BUS: OnceLock<broadcast::Sender<BundleLifecycleEvent>> = OnceLock::new();

fn sender() -> &'static broadcast::Sender<BundleLifecycleEvent> {
    BUS.get_or_init(|| {
        let (tx, _rx) = broadcast::channel(128);
        tx
    })
}

pub fn subscribe() -> broadcast::Receiver<BundleLifecycleEvent> {
    sender().subscribe()
}

pub fn emit(event: BundleLifecycleEvent) {
    tracing::info!(target: "openclaw::bundle", event = ?event, "bundle lifecycle event");
    let _ = sender().send(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lifecycle_events_broadcast() {
        let mut rx = subscribe();
        emit(BundleLifecycleEvent::Installing {
            slug: "oc_x".into(),
            version: "1.0.0".into(),
        });
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.slug(), "oc_x");
    }
}
