//! Deny-live fake [`DesktopAssociationTransport`] for completion tests
//! (OSC-033), Task 3.3.
//!
//! Compiled only under `os-control-test`. It scripts default-application/
//! autostart reads and dispatch outcomes over a plain in-memory map — never
//! a live filesystem or D-Bus transport.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::AdmittedMutationContext;
use crate::os_control::contract::ProviderId;
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{ApplyOutcome, AppliedDispatch};

use super::{DesktopAssociationTransport, DESKTOP_ASSOCIATION_PROVIDER_ID};

/// A scripted, in-memory desktop-association transport.
pub struct FakeDesktopAssociationTransport {
    defaults: Mutex<HashMap<String, String>>,
    autostart: Mutex<HashMap<String, bool>>,
    set_default_calls: Mutex<Vec<(String, String)>>,
    set_autostart_calls: Mutex<Vec<(String, bool)>>,
}

impl FakeDesktopAssociationTransport {
    /// Create a fresh fake with no prior associations.
    #[must_use]
    pub fn new() -> Self {
        Self {
            defaults: Mutex::new(HashMap::new()),
            autostart: Mutex::new(HashMap::new()),
            set_default_calls: Mutex::new(Vec::new()),
            set_autostart_calls: Mutex::new(Vec::new()),
        }
    }

    /// Seed an existing default-application association.
    #[must_use]
    pub fn with_default(self, mime: impl Into<String>, app_id: impl Into<String>) -> Self {
        self.defaults.lock().unwrap().insert(mime.into(), app_id.into());
        self
    }

    /// Seed an existing autostart state.
    #[must_use]
    pub fn with_autostart(self, app_id: impl Into<String>, enabled: bool) -> Self {
        self.autostart.lock().unwrap().insert(app_id.into(), enabled);
        self
    }

    /// The recorded `set_default_application` calls, in order.
    #[must_use]
    pub fn set_default_calls(&self) -> Vec<(String, String)> {
        self.set_default_calls.lock().unwrap().clone()
    }

    /// The recorded `set_autostart` calls, in order.
    #[must_use]
    pub fn set_autostart_calls(&self) -> Vec<(String, bool)> {
        self.set_autostart_calls.lock().unwrap().clone()
    }
}

impl Default for FakeDesktopAssociationTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DesktopAssociationTransport for FakeDesktopAssociationTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(DESKTOP_ASSOCIATION_PROVIDER_ID)
    }

    async fn read_default_application(
        &self,
        mime: &str,
    ) -> Result<Option<String>, OsControlError> {
        Ok(self.defaults.lock().unwrap().get(mime).cloned())
    }

    async fn read_autostart(&self, app_id: &str) -> Result<bool, OsControlError> {
        Ok(*self.autostart.lock().unwrap().get(app_id).unwrap_or(&false))
    }

    async fn set_default_application(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        mime: &str,
        app_id: &str,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.set_default_calls
            .lock()
            .unwrap()
            .push((mime.to_string(), app_id.to_string()));
        self.defaults
            .lock()
            .unwrap()
            .insert(mime.to_string(), app_id.to_string());
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }

    async fn set_autostart(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        app_id: &str,
        enabled: bool,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.set_autostart_calls
            .lock()
            .unwrap()
            .push((app_id.to_string(), enabled));
        self.autostart.lock().unwrap().insert(app_id.to_string(), enabled);
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}
