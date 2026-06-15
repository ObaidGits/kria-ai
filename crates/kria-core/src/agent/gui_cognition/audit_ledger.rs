//! Task 9.2 (Requirements 10, 11, 12, 13, 14, 15, 22, 23): an APPEND-ONLY,
//! SANITIZED audit ledger of EXECUTED GUI actions, scoped to GUI cognition.
//!
//! This is distinct from the broader `safety/audit.rs` audit layer: it is a
//! GUI-cognition-scoped, per-turn record of the actions the runtime actually
//! executed (backend ran), captured for tamper-evident inspection.
//!
//! Invariants (KRIA runtime authority — the audit ledger is sanitized; no
//! secrets):
//! - **Append-only**: entries are added in order and NEVER mutated or removed.
//!   [`GuiActionLedger`] exposes only `append` + read accessors; there is no
//!   mutating accessor into a stored entry.
//! - **Sanitized**: an entry records only a sanitized target *descriptor*
//!   (label/role), the `execution_id`/`proposal_hash`, the authorization
//!   source (safe-no-approval vs HITL-approved + decision id), the verification
//!   verdict, timestamps, and the `prompt_hash`. It NEVER records secret
//!   payloads (passwords/clipboard contents), raw coordinates, or the raw
//!   prompt — only the redacted summary/hash already produced upstream.
//! - **Tamper-evident**: each entry carries a `prev_hash`/`entry_hash` chain
//!   (`blake3` via [`stable_hash`], consistent with the receipt-hash pattern in
//!   `workflow_runtime::compute_receipt_hash`).
//!
//! This module is only exercised when the `gui_cog_safety_polish` flag is ON
//! (Task 9, default OFF until the wave gate 9.7). While the flag is OFF the
//! ledger is never populated and no ledger event is emitted.

use super::perception::{sanitize_gui_text, stable_hash};

/// Genesis hash that seeds the chain for the first ledger entry. Constant so a
/// ledger's chain is reproducible / verifiable from its entries alone.
pub const LEDGER_GENESIS_HASH: &str = "gui_action_ledger_genesis";

/// Cap on a sanitized descriptor field length (defense-in-depth; descriptors
/// are short labels/roles, never free text).
const DESCRIPTOR_LIMIT: usize = 80;

/// The sanitized inputs used to record one executed action. The ledger turns
/// this into a hashed, append-only [`GuiActionLedgerEntry`]; the record itself
/// carries no secret payload, no coordinates, and no raw prompt — only the
/// redacted descriptor + hashes produced upstream.
#[derive(Debug, Clone)]
pub struct GuiActionLedgerRecord {
    pub action_type: String,
    pub target_label: Option<String>,
    pub target_role: Option<String>,
    pub execution_id: String,
    pub proposal_hash: String,
    pub authorization_source: String,
    pub hitl_decision_id: Option<String>,
    pub verification_verdict: String,
    pub is_secret_payload: bool,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub prompt_hash: String,
}

/// One append-only, sanitized ledger entry for an executed GUI action.
///
/// Every text field that originates from observed UI (`target_label`,
/// `target_role`) is sanitized on construction; the value/coordinates are never
/// present. The `entry_hash` chains to `prev_hash` for tamper evidence.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuiActionLedgerEntry {
    /// 0-based append order; equals the entry's index in the ledger.
    pub sequence: u64,
    pub action_type: String,
    /// Sanitized target descriptor (label) — never raw secrets/coordinates.
    pub target_label: Option<String>,
    /// Sanitized target descriptor (role) — never raw secrets/coordinates.
    pub target_role: Option<String>,
    pub execution_id: String,
    pub proposal_hash: String,
    /// `safe_no_approval_required` or `hitl_approved`.
    pub authorization_source: String,
    /// The HITL decision id when the action was HITL-approved; `None` for a
    /// safe (no-approval) action.
    pub hitl_decision_id: Option<String>,
    /// `verified` / `verification_failed` / `inconclusive` / `blocked`.
    pub verification_verdict: String,
    /// Whether the action carried a secret payload (a flag only — the value is
    /// NEVER recorded).
    pub is_secret_payload: bool,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub prompt_hash: String,
    /// Hash of the previous entry (or [`LEDGER_GENESIS_HASH`] for the first),
    /// forming the tamper-evident chain.
    pub prev_hash: String,
    /// Tamper-evident hash over this entry's canonical fields + `prev_hash`.
    pub entry_hash: String,
}

