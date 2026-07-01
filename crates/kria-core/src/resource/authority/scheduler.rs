//! Scheduler (HRA Task 6 / R6, R21.1, R21.3).
//!
//! Admission, leasing, priority, preemption, and bounded load-shedding over the `DeviceTable`.
//! Deterministic and synchronous at the core (the runtime wraps it with async `Notify` waiting).
//!
//! Guarantees enforced here:
//! - No over-commit: admission reserves through `DeviceTable` (Property 1).
//! - Epoch fencing: every lease carries the authority epoch (Property 11 / R21.1).
//! - Bounded queues: per-class depth caps with lowest-class-first shedding (Property 14 / R21.3).
//! - Preemption: a higher class may preempt a lower-class holder on a contended device (R6.3),
//!   reported as a victim token for the runtime to checkpoint+reclaim.

use super::device_table::DeviceTable;
use super::types::{Capacity, DeviceId, Epoch, Plan, PriorityClass, Residency, TurnId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LeaseToken(pub u64);

/// A granted lease. Carries the epoch for split-brain fencing (R21.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub token: LeaseToken,
    pub device: DeviceId,
    pub budget: Capacity,
    pub class: PriorityClass,
    pub residency: Residency,
    pub epoch: Epoch,
    pub turn_id: TurnId,
    pub speculative: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmitError {
    /// Device contended by an equal-or-higher class; caller should wait or fall back.
    Busy { holder: PriorityClass },
    /// A higher class request requires preempting this victim first.
    PreemptionRequired { victim: LeaseToken },
    /// Request shed due to overload (low priority + full queue).
    Shed,
    /// No device in the plan (and its fallbacks) could be admitted.
    NoCapacity,
}

#[derive(Debug, Clone)]
struct ActiveLease {
    lease: Lease,
    vram_mb: u64,
}

/// Per-class queue depth caps for load-shedding (R21.3).
#[derive(Debug, Clone, Copy)]
pub struct QueueCaps {
    pub maintenance: usize,
    pub batch: usize,
    pub interactive_bg: usize,
    pub realtime_voice: usize,
    pub interactive_fg: usize,
}

impl Default for QueueCaps {
    fn default() -> Self {
        Self {
            maintenance: 2,
            batch: 8,
            interactive_bg: 16,
            realtime_voice: 32,
            interactive_fg: 32,
        }
    }
}

pub struct Scheduler {
    next_token: u64,
    epoch: Epoch,
    active: Vec<ActiveLease>,
    queue_depth: std::collections::HashMap<PriorityClass, usize>,
    caps: QueueCaps,
}

impl Scheduler {
    pub fn new(epoch: Epoch) -> Self {
        Self {
            next_token: 1,
            epoch,
            active: Vec::new(),
            queue_depth: std::collections::HashMap::new(),
            caps: QueueCaps::default(),
        }
    }

    pub fn with_caps(epoch: Epoch, caps: QueueCaps) -> Self {
        let mut s = Self::new(epoch);
        s.caps = caps;
        s
    }

    pub fn set_epoch(&mut self, epoch: Epoch) {
        self.epoch = epoch;
    }

    fn cap_for(&self, class: PriorityClass) -> usize {
        match class {
            PriorityClass::Maintenance => self.caps.maintenance,
            PriorityClass::Batch => self.caps.batch,
            PriorityClass::InteractiveBg => self.caps.interactive_bg,
            PriorityClass::RealtimeVoice => self.caps.realtime_voice,
            PriorityClass::InteractiveFg => self.caps.interactive_fg,
        }
    }

    /// Enqueue a request for admission; returns Shed if the class queue is full (R21.3).
    pub fn enqueue(&mut self, class: PriorityClass) -> Result<(), AdmitError> {
        let cap = self.cap_for(class);
        let depth = self.queue_depth.entry(class).or_insert(0);
        if *depth >= cap {
            return Err(AdmitError::Shed);
        }
        *depth += 1;
        Ok(())
    }

    pub fn dequeue(&mut self, class: PriorityClass) {
        if let Some(d) = self.queue_depth.get_mut(&class) {
            *d = d.saturating_sub(1);
        }
    }

    /// Attempt to admit `req_class` onto the device in `plan`, walking `fallback_chain` if the
    /// primary device cannot be reserved. On a GPU device held by a strictly-lower class, returns
    /// `PreemptionRequired` so the runtime can checkpoint+reclaim the victim first.
    pub fn admit(
        &mut self,
        table: &mut DeviceTable,
        class: PriorityClass,
        turn_id: TurnId,
        plan: &Plan,
    ) -> Result<Lease, AdmitError> {
        let mut last_err = AdmitError::NoCapacity;
        for candidate in std::iter::once(plan).chain(plan.fallback_chain.iter()) {
            match self.try_admit_one(table, class, turn_id.clone(), candidate) {
                Ok(lease) => return Ok(lease),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    fn try_admit_one(
        &mut self,
        table: &mut DeviceTable,
        class: PriorityClass,
        turn_id: TurnId,
        plan: &Plan,
    ) -> Result<Lease, AdmitError> {
        let vram = plan.budget.vram_mb;

        // Non-GPU devices (CPU/cloud) are not exclusively contended here.
        let is_gpu = matches!(plan.device, DeviceId::Gpu(_));

        if is_gpu {
            // Try a clean reservation first.
            if table.reserve_vram(&plan.device, vram) {
                return Ok(self.grant(plan, class, turn_id, vram));
            }
            // Contended: check the holder on this device.
            if let Some(victim) = self.lowest_holder_on(&plan.device) {
                if class > victim.lease.class {
                    return Err(AdmitError::PreemptionRequired {
                        victim: victim.lease.token,
                    });
                }
                return Err(AdmitError::Busy {
                    holder: victim.lease.class,
                });
            }
            // No holder but reservation failed → genuine capacity exhaustion.
            return Err(AdmitError::NoCapacity);
        }

        // CPU/cloud: always admissible (fail-open path).
        Ok(self.grant(plan, class, turn_id, 0))
    }

    fn grant(&mut self, plan: &Plan, class: PriorityClass, turn_id: TurnId, vram: u64) -> Lease {
        let token = LeaseToken(self.next_token);
        self.next_token += 1;
        let lease = Lease {
            token,
            device: plan.device.clone(),
            budget: plan.budget,
            class,
            residency: plan.residency,
            epoch: self.epoch,
            turn_id,
            speculative: false,
        };
        self.active.push(ActiveLease {
            lease: lease.clone(),
            vram_mb: vram,
        });
        lease
    }

    fn lowest_holder_on(&self, device: &DeviceId) -> Option<&ActiveLease> {
        self.active
            .iter()
            .filter(|a| &a.lease.device == device)
            .min_by_key(|a| a.lease.class)
    }

    /// Release a lease, returning its reserved VRAM to the table.
    pub fn release(&mut self, table: &mut DeviceTable, token: LeaseToken) {
        if let Some(pos) = self.active.iter().position(|a| a.lease.token == token) {
            let a = self.active.remove(pos);
            if a.vram_mb > 0 {
                table.release_vram(&a.lease.device, a.vram_mb);
            }
        }
    }

    /// Validate a lease epoch (consumers call before each GPU op — Property 11).
    pub fn lease_epoch_valid(&self, lease: &Lease) -> bool {
        lease.epoch == self.epoch
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }
}

#[cfg(test)]
mod tests {
    use super::super::device_table::{DeviceRecord, DeviceTable};
    use super::super::types::RationaleCode;
    use super::*;

    fn gpu_plan(vram_mb: u64) -> Plan {
        Plan {
            device: DeviceId::Gpu(0),
            residency: Residency::VramHot,
            budget: Capacity::vram(vram_mb),
            fallback_chain: vec![Plan {
                device: DeviceId::Cpu,
                residency: Residency::RamWarm,
                budget: Capacity::default(),
                fallback_chain: vec![],
                rationale: RationaleCode::FailOpenCpu,
            }],
            rationale: RationaleCode::FitsLocal,
        }
    }

    fn table() -> DeviceTable {
        let mut t = DeviceTable::new();
        t.upsert(DeviceRecord::gpu(0, 12288, 512));
        t.upsert(DeviceRecord::cpu(32768));
        t
    }

    #[test]
    fn admit_reserves_and_release_returns() {
        let mut t = table();
        let mut s = Scheduler::new(Epoch(1));
        let lease = s
            .admit(&mut t, PriorityClass::InteractiveFg, TurnId("a".into()), &gpu_plan(4000))
            .unwrap();
        assert_eq!(lease.device, DeviceId::Gpu(0));
        assert_eq!(lease.epoch, Epoch(1));
        assert_eq!(t.get(&DeviceId::Gpu(0)).unwrap().reserved_vram_mb, 4000);
        s.release(&mut t, lease.token);
        assert_eq!(t.get(&DeviceId::Gpu(0)).unwrap().reserved_vram_mb, 0);
    }

    #[test]
    fn higher_class_triggers_preemption_of_lower_holder() {
        let mut t = table();
        let mut s = Scheduler::new(Epoch(1));
        // Fill the GPU with a background lease so a second GPU reservation fails.
        let bg = s
            .admit(&mut t, PriorityClass::Batch, TurnId("bg".into()), &gpu_plan(11000))
            .unwrap();
        // Foreground wants GPU; reservation fails → preemption required against the batch victim.
        let err = s
            .admit(&mut t, PriorityClass::InteractiveFg, TurnId("fg".into()), &{
                // plan with no CPU fallback so we observe the GPU contention error directly
                Plan {
                    device: DeviceId::Gpu(0),
                    residency: Residency::VramHot,
                    budget: Capacity::vram(4000),
                    fallback_chain: vec![],
                    rationale: RationaleCode::FitsLocal,
                }
            })
            .unwrap_err();
        assert_eq!(err, AdmitError::PreemptionRequired { victim: bg.token });
    }

    #[test]
    fn equal_or_lower_class_gets_busy_not_preempt() {
        let mut t = table();
        let mut s = Scheduler::new(Epoch(1));
        let _fg = s
            .admit(&mut t, PriorityClass::InteractiveFg, TurnId("fg".into()), &gpu_plan(11000))
            .unwrap();
        let err = s
            .admit(&mut t, PriorityClass::Batch, TurnId("bg".into()), &Plan {
                device: DeviceId::Gpu(0),
                residency: Residency::VramHot,
                budget: Capacity::vram(4000),
                fallback_chain: vec![],
                rationale: RationaleCode::FitsLocal,
            })
            .unwrap_err();
        assert_eq!(err, AdmitError::Busy { holder: PriorityClass::InteractiveFg });
    }

    #[test]
    fn gpu_full_falls_back_to_cpu_via_chain() {
        let mut t = table();
        let mut s = Scheduler::new(Epoch(1));
        let _hog = s
            .admit(&mut t, PriorityClass::InteractiveFg, TurnId("h".into()), &gpu_plan(11000))
            .unwrap();
        // Equal-class request with a CPU fallback → lands on CPU instead of erroring.
        let lease = s
            .admit(&mut t, PriorityClass::InteractiveFg, TurnId("x".into()), &gpu_plan(4000))
            .unwrap();
        assert_eq!(lease.device, DeviceId::Cpu);
    }

    #[test]
    fn queue_caps_shed_low_priority() {
        let mut s = Scheduler::with_caps(
            Epoch(1),
            QueueCaps {
                maintenance: 1,
                ..QueueCaps::default()
            },
        );
        assert!(s.enqueue(PriorityClass::Maintenance).is_ok());
        assert_eq!(s.enqueue(PriorityClass::Maintenance), Err(AdmitError::Shed));
    }

    #[test]
    fn epoch_fencing_detects_stale_lease() {
        let mut t = table();
        let mut s = Scheduler::new(Epoch(1));
        let lease = s
            .admit(&mut t, PriorityClass::InteractiveFg, TurnId("a".into()), &gpu_plan(2000))
            .unwrap();
        assert!(s.lease_epoch_valid(&lease));
        s.set_epoch(Epoch(2)); // simulate RA restart
        assert!(!s.lease_epoch_valid(&lease));
    }
}
