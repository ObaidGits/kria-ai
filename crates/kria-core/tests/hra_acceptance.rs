//! HRA acceptance suite (headless) — closes the *logic* of Tasks 21 (multi-GPU), 22 (fail-open),
//! 41 (chaos/soak invariants), and 23 (Production Readiness matrix) against the in-memory Resource
//! Authority. Hardware-specific GPU validation still requires real multi-GPU silicon; these tests
//! prove the deterministic control-plane behavior that does not need a physical GPU.

use kria_core::resource::authority::{
    ConsumerId, Constraints, DeviceId, HraService, LocalAuthority, PolicyProfile, PriorityClass,
    PrivacyReq, RaOutcome, ResourceAuthority, ResourceNeed, ResourceRequest, TurnId,
};

fn req(consumer: ConsumerId, vram: u64, privacy: PrivacyReq, allow_cloud: bool, turn: &str) -> ResourceRequest {
    ResourceRequest {
        consumer,
        class: PriorityClass::InteractiveFg,
        need: ResourceNeed {
            vram_mb: vram,
            ram_mb: 2048,
            cpu_threads: 4,
            exclusivity: false,
            model_id: Some("m".into()),
            est_ms: 1000,
        },
        constraints: Constraints { privacy, allow_cloud, ..Default::default() },
        turn_id: TurnId(turn.into()),
    }
}

// ── Task 21: multi-GPU placement ─────────────────────────────────────

#[test]
fn task21_two_consumers_land_on_two_gpus() {
    // Two 12 GB GPUs; two 8 GB requests cannot co-reside on one device → second lands on GPU1.
    let ra = LocalAuthority::bootstrap(&[(0, 12288), (1, 12288)], 512, 32768, &[], PolicyProfile::Balanced);

    let l0 = match ra.request(&req(ConsumerId::Llm, 8000, PrivacyReq::Standard, true, "a")) {
        RaOutcome::Granted(l) => l,
        o => panic!("expected grant, got {o:?}"),
    };
    let l1 = match ra.request(&req(ConsumerId::Image, 8000, PrivacyReq::Standard, true, "b")) {
        RaOutcome::Granted(l) => l,
        o => panic!("expected grant, got {o:?}"),
    };
    assert_ne!(l0.device, l1.device, "two big consumers must occupy two distinct GPUs");
    assert!(matches!(l0.device, DeviceId::Gpu(_)));
    assert!(matches!(l1.device, DeviceId::Gpu(_)));
}

#[test]
fn task21_no_overcommit_on_single_gpu() {
    let ra = LocalAuthority::bootstrap(&[(0, 12288)], 512, 32768, &["openai"], PolicyProfile::Balanced);
    // First 8 GB grant on GPU0.
    let _l0 = ra.request(&req(ConsumerId::Llm, 8000, PrivacyReq::Standard, true, "a"));
    // Second 8 GB cannot fit GPU0 → must fall back (cloud/CPU), never over-commit GPU0.
    match ra.request(&req(ConsumerId::Image, 8000, PrivacyReq::Standard, true, "b")) {
        RaOutcome::Granted(l) => assert!(matches!(l.device, DeviceId::CloudPool(_) | DeviceId::Cpu)),
        RaOutcome::Busy => {} // also acceptable (no fallback fit) — never an over-commit
        o => panic!("unexpected {o:?}"),
    }
}

// ── Task 22: fail-open ───────────────────────────────────────────────

#[test]
fn task22_fail_open_to_cpu_when_no_gpu_and_no_cloud() {
    // Tiny GPU that cannot admit; cloud disallowed → must fail open to CPU, never hang/error.
    let ra = LocalAuthority::bootstrap(&[(0, 2048)], 512, 16384, &[], PolicyProfile::Balanced);
    match ra.request(&req(ConsumerId::Llm, 8000, PrivacyReq::Standard, false, "a")) {
        RaOutcome::Granted(l) => assert_eq!(l.device, DeviceId::Cpu),
        o => panic!("expected CPU fail-open, got {o:?}"),
    }
}

#[test]
fn task22_privacy_strict_fails_to_cpu_never_cloud() {
    let ra = LocalAuthority::bootstrap(&[(0, 2048)], 512, 16384, &["openai"], PolicyProfile::Balanced);
    match ra.request(&req(ConsumerId::Llm, 8000, PrivacyReq::Strict, true, "a")) {
        RaOutcome::Granted(l) => assert_eq!(l.device, DeviceId::Cpu),
        o => panic!("expected CPU for privacy-strict, got {o:?}"),
    }
}

// ── Task 41: chaos / soak invariants ─────────────────────────────────

/// Tiny deterministic LCG so the soak is reproducible (no rand dependency).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 16
    }
}

#[test]
fn task41_chaos_soak_holds_invariants() {
    let svc = HraService::new(&[(0, 12288), (1, 8192)], 512, 32768, &["openai"], PolicyProfile::Balanced);
    let mut rng = Lcg(0xC0FFEE);
    let consumers = [ConsumerId::Llm, ConsumerId::Image, ConsumerId::Vision, ConsumerId::Embed];
    let mut held: Vec<kria_core::resource::authority::LeaseToken> = Vec::new();

    for i in 0..2000 {
        let r = rng.next();
        if (r & 1) == 0 || held.is_empty() {
            // issue a request
            let c = consumers[(r >> 1) as usize % consumers.len()];
            let vram = 1000 + ((r >> 3) % 6000);
            let privacy = if (r >> 5) & 7 == 0 { PrivacyReq::Strict } else { PrivacyReq::Standard };
            let turn = format!("t{i}");
            match svc.request(&req(c, vram, privacy, true, &turn)) {
                RaOutcome::Granted(l) => held.push(l.token),
                RaOutcome::Busy | RaOutcome::Shed | RaOutcome::PreemptThenRetry { .. } => {}
            }
        } else {
            // release a random held lease
            let idx = (r >> 2) as usize % held.len();
            let tok = held.swap_remove(idx);
            svc.release(tok);
        }
    }
    // Release the remainder.
    for tok in held.drain(..) {
        svc.release(tok);
    }

    // Invariants after the soak: shadow comparator never flagged a violation (no over-commit,
    // no privacy-strict cloud), and the foreground-safety invariant held.
    assert!(svc.shadow_gate_passes(), "shadow comparator must remain clean over the soak");
    assert!(svc.metrics().foreground_invariant_ok(), "no non-emergency foreground interrupts");
}