impl GuiActionLedgerEntry {
    /// Recompute the entry hash from canonical fields. Used internally on
    /// `append` and exposed for verification ([`GuiActionLedger::verify_chain`]).
    fn compute_hash(
        sequence: u64,
        action_type: &str,
        target_label: Option<&str>,
        target_role: Option<&str>,
        execution_id: &str,
        proposal_hash: &str,
        authorization_source: &str,
        hitl_decision_id: Option<&str>,
        verification_verdict: &str,
        is_secret_payload: bool,
        started_at_ms: i64,
        completed_at_ms: i64,
        prompt_hash: &str,
        prev_hash: &str,
    ) -> String {
        stable_hash(&format!(
            "{prev_hash}|{sequence}|{action_type}|{}|{}|{execution_id}|{proposal_hash}|{authorization_source}|{}|{verification_verdict}|{}|{started_at_ms}|{completed_at_ms}|{prompt_hash}",
            target_label.unwrap_or(""),
            target_role.unwrap_or(""),
            hitl_decision_id.unwrap_or(""),
            if is_secret_payload { "secret" } else { "plain" },
        ))
    }

    /// Non-revealing JSON summary for telemetry / the turn's event stream and
    /// the inspectable read API. Carries no secret payload / coordinate / raw
    /// prompt.
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "sequence": self.sequence,
            "action_type": self.action_type,
            "target_label": self.target_label,
            "target_role": self.target_role,
            "execution_id": self.execution_id,
            "proposal_hash": self.proposal_hash,
            "authorization_source": self.authorization_source,
            "hitl_decision_id": self.hitl_decision_id,
            "verification_verdict": self.verification_verdict,
            "is_secret_payload": self.is_secret_payload,
            "started_at_ms": self.started_at_ms,
            "completed_at_ms": self.completed_at_ms,
            "prompt_hash": self.prompt_hash,
            "prev_hash": self.prev_hash,
            "entry_hash": self.entry_hash,
        })
    }
}

/// An append-only, sanitized, tamper-evident ledger of executed GUI actions.
///
/// The internal `Vec` is private and only ever grows via [`append`](Self::append);
/// there is no API to mutate or remove a recorded entry. Reads go through
/// [`entries`](Self::entries) / [`summary_json`](Self::summary_json).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GuiActionLedger {
    entries: Vec<GuiActionLedgerEntry>,
}

impl GuiActionLedger {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Hash at the head of the chain — the last entry's `entry_hash`, or
    /// [`LEDGER_GENESIS_HASH`] when empty.
    pub fn head_hash(&self) -> String {
        self.entries
            .last()
            .map(|entry| entry.entry_hash.clone())
            .unwrap_or_else(|| LEDGER_GENESIS_HASH.to_string())
    }

    /// Append a sanitized record as the next ledger entry and return a
    /// reference to it. Append-only: the new entry is pushed to the end with
    /// the next sequence number and chained to the current head; existing
    /// entries are never touched.
    ///
    /// Target descriptors are sanitized here (label/role) so even a caller that
    /// forgets to sanitize cannot leak raw UI text; secret payload values,
    /// coordinates, and the raw prompt are structurally absent from
    /// [`GuiActionLedgerRecord`].
    pub fn append(&mut self, record: GuiActionLedgerRecord) -> &GuiActionLedgerEntry {
        let sequence = self.entries.len() as u64;
        let prev_hash = self.head_hash();

        let target_label = sanitize_descriptor(record.target_label.as_deref());
        let target_role = sanitize_descriptor(record.target_role.as_deref());

        let entry_hash = GuiActionLedgerEntry::compute_hash(
            sequence,
            &record.action_type,
            target_label.as_deref(),
            target_role.as_deref(),
            &record.execution_id,
            &record.proposal_hash,
            &record.authorization_source,
            record.hitl_decision_id.as_deref(),
            &record.verification_verdict,
            record.is_secret_payload,
            record.started_at_ms,
            record.completed_at_ms,
            &record.prompt_hash,
            &prev_hash,
        );

        self.entries.push(GuiActionLedgerEntry {
            sequence,
            action_type: record.action_type,
            target_label,
            target_role,
            execution_id: record.execution_id,
            proposal_hash: record.proposal_hash,
            authorization_source: record.authorization_source,
            hitl_decision_id: record.hitl_decision_id,
            verification_verdict: record.verification_verdict,
            is_secret_payload: record.is_secret_payload,
            started_at_ms: record.started_at_ms,
            completed_at_ms: record.completed_at_ms,
            prompt_hash: record.prompt_hash,
            prev_hash,
            entry_hash,
        });
        // Safe: just pushed.
        self.entries.last().expect("entry was just appended")
    }

