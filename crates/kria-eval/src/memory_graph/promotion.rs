//! Predecessor / gate-promotion logic for the Memory Graph Production Redesign
//! spec (task F0.4 / 0.4.5).
//!
//! `tasks.md` **Execution Contract** requires gates to execute strictly
//! `F0 → F1 → F2 → F3 → F4 → F5`, with `F6` optional and startable "only from a
//! signed F5 manifest", and mandates that "no later-gate polish may mask an
//! earlier P0 failure". `validation.md` §6 records the predecessor requirement
//! for `F6` ("V-3D-01 and predecessor F5 manifest hash") and §7 forbids a Pass
//! that "points only to a checklist".
//!
//! This module renders that contract as the [`EvidenceManifest::can_promote`]
//! evaluator: given a manifest and the chain of predecessor manifests, it
//! computes — **deterministically** — whether the manifest's gate may be
//! promoted to [`RunStatus::Pass`], or is [`GatePromotion::Blocked`] with
//! structured [`ManifestDiagnostic`] reasons.
//!
//! ## Critical invariants (task 0.4.5)
//!
//! 1. **Changes only generated evidence status.** Promotion is a *pure*
//!    computation over manifests. It returns a [`GatePromotion`] value; it never
//!    writes to `tasks.md`, never mutates spec checkboxes, and never touches any
//!    file. The only "status" it produces is the generated evidence status
//!    carried by [`GatePromotion::Promoted`].
//!
//! 2. **Refuses to derive status from `tasks.md` boxes.** [`can_promote`] takes
//!    *no* `tasks.md` parameter and reads *no* checkbox state. Promotion derives
//!    solely from (a) this manifest's own [`EvidenceManifest::validate`] +
//!    [`EvidenceManifest::verify_artifacts`] + [`EvidenceManifest::enforce_governance`]
//!    being clean and (b) the signed predecessor-manifest chain. A checked box
//!    carries no executed command, no checksummed artifact, and no reviewer
//!    sign-off, so a checklist-only claim is rejected with
//!    [`ManifestDiagnosticKind::ChecklistOnlyPromotion`].
//!
//! [`can_promote`]: EvidenceManifest::can_promote

use std::collections::BTreeSet;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::fixtures::hex_lower;
use super::manifest::{
    EvidenceManifest, Gate, ManifestDiagnostic, ManifestDiagnosticKind, RunStatus,
};

/// Backend-first gate order, ascending (`F0` < `F1` < … < `F6`). Mirrors the
/// `Ord` derived on [`Gate`]; kept explicit so the required-chain walk is
/// self-documenting.
const GATE_ORDER: [Gate; 7] = [
    Gate::F0,
    Gate::F1,
    Gate::F2,
    Gate::F3,
    Gate::F4,
    Gate::F5,
    Gate::F6,
];

impl Gate {
    /// The immediate predecessor gate, or `None` for `F0` (which has no
    /// predecessor). `F6`'s predecessor is `F5` (`F6` may start "only from a
    /// signed F5 manifest").
    pub fn predecessor(self) -> Option<Gate> {
        match self {
            Gate::F0 => None,
            Gate::F1 => Some(Gate::F0),
            Gate::F2 => Some(Gate::F1),
            Gate::F3 => Some(Gate::F2),
            Gate::F4 => Some(Gate::F3),
            Gate::F5 => Some(Gate::F4),
            Gate::F6 => Some(Gate::F5),
        }
    }

    /// Every gate that must precede this one, ascending (`F0..=predecessor`).
    /// Empty for `F0`. Used to detect chain gaps: each gate in this list must be
    /// present as a valid signed `Pass` in the supplied predecessor chain.
    pub fn required_chain(self) -> Vec<Gate> {
        match self.predecessor() {
            None => Vec::new(),
            Some(pred) => GATE_ORDER.iter().copied().filter(|g| *g <= pred).collect(),
        }
    }
}

