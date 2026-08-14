//! Deny-live fake [`CredentialStore`] (OSC-007, OSC-025, OSC-026, OSC-029), Task 1.10.
//!
//! Compiled only under `os-control-test`. It stores, replaces, deletes and
//! resolves credentials in a plain in-memory map — never the freedesktop Secret
//! Service, a D-Bus connection, or a keyring process.
//!
//! It preserves every fail-closed rule the live store must honour:
//! * a locked / unavailable service fails closed with no plaintext fallback,
//! * an unknown reference fails closed before any mutation,
//! * a purpose/scope mismatch on resolution is denied (OSC-025.3),
//! * an expired secret does not resolve,
//! * values leave only as a [`SecretPayload`], which cannot serialize or clone.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, SafeText};
use crate::os_control::error::OsControlError;

use super::{
    purpose_scope_mismatch, service_unavailable, unknown_reference, CredentialStore,
    ProtectedInputHandle, SecretMetadata, SecretMetadataPage, SecretPayload, SecretPurpose,
    SecretRef, SecretResolutionRequest, SecretScope, SecretServiceState,
    SECRET_METADATA_PAGE_CAP,
};

/// One stored entry: value-free metadata plus the raw bytes held only in memory.
struct FakeEntry {
    metadata: SecretMetadata,
    bytes: Vec<u8>,
}

/// A scripted, in-memory credential store.
///
/// `now_unix` is injected so expiry assertions are deterministic, and
/// [`Self::with_state`] scripts a locked/unavailable backend so the fail-closed
/// paths can be exercised without a real keyring.
pub struct FakeCredentialStore {
    state: SecretServiceState,
    now_unix: u64,
    next_id: Mutex<u64>,
    entries: Mutex<HashMap<SecretRef, FakeEntry>>,
    resolve_calls: Mutex<Vec<SecretResolutionRequest>>,
    store_calls: Mutex<Vec<SecretRef>>,
    delete_calls: Mutex<Vec<SecretRef>>,
}

impl FakeCredentialStore {
    /// Create a fresh, empty, available store with `now = 0`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: SecretServiceState::Available,
            now_unix: 0,
            next_id: Mutex::new(1),
            entries: Mutex::new(HashMap::new()),
            resolve_calls: Mutex::new(Vec::new()),
            store_calls: Mutex::new(Vec::new()),
            delete_calls: Mutex::new(Vec::new()),
        }
    }

    /// Builder: script the backend lock/availability state.
    #[must_use]
    pub fn with_state(mut self, state: SecretServiceState) -> Self {
        self.state = state;
        self
    }

    /// Builder: fix the clock used for creation/expiry checks.
    #[must_use]
    pub fn with_now(mut self, now_unix: u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    /// Builder: seed an existing secret, returning `self` for chaining.
    #[must_use]
    pub fn with_secret(
        self,
        reference: SecretRef,
        purpose: SecretPurpose,
        scope: SecretScope,
        bytes: Vec<u8>,
    ) -> Self {
        self.insert_entry(reference, purpose, scope, SafeText::new("seeded"), None, bytes);
        self
    }

    /// Builder: seed a secret that expires at `expires_unix`.
    #[must_use]
    pub fn with_expiring_secret(
        self,
        reference: SecretRef,
        purpose: SecretPurpose,
        scope: SecretScope,
        expires_unix: u64,
        bytes: Vec<u8>,
    ) -> Self {
        self.insert_entry(
            reference,
            purpose,
            scope,
            SafeText::new("seeded"),
            Some(expires_unix),
            bytes,
        );
        self
    }

    /// The resolution requests seen by this store, in order.
    #[must_use]
    pub fn resolve_calls(&self) -> Vec<SecretResolutionRequest> {
        self.resolve_calls.lock().unwrap().clone()
    }

    /// The references stored/replaced through this store, in order.
    #[must_use]
    pub fn store_calls(&self) -> Vec<SecretRef> {
        self.store_calls.lock().unwrap().clone()
    }

    /// The references deleted through this store, in order.
    #[must_use]
    pub fn delete_calls(&self) -> Vec<SecretRef> {
        self.delete_calls.lock().unwrap().clone()
    }

    /// Whether a reference is currently held.
    #[must_use]
    pub fn contains(&self, reference: &SecretRef) -> bool {
        self.entries.lock().unwrap().contains_key(reference)
    }

    /// How many secrets are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// Whether the store holds no secrets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.lock().unwrap().is_empty()
    }

    fn insert_entry(
        &self,
        reference: SecretRef,
        purpose: SecretPurpose,
        scope: SecretScope,
        label: SafeText,
        expires_unix: Option<u64>,
        bytes: Vec<u8>,
    ) {
        let metadata = SecretMetadata {
            reference: reference.clone(),
            purpose,
            scope,
            label,
            created_unix: self.now_unix,
            expires_unix,
        };
        self.entries
            .lock()
            .unwrap()
            .insert(reference, FakeEntry { metadata, bytes });
    }

    /// Fail closed when the backend is not usable — no plaintext fallback.
    fn guard_available(&self) -> Result<(), OsControlError> {
        match self.state {
            SecretServiceState::Available => Ok(()),
            other => Err(service_unavailable(other)),
        }
    }

    fn mint_ref(&self) -> SecretRef {
        let mut n = self.next_id.lock().unwrap();
        let r = SecretRef::new(format!("fake-secret-{n}"));
        *n += 1;
        r
    }
}