    /// Inspectable read API: the recorded entries in append order.
    pub fn entries(&self) -> &[GuiActionLedgerEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Inspectable JSON view of the whole ledger (entries + chain head).
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "entry_count": self.entries.len(),
            "head_hash": self.head_hash(),
            "entries": self
                .entries
                .iter()
                .map(GuiActionLedgerEntry::summary_json)
                .collect::<Vec<_>>(),
        })
    }

    /// The event emitted when a ledger entry is recorded, so the executed
    /// action surfaces in the turn's event stream. Carries the recorded entry +
    /// the post-append ledger size and chain head.
    pub fn entry_recorded_event(&self, entry: &GuiActionLedgerEntry) -> serde_json::Value {
        serde_json::json!({
            "type": "GuiActionLedgerEntryRecorded",
            "entry": entry.summary_json(),
            "entry_count": self.entries.len(),
            "head_hash": self.head_hash(),
        })
    }

    /// Verify the tamper-evident chain: every entry's `prev_hash` links to the
    /// prior entry's `entry_hash` (genesis for the first), and every
    /// `entry_hash` recomputes from its canonical fields. Returns `true` for an
    /// intact, unmodified ledger.
    pub fn verify_chain(&self) -> bool {
        let mut expected_prev = LEDGER_GENESIS_HASH.to_string();
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.sequence != index as u64 {
                return false;
            }
            if entry.prev_hash != expected_prev {
                return false;
            }
            let recomputed = GuiActionLedgerEntry::compute_hash(
                entry.sequence,
                &entry.action_type,
                entry.target_label.as_deref(),
                entry.target_role.as_deref(),
                &entry.execution_id,
                &entry.proposal_hash,
                &entry.authorization_source,
                entry.hitl_decision_id.as_deref(),
                &entry.verification_verdict,
                entry.is_secret_payload,
                entry.started_at_ms,
                entry.completed_at_ms,
                &entry.prompt_hash,
                &entry.prev_hash,
            );
            if recomputed != entry.entry_hash {
                return false;
            }
            expected_prev = entry.entry_hash.clone();
        }
        true
    }
}