/// The deterministic outcome of a gate-promotion evaluation.
///
/// This value *is* the generated evidence status change: [`Promoted`] carries
/// the promoted [`RunStatus::Pass`]; [`Blocked`] carries the structured reasons.
/// Producing this value is the only side effect of promotion — no file, and in
/// particular no `tasks.md` checkbox, is ever written.
///
/// [`Promoted`]: GatePromotion::Promoted
/// [`Blocked`]: GatePromotion::Blocked
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatePromotion {
    /// The gate may be promoted; `status` is the generated evidence status
    /// ([`RunStatus::Pass`]).
    Promoted {
        /// The gate that was promoted.
        gate: Gate,
        /// The generated evidence status (always [`RunStatus::Pass`]).
        status: RunStatus,
    },
    /// The gate may not be promoted; `reasons` are the deterministic,
    /// sorted diagnostics explaining why.
    Blocked {
        /// The gate that could not be promoted.
        gate: Gate,
        /// Sorted, de-duplicated blocking diagnostics.
        reasons: Vec<ManifestDiagnostic>,
    },
}

impl GatePromotion {
    /// Whether the gate was promoted.
    pub fn is_promoted(&self) -> bool {
        matches!(self, GatePromotion::Promoted { .. })
    }

    /// The gate this outcome refers to.
    pub fn gate(&self) -> Gate {
        match self {
            GatePromotion::Promoted { gate, .. } | GatePromotion::Blocked { gate, .. } => *gate,
        }
    }

    /// The blocking reasons (empty when promoted).
    pub fn reasons(&self) -> &[ManifestDiagnostic] {
        match self {
            GatePromotion::Blocked { reasons, .. } => reasons,
            GatePromotion::Promoted { .. } => &[],
        }
    }

    /// Whether a blocking diagnostic of the given kind is present.
    pub fn has_kind(&self, kind: ManifestDiagnosticKind) -> bool {
        self.reasons().iter().any(|d| d.kind == kind)
    }
}

/// Whether `m` is a valid, signed `Pass` manifest that can license a successor
/// gate: it claims [`RunStatus::Pass`], is schema-valid, and carries a clean,
/// complete reviewer sign-off ([`EvidenceManifest::enforce_governance`] clean —
/// which for a `Pass` requires every mandatory reviewer role). Artifact on-disk
/// verification is each predecessor gate's own concern and is not re-run here.
fn is_valid_signed_pass(m: &EvidenceManifest) -> bool {
    m.status == RunStatus::Pass && m.validate().ok && m.enforce_governance().ok
}

impl EvidenceManifest {
    /// Compute this manifest's canonical content hash: the lowercase 64-hex
    /// SHA-256 of its compact JSON serialization. Serialization is field-ordered
    /// and map-sorted, so the hash is byte-stable and is the value a successor
    /// manifest records in its `predecessorHashes`.
    pub fn manifest_hash(&self) -> String {
        let json = self.to_json().unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        hex_lower(&hasher.finalize())
    }

    /// Evaluate whether this manifest's gate may be promoted to
    /// [`RunStatus::Pass`], given the chain of `predecessors` and, optionally, an
    /// `evidence_root` under which to re-verify declared artifacts on disk.
    ///
    /// The result is computed **deterministically** from manifest evidence only:
    ///
    /// * this manifest's own [`validate`](EvidenceManifest::validate),
    ///   [`verify_artifacts`](EvidenceManifest::verify_artifacts) (only when
    ///   `evidence_root` is supplied), and
    ///   [`enforce_governance`](EvidenceManifest::enforce_governance) must all be
    ///   clean;
    /// * the manifest must carry real machine evidence (at least one executed
    ///   command *and* at least one checksummed artifact) — a checklist/checkbox
    ///   claim is refused;
    /// * the `F0→Fn-1` predecessor chain must be contiguous, every predecessor
    ///   gate must have a valid signed `Pass` manifest, no predecessor may be
    ///   `Fail`/`Blocked`, and the immediate predecessor's manifest hash must be
    ///   recorded in this manifest's `predecessorHashes`.
    ///
    /// This function takes **no** `tasks.md` input and reads **no** checkbox
    /// state; a checked spec box can never promote a gate. It has no side
    /// effects — it neither writes `tasks.md` nor mutates any file; the returned
    /// [`GatePromotion`] is the sole (generated) status output.
    pub fn can_promote(
        &self,
        predecessors: &[EvidenceManifest],
        evidence_root: Option<&Path>,
    ) -> GatePromotion {
        let mut reasons: Vec<ManifestDiagnostic> = Vec::new();

        // (1) The manifest must be self-consistent: schema-valid, artifacts
        //     verified on disk (when a root is supplied), and governance clean.
        reasons.extend(self.validate().diagnostics);
        if let Some(root) = evidence_root {
            reasons.extend(self.verify_artifacts(root).diagnostics);
        }
        reasons.extend(self.enforce_governance().diagnostics);

        // (2) Refuse checklist-only promotion. A checked `tasks.md` box carries
        //     no executed command and no checksummed artifact; promotion derives
        //     only from machine evidence, never from a checkbox.
        if self.commands.is_empty() || self.artifacts.is_empty() {
            reasons.push(ManifestDiagnostic::new(
                ManifestDiagnosticKind::ChecklistOnlyPromotion,
                "artifacts",
                "gate promotion requires at least one executed command and one \
                 checksummed artifact; a checklist/checkbox claim is not evidence",
            ));
        }

        // (3) Verify the predecessor chain (F0 has none).
        if let Some(pred_gate) = self.gate.predecessor() {
            self.check_predecessor_chain(pred_gate, predecessors, &mut reasons);
        }

        // Deterministic, order-independent output.
        reasons.sort_by(|a, b| {
            (a.kind.code(), a.field.as_str(), a.reason.as_str()).cmp(&(
                b.kind.code(),
                b.field.as_str(),
                b.reason.as_str(),
            ))
        });
        reasons.dedup();

        if reasons.is_empty() {
            GatePromotion::Promoted {
                gate: self.gate,
                status: RunStatus::Pass,
            }
        } else {
            GatePromotion::Blocked {
                gate: self.gate,
                reasons,
            }
        }
    }

