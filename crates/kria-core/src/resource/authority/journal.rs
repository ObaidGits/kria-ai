//! Decision Journal (HRA Task 8 / R10.2, R21.2).
//!
//! Append-only, checksummed, versioned decision log. Backs crash recovery (Reconciler) and
//! diagnostics correlation. Records carry a monotonic `seq`, a `turn_id` correlation id, and a
//! checksum; recovery truncates at the first bad record (last-good wins) and tolerates unknown
//! future fields, refusing only on incompatible major version.
//!
//! This in-memory journal is the logical model; the runtime persists/loads the same records.

use serde::{Deserialize, Serialize};

use super::types::{DeviceId, Epoch, RationaleCode, TurnId};

/// Current journal record schema major version. Bump only on incompatible change.
pub const JOURNAL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    EpochBump { to: u64 },
    LeaseGranted { token: u64, device: DeviceId, vram_mb: u64 },
    LeaseReleased { token: u64 },
    Planned { device: DeviceId, rationale: RationaleCode },
    Preempted { victim_token: u64, reason: String },
    Evicted { model: String },
    Failover { pool: String },
    SimulateReject { rationale: RationaleCode },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub seq: u64,
    pub turn_id: TurnId,
    pub kind: DecisionKind,
    pub at_ms: u64,
}

/// A durable, integrity-checked record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub ver: u16,
    pub payload: Decision,
    pub checksum: u32,
}

impl JournalRecord {
    fn new(payload: Decision) -> Self {
        let checksum = checksum_of(&payload);
        Self {
            ver: JOURNAL_VERSION,
            payload,
            checksum,
        }
    }

    /// Validate version compatibility + checksum integrity.
    pub fn is_valid(&self) -> bool {
        self.ver <= JOURNAL_VERSION && self.checksum == checksum_of(&self.payload)
    }
}

/// FNV-1a 32-bit over the canonical JSON of the payload. Deterministic + dependency-free.
fn checksum_of(payload: &Decision) -> u32 {
    let bytes = serde_json::to_vec(payload).unwrap_or_default();
    let mut hash: u32 = 0x811c_9dc5;
    for b in bytes {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[derive(Debug, Clone, Default)]
pub struct Journal {
    records: Vec<JournalRecord>,
    next_seq: u64,
    epoch: u64,
}

impl Journal {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            next_seq: 1,
            epoch: 0,
        }
    }

    pub fn current_epoch(&self) -> Epoch {
        Epoch(self.epoch)
    }

    /// Increment the authority epoch (called on RA restart) and journal it.
    pub fn bump_epoch(&mut self, turn: TurnId, at_ms: u64) -> Epoch {
        self.epoch += 1;
        self.append(turn, DecisionKind::EpochBump { to: self.epoch }, at_ms);
        Epoch(self.epoch)
    }

    /// Append a decision; returns its seq.
    pub fn append(&mut self, turn_id: TurnId, kind: DecisionKind, at_ms: u64) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let rec = JournalRecord::new(Decision {
            seq,
            turn_id,
            kind,
            at_ms,
        });
        self.records.push(rec);
        seq
    }

    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }

    /// Reconstruct a journal from persisted records, truncating at the first invalid record
    /// (last-good wins) per R21.2. Returns (journal, truncated_count).
    pub fn replay(records: Vec<JournalRecord>) -> (Self, usize) {
        let mut good = Vec::new();
        let mut truncated = 0;
        for (i, rec) in records.iter().enumerate() {
            if rec.is_valid() {
                good.push(rec.clone());
            } else {
                truncated = records.len() - i;
                break;
            }
        }
        let next_seq = good.last().map(|r| r.payload.seq + 1).unwrap_or(1);
        let epoch = good
            .iter()
            .rev()
            .find_map(|r| match &r.payload.kind {
                DecisionKind::EpochBump { to } => Some(*to),
                _ => None,
            })
            .unwrap_or(0);
        (
            Self {
                records: good,
                next_seq,
                epoch,
            },
            truncated,
        )
    }

    /// Serialize the journal records for on-disk persistence (Task 27). The runtime writes these
    /// bytes (with fsync policy); `from_bytes` + `replay` recover them.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.records).unwrap_or_default()
    }

    /// Load + recover from persisted bytes. Corrupt/partial tails are truncated (R21.2). On a fully
    /// unreadable buffer, returns an empty journal (safe cold reconcile from live device state).
    pub fn from_bytes(bytes: &[u8]) -> (Self, usize) {
        match serde_json::from_slice::<Vec<JournalRecord>>(bytes) {
            Ok(records) => Self::replay(records),
            Err(_) => (Self::new(), 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> TurnId {
        TurnId("turn".into())
    }

    #[test]
    fn append_and_replay_round_trip() {
        let mut j = Journal::new();
        j.append(t(), DecisionKind::Evicted { model: "m".into() }, 1);
        j.append(t(), DecisionKind::LeaseReleased { token: 5 }, 2);
        let (j2, truncated) = Journal::replay(j.records().to_vec());
        assert_eq!(truncated, 0);
        assert_eq!(j2.records().len(), 2);
    }

    #[test]
    fn epoch_is_monotonic_and_recovered() {
        let mut j = Journal::new();
        j.bump_epoch(t(), 1);
        j.bump_epoch(t(), 2);
        assert_eq!(j.current_epoch(), Epoch(2));
        let (j2, _) = Journal::replay(j.records().to_vec());
        assert_eq!(j2.current_epoch(), Epoch(2));
    }

    #[test]
    fn corrupted_record_truncates_at_first_bad() {
        let mut j = Journal::new();
        j.append(t(), DecisionKind::Evicted { model: "a".into() }, 1);
        j.append(t(), DecisionKind::Evicted { model: "b".into() }, 2);
        j.append(t(), DecisionKind::Evicted { model: "c".into() }, 3);
        let mut recs = j.records().to_vec();
        // Corrupt the 2nd record's checksum.
        recs[1].checksum ^= 0xDEAD_BEEF;
        let (j2, truncated) = Journal::replay(recs);
        assert_eq!(j2.records().len(), 1); // only the first good record survives
        assert_eq!(truncated, 2);
    }

    #[test]
    fn future_major_version_is_rejected() {
        let mut j = Journal::new();
        j.append(t(), DecisionKind::LeaseReleased { token: 1 }, 1);
        let mut recs = j.records().to_vec();
        recs[0].ver = JOURNAL_VERSION + 1;
        assert!(!recs[0].is_valid());
    }

    #[test]
    fn persist_to_bytes_and_recover() {
        let mut j = Journal::new();
        j.bump_epoch(t(), 1);
        j.append(t(), DecisionKind::Evicted { model: "m".into() }, 2);
        let bytes = j.to_bytes();
        let (j2, truncated) = Journal::from_bytes(&bytes);
        assert_eq!(truncated, 0);
        assert_eq!(j2.current_epoch(), Epoch(1));
        assert_eq!(j2.records().len(), j.records().len());
    }

    #[test]
    fn unreadable_bytes_recover_to_empty() {
        let (j, truncated) = Journal::from_bytes(b"not-json");
        assert_eq!(j.records().len(), 0);
        assert_eq!(truncated, 0);
    }
}