/// Sanitize a target descriptor (label/role). Empty/whitespace becomes `None`
/// so the ledger never stores a meaningless blank; otherwise the value is run
/// through the shared GUI text sanitizer and length-capped.
fn sanitize_descriptor(value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    let sanitized = sanitize_gui_text(value, DESCRIPTOR_LIMIT).text;
    if sanitized.trim().is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

#[cfg(test)]
mod tests {
    //! Task 9.2 (Requirements 10, 11, 12, 13, 14, 15, 22, 23) T1: the GUI-action
    //! audit ledger is APPEND-ONLY (entries preserved + ordered, never mutated),
    //! SANITIZED (no secret/clipboard value, no coordinates, no raw prompt),
    //! tamper-evident (chained hash), and inspectable.

    use super::*;

    const RAW_PROMPT_MARKER: &str = "ZZZRAWPROMPTMARKER";
    const SECRET_MARKER: &str = "hunter2-super-secret";

    fn record(action: &str, exec_id: &str) -> GuiActionLedgerRecord {
        GuiActionLedgerRecord {
            action_type: action.into(),
            target_label: Some("Search".into()),
            target_role: Some("text".into()),
            execution_id: exec_id.into(),
            proposal_hash: format!("phash-{exec_id}"),
            authorization_source: "safe_no_approval_required".into(),
            hitl_decision_id: None,
            verification_verdict: "verified".into(),
            is_secret_payload: false,
            started_at_ms: 1_000,
            completed_at_ms: 1_050,
            prompt_hash: "prompt-hash-abc".into(),
        }
    }

    #[test]
    fn append_is_ordered_and_sequenced() {
        let mut ledger = GuiActionLedger::new();
        assert!(ledger.is_empty());
        ledger.append(record("OpenApp", "e0"));
        ledger.append(record("FocusField", "e1"));
        ledger.append(record("TypeText", "e2"));

        assert_eq!(ledger.len(), 3);
        let entries = ledger.entries();
        assert_eq!(entries[0].sequence, 0);
        assert_eq!(entries[1].sequence, 1);
        assert_eq!(entries[2].sequence, 2);
        // Insertion order is preserved.
        assert_eq!(entries[0].execution_id, "e0");
        assert_eq!(entries[1].execution_id, "e1");
        assert_eq!(entries[2].execution_id, "e2");
    }

    #[test]
    fn append_only_preserves_prior_entries_unmutated() {
        let mut ledger = GuiActionLedger::new();
        let first = ledger.append(record("OpenApp", "e0")).clone();
        ledger.append(record("FocusField", "e1"));
        ledger.append(record("ClickControl", "e2"));

        // The first entry is byte-for-byte identical after later appends: an
        // append never mutates or removes an existing entry.
        assert_eq!(ledger.entries()[0], first);
        // There is no API to remove/replace an entry; the only mutation is
        // growth. The length only ever increases.
        assert_eq!(ledger.len(), 3);
    }

    #[test]
    fn chain_links_each_entry_to_the_previous() {
        let mut ledger = GuiActionLedger::new();
        ledger.append(record("OpenApp", "e0"));
        ledger.append(record("FocusField", "e1"));
        ledger.append(record("TypeText", "e2"));

        let entries = ledger.entries();
        assert_eq!(entries[0].prev_hash, LEDGER_GENESIS_HASH);
        assert_eq!(entries[1].prev_hash, entries[0].entry_hash);
        assert_eq!(entries[2].prev_hash, entries[1].entry_hash);
        assert_eq!(ledger.head_hash(), entries[2].entry_hash);
        // The intact ledger verifies.
        assert!(ledger.verify_chain());
    }

    #[test]
    fn verify_chain_detects_tampering() {
        let mut ledger = GuiActionLedger::new();
        ledger.append(record("OpenApp", "e0"));
        ledger.append(record("FocusField", "e1"));
        assert!(ledger.verify_chain());

        // Tamper with a recorded entry's field WITHOUT recomputing its hash:
        // the chain verification must fail (tamper-evidence).
        ledger.entries[0].verification_verdict = "verification_failed".into();
        assert!(!ledger.verify_chain());
    }

    #[test]
    fn descriptor_sanitized_and_blank_dropped() {
        let mut ledger = GuiActionLedger::new();
        let mut rec = record("FocusField", "e0");
        rec.target_label = Some("   ".into()); // blank → None
        rec.target_role = Some("text\nrole".into()); // newline collapsed by sanitizer
        ledger.append(rec);

        let entry = &ledger.entries()[0];
        assert_eq!(entry.target_label, None);
        let role = entry.target_role.as_deref().unwrap();
        assert!(!role.contains('\n'), "descriptor must be sanitized: {role:?}");
    }

    #[test]
    fn secret_payload_records_only_the_flag_never_a_value() {
        // A secret-payload action: the record carries no value field at all —
        // structurally only the `is_secret_payload` flag is recorded. The
        // serialized entry must never contain the secret marker.
        let mut ledger = GuiActionLedger::new();
        let mut rec = record("TypeText", "e0");
        rec.is_secret_payload = true;
        // Even if a caller mistakenly put a secret into a descriptor, the ledger
        // structurally has no payload field; confirm nothing carries the value.
        ledger.append(rec);

        let entry = &ledger.entries()[0];
        assert!(entry.is_secret_payload);
        let serialized = serde_json::to_string(&entry.summary_json()).unwrap();
        assert!(
            !serialized.contains(SECRET_MARKER),
            "ledger entry must never contain a secret value: {serialized}"
        );
    }

    #[test]
    fn ledger_is_inspectable_and_carries_no_raw_prompt_or_coordinates() {
        let mut ledger = GuiActionLedger::new();
        ledger.append(record("OpenApp", "e0"));
        ledger.append(record("ClickControl", "e1"));

        let summary = ledger.summary_json();
        assert_eq!(summary["entry_count"], 2);
        assert_eq!(summary["entries"].as_array().unwrap().len(), 2);
        assert_eq!(summary["head_hash"], ledger.head_hash());

        // Only the prompt_hash is recorded — never the raw prompt; and there are
        // no coordinate fields anywhere in the serialized ledger.
        let serialized = serde_json::to_string(&summary).unwrap();
        assert!(!serialized.contains(RAW_PROMPT_MARKER));
        for coord_key in ["\"x\"", "\"y\"", "\"width\"", "\"height\"", "\"bounds\""] {
            assert!(
                !serialized.contains(coord_key),
                "ledger must not carry coordinates ({coord_key}): {serialized}"
            );
        }
        assert!(serialized.contains("prompt-hash-abc"));
    }

    #[test]
    fn entry_recorded_event_surfaces_the_entry() {
        let mut ledger = GuiActionLedger::new();
        let entry = ledger.append(record("OpenApp", "e0")).clone();
        let event = ledger.entry_recorded_event(&entry);
        assert_eq!(event["type"], "GuiActionLedgerEntryRecorded");
        assert_eq!(event["entry_count"], 1);
        assert_eq!(event["entry"]["execution_id"], "e0");
        assert_eq!(event["head_hash"], ledger.head_hash());
    }
}