    /// Walk the `F0..=pred_gate` chain, appending diagnostics for every defect:
    /// missing links, chain gaps, non-passing predecessors, unresolved earlier
    /// failures, and an unrecorded immediate-predecessor hash.
    fn check_predecessor_chain(
        &self,
        pred_gate: Gate,
        predecessors: &[EvidenceManifest],
        reasons: &mut Vec<ManifestDiagnostic>,
    ) {
        // Recorded predecessor hashes, normalized to lowercase.
        let recorded: BTreeSet<String> = self
            .predecessor_hashes
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .collect();

        for gate in self.gate.required_chain() {
            let matching: Vec<&EvidenceManifest> =
                predecessors.iter().filter(|m| m.gate == gate).collect();

            // A required gate entirely absent from the chain: the immediate
            // predecessor is `PredecessorMissing`; an intermediate hole is a
            // `GateChainGap`.
            if matching.is_empty() {
                if gate == pred_gate {
                    reasons.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::PredecessorMissing,
                        "predecessorHashes",
                        format!(
                            "immediate predecessor gate {gate:?} has no manifest in the \
                             supplied chain; gate {:?} cannot be promoted",
                            self.gate
                        ),
                    ));
                } else {
                    reasons.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::GateChainGap,
                        "predecessorHashes",
                        format!(
                            "required intermediate gate {gate:?} is absent from the \
                             predecessor chain of gate {:?}",
                            self.gate
                        ),
                    ));
                }
                continue;
            }

            // No later-gate polish may mask an earlier P0 failure.
            if matching
                .iter()
                .any(|m| matches!(m.status, RunStatus::Fail | RunStatus::Blocked))
            {
                reasons.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::EarlierGateP0Unresolved,
                    "predecessorHashes",
                    format!(
                        "predecessor gate {gate:?} carries a Fail/Blocked manifest; a later \
                         gate cannot be promoted over an unresolved earlier failure"
                    ),
                ));
            }

            // At least one valid signed Pass must license the successor.
            let valid_passes: Vec<&EvidenceManifest> = matching
                .iter()
                .copied()
                .filter(|m| is_valid_signed_pass(m))
                .collect();
            if valid_passes.is_empty() {
                reasons.push(ManifestDiagnostic::new(
                    ManifestDiagnosticKind::PredecessorNotPassed,
                    "predecessorHashes",
                    format!(
                        "predecessor gate {gate:?} has no valid, signed Pass manifest to \
                         license promotion"
                    ),
                ));
                continue;
            }

            // The immediate predecessor's hash must be recorded here.
            if gate == pred_gate {
                let hash_recorded = valid_passes
                    .iter()
                    .any(|m| recorded.contains(&m.manifest_hash()));
                if !hash_recorded {
                    reasons.push(ManifestDiagnostic::new(
                        ManifestDiagnosticKind::PredecessorHashMismatch,
                        "predecessorHashes",
                        format!(
                            "no recorded predecessor hash matches a valid signed Pass \
                             {gate:?} manifest"
                        ),
                    ));
                }
            }
        }
    }
}