impl Default for FakeCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CredentialStore for FakeCredentialStore {
    async fn list_metadata(
        &self,
        _ctx: &HostExecutionContext,
        purpose: Option<SecretPurpose>,
        _cursor: Option<&str>,
        limit: u16,
    ) -> Result<SecretMetadataPage, OsControlError> {
        self.guard_available()?;
        let cap = (limit as usize).clamp(1, SECRET_METADATA_PAGE_CAP);
        let entries = self.entries.lock().unwrap();
        let mut items: Vec<SecretMetadata> = entries
            .values()
            .filter(|e| purpose.is_none_or(|p| e.metadata.purpose == p))
            .map(|e| e.metadata.clone())
            .collect();
        // Deterministic order so snapshot assertions are stable.
        items.sort_by(|a, b| a.reference.cmp(&b.reference));
        Ok(SecretMetadataPage {
            items: BoundedVec::from_iter_capped(items, cap),
            next_cursor: None,
        })
    }

    async fn store(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        purpose: SecretPurpose,
        scope: SecretScope,
        label: SafeText,
        input: ProtectedInputHandle,
    ) -> Result<SecretMetadata, OsControlError> {
        self.guard_available()?;
        let reference = self.mint_ref();
        let bytes = input.into_payload().expose_secret().to_vec();
        self.insert_entry(reference.clone(), purpose, scope, label, None, bytes);
        self.store_calls.lock().unwrap().push(reference.clone());
        let entries = self.entries.lock().unwrap();
        Ok(entries
            .get(&reference)
            .expect("just inserted")
            .metadata
            .clone())
    }

    async fn replace(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        reference: &SecretRef,
        input: ProtectedInputHandle,
    ) -> Result<SecretMetadata, OsControlError> {
        self.guard_available()?;
        let bytes = input.into_payload().expose_secret().to_vec();
        let mut entries = self.entries.lock().unwrap();
        // Fail closed before mutating anything when the reference is unknown.
        let entry = entries.get_mut(reference).ok_or_else(unknown_reference)?;
        entry.bytes = bytes;
        self.store_calls.lock().unwrap().push(reference.clone());
        Ok(entry.metadata.clone())
    }

    async fn delete(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        reference: &SecretRef,
    ) -> Result<(), OsControlError> {
        self.guard_available()?;
        let mut entries = self.entries.lock().unwrap();
        if entries.remove(reference).is_none() {
            return Err(unknown_reference());
        }
        self.delete_calls.lock().unwrap().push(reference.clone());
        Ok(())
    }

    async fn resolve_for_operation(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &SecretResolutionRequest,
    ) -> Result<SecretPayload, OsControlError> {
        self.resolve_calls.lock().unwrap().push(request.clone());
        self.guard_available()?;
        let entries = self.entries.lock().unwrap();
        let entry = entries.get(&request.reference).ok_or_else(unknown_reference)?;
        // Purpose AND scope must match the stored binding exactly (OSC-025.3).
        if entry.metadata.purpose != request.purpose || entry.metadata.scope != request.scope {
            return Err(purpose_scope_mismatch());
        }
        if entry.metadata.is_expired(self.now_unix) {
            return Err(unknown_reference());
        }
        Ok(SecretPayload::new(entry.bytes.clone()))
    }
}