// ── Task 23: Production Readiness acceptance matrix ───────────────────

#[test]
fn task23_prr_matrix() {
    let ra = LocalAuthority::bootstrap(&[(0, 12288)], 512, 32768, &["openai"], PolicyProfile::Balanced);

    // A1-ish: single authority grants; epoch starts at 1 (fresh authority fences prior leases).
    assert_eq!(ra.current_epoch().0, 1);

    // A10/R13: a normal request is granted deterministically.
    let g1 = ra.request(&req(ConsumerId::Llm, 4000, PrivacyReq::Standard, true, "x"));
    let g2 = ra.request(&req(ConsumerId::Llm, 4000, PrivacyReq::Standard, true, "x"));
    assert!(matches!(g1, RaOutcome::Granted(_)));
    assert!(matches!(g2, RaOutcome::Granted(_)));

    // A19: bypass kill-switch yields a static-plan grant with no authority gating.
    ra.set_bypass(ConsumerId::Image, true);
    assert!(ra.is_bypassed(ConsumerId::Image));
    match ra.request(&req(ConsumerId::Image, 3000, PrivacyReq::Standard, true, "y")) {
        RaOutcome::Granted(l) => assert!(matches!(l.device, DeviceId::Gpu(_) | DeviceId::Cpu)),
        o => panic!("bypass should grant, got {o:?}"),
    }
}

// ── Phase B: Co-Residency GPU Lease Manager (end-to-end through HraService) ───

use kria_core::resource::authority::ResidencyTarget;

fn cor_req(consumer: ConsumerId, class: PriorityClass, vram: u64, model: &str) -> ResourceRequest {
    ResourceRequest {
        consumer,
        class,
        need: ResourceNeed {
            vram_mb: vram,
            ram_mb: 2048,
            cpu_threads: 4,
            exclusivity: false,
            model_id: Some(model.into()),
            est_ms: 1000,
        },
        constraints: Constraints::default(),
        turn_id: TurnId(format!("cor-{model}")),
    }
}

#[tokio::test]
async fn phaseb_llm_and_image_co_reside_then_fg_preempts() {
    // 12 GB GPU: LLM (fg, 4 GB) + Image (batch, 4 GB) co-reside — the whole point of Phase B.
    let hra = HraService::new(&[(0, 12288)], 512, 32768, &[], PolicyProfile::Balanced);
    let cor = hra.co_residency();

    let llm = cor
        .acquire(&cor_req(ConsumerId::Llm, PriorityClass::InteractiveFg, 4000, "llm"), ResidencyTarget::Hot)
        .await
        .expect("llm co-resident grant");
    let img = cor
        .acquire(&cor_req(ConsumerId::Image, PriorityClass::Batch, 4000, "img"), ResidencyTarget::Hot)
        .await
        .expect("image co-resident grant");
    assert!(llm.is_valid() && img.is_valid(), "both co-resident on one GPU");
    assert_eq!(cor.resident_count().await, 2);

    llm.release().await;
    img.release().await;
    assert_eq!(cor.resident_count().await, 0, "reservations drain — no leak");
}

#[tokio::test]
async fn phaseb_foreground_preempts_background_on_full_gpu() {
    // 8 GB GPU: background image 6 GB resident; foreground LLM 6 GB must preempt it (fg protected).
    let hra = HraService::new(&[(0, 8192)], 512, 32768, &[], PolicyProfile::Balanced);
    let cor = hra.co_residency();

    let img = cor
        .acquire(&cor_req(ConsumerId::Image, PriorityClass::Batch, 6000, "img"), ResidencyTarget::Hot)
        .await
        .expect("image grant");
    let llm = cor
        .acquire(&cor_req(ConsumerId::Llm, PriorityClass::InteractiveFg, 6000, "llm"), ResidencyTarget::Hot)
        .await
        .expect("fg preempts bg and grants");
    assert!(llm.is_valid());
    assert!(!img.is_valid(), "preempted background lease is cooperatively revoked");
    assert!(hra.co_residency_metrics().preemptions >= 1);
}

#[tokio::test]
async fn phaseb_background_cannot_preempt_foreground() {
    // 8 GB GPU: foreground LLM 6 GB resident; background image 6 GB cannot evict it → Busy.
    let hra = HraService::new(&[(0, 8192)], 512, 32768, &[], PolicyProfile::Balanced);
    let cor = hra.co_residency();

    let llm = cor
        .acquire(&cor_req(ConsumerId::Llm, PriorityClass::InteractiveFg, 6000, "llm"), ResidencyTarget::Hot)
        .await
        .expect("llm grant");
    let outcome = cor
        .acquire(&cor_req(ConsumerId::Image, PriorityClass::Batch, 6000, "img"), ResidencyTarget::Hot)
        .await;
    assert!(outcome.is_err(), "background must not preempt foreground");
    assert!(llm.is_valid(), "foreground residency untouched");
}
