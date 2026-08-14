# Linux OS Control — Planned Implementation Design

**Feature:** `linux-os-control-production`  
**Status:** Planned target for OSC-001–OSC-036; implementation not begun  
**Posture:** Single-user, single-process, local Ubuntu laptop; production-quality safety and correctness remain mandatory  
**Authority:** `requirements.md` is normative; this document defines the implementation architecture

## Overview

KRIA will expose normal desktop operating-system behavior through typed capability providers owned by `kria-core`. Tool handlers remain the model-callable facade, while provider ports own discovery, preflight, observation, application, verification evidence, and rollback tokens. Existing agent routing, `ExecutionGate`, durable interaction decisions, HITL streaming, cancellation, and safety audit infrastructure remain the only governed path.

The architecture deliberately separates:

- **Intent and policy:** what the user requested and whether KRIA may do it.
- **Domain state:** normalized, provider-independent observations and desired state.
- **Adapters:** D-Bus, freedesktop, desktop-specific, and structured-command mechanisms.
- **Evidence:** what was observed before and after the operation.
- **Presentation:** existing tool and WebSocket result envelopes.

No approved OS action may be implemented as an LLM-produced shell string. Native D-Bus and stable freedesktop APIs are preferred; controlled command adapters use fixed executables and argv arrays without shell parsing.

## Architecture

The authoritative architecture is the layered prompt → tool → `ExecutionGate` → OS-control runtime → domain provider → verification → audit flow defined in §§2–9 and §15. `kria-core` owns semantics; desktop/server adapters only inject live providers and present existing events.

## Components and Interfaces

The planned module tree, `HostOsControl` aggregate, domain ports, execution context, privilege broker, provider selection, and canonical tools are defined in §§3–12. Provider interfaces are typed, host-bound, injectable, and return normalized state rather than prose.

## Data Models

Provider-independent capability snapshots, observations, desired states, execution grants, receipts, postconditions, evidence, rollback metadata, errors, audit records, package plans, and durable projection data are defined in §§4–7 and §§13–16.

## Correctness Properties

Each property is Planned. Generative tests run at least 100 cases where a generator is meaningful and carry `Feature: linux-os-control-production, Property N`; deterministic examples and edge cases supplement properties. The numbered table is the complete property catalog and test-oracle source.

### Property 1: Governed host-action admission

**Validates: Requirements 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9**

Traceability requirement: OSC-001.

For every generated OS action, provider apply is unreachable unless readiness, strict validation, host authority, capability availability, policy, approval when required, deterministic resources, and durable audit admission all succeed for the same action, parameter, target, session, and capability-snapshot bindings. This heading anchors the property catalog in Kiro spec format; Properties 2–36 remain enumerated with their complete oracles below.

| P | Requirement | Property / required oracle |
|---:|---|---|
| 1 | OSC-001 | Apply is unreachable without a fresh ExecutionGate grant bound to action/parameters/host/session/risk/capability/resource digest and a runtime-only permit borrowing matching live leases plus committed audit admission. |
| 2 | OSC-002 | A structured capability never selects generated shell, a shell interpreter, a non-host target, secret interpolation, or an unbounded command fallback. |
| 3 | OSC-003 | Equal runtime probes produce an equal redacted operation-level snapshot; loss of one provider degrades only its affected operations. |
| 4 | OSC-004 | Risk never weakens below capability policy, BLACK is never encodable, provider code cannot approve, and privilege denial has no broader fallback. |
| 5 | OSC-005 | Verified requires fresh satisfying postcondition evidence; accepted is used only when observability terminates; uncertain or contradictory state never reports success. |
| 6 | OSC-006 | Already-satisfied state causes zero apply calls; rollback is advertised only with sufficient prior state and a tested inverse; partial work is reported exactly. |
| 7 | OSC-007 | One admission covers pre-observation through terminal outcome; mutation cannot begin if admission fails; terminal persistence interruption leaves one detectable incomplete admission that idempotent recovery closes without redispatch; no durable record, trace, approval projection, recovery payload, or error contains classified secret/content values. |
| 8 | OSC-008 | Equal resource sets acquire in one canonical order; conflicting writes cannot overlap; cancellation never causes a second uncertain mutation. |
| 9 | OSC-009 | Every approved prompt resolves to one canonical strict schema and stable result/event contract; ambiguity and BLACK scope produce no provider invocation. |
| 10 | OSC-010 | File lifecycle preserves path authority, symlink/protected-root policy, atomicity where supported, bounded verification, and governed writes. |
| 11 | OSC-011 | Default deletion is Trash; permanent deletion is distinct RED; archive extraction cannot escape staging or exceed declared bounds. |
| 12 | OSC-012 | Storage mutations bind typed UDisks identities and fresh topology evidence; destructive disk administration remains unrepresentable. |
| 13 | OSC-013 | App/process actions bind stable app or PID-plus-start-time identity; default process observations contain no argv/environment/cwd; command arguments require the separate RED operation; force kill is distinct RED and never claims rollback. |
| 14 | OSC-014 | Package mutation exactly matches an approved preflight plan and fresh package state; updates/removals never claim automatic downgrade rollback. |
| 15 | OSC-015 | Connectivity changes bind stable device/profile identity and opaque credential references; secrets are never model-visible. |
| 16 | OSC-016 | Network diagnosis is bounded and evidence-structured; VPN operations can only activate/deactivate existing profiles. |
| 17 | OSC-017 | Firewall control is high-level and ownership-aware; raw rules and mutation of grants not created by KRIA are impossible. |
| 18 | OSC-018 | Audio operations bind stable endpoints/streams, respect microphone privacy, and verify within declared rounding tolerance. |
| 19 | OSC-019 | Wayland never invokes X11-only display providers; topology changes either confirm with fresh evidence or attempt timed restore. |
| 20 | OSC-020 | Session-ending actions return Accepted only after OS acceptance; charge/profile/session operations obey privilege, rollback, and inhibitor bounds. |
| 21 | OSC-021 | Bluetooth, printer, scanner, and device operations use stable identities, bounded discovery, explicit ambiguity handling, and typed provider receipts. |
| 22 | OSC-022 | Health/log/recovery outputs are bounded and untrusted; allowlisted recipes cannot encode arbitrary units, commands, paths, or privileged operations. |
| 23 | OSC-023 | Clipboard/notification payloads obey consent, focus, retention, encryption, and redaction policy and never become audit parameters. |
| 24 | OSC-024 | Search roots/exclusions never broaden without approval; indexed content obeys permissions/bounds, and rebuilding the disposable projection preserves authorized result semantics. |
| 25 | OSC-025 | Secret payloads cannot serialize, debug-print, enter approval/audit/tool streams, or resolve outside their bound provider purpose and scope. |
| 26 | OSC-026 | Skills receive only expiring scoped grants; no skill can obtain raw HostOsControl, broker access, or broader authority than its grant. |
| 27 | OSC-027 | Automation persists typed canonical invocations, re-evaluates current admission at run time, and never stores or executes shell strings. |
| 28 | OSC-028 | Multi-step orchestration applies each step at most once, compensates only declared reversible work in reverse order, and preserves child receipts. |
| 29 | OSC-029 | Privacy-sensitive state and KRIA-owned retention obey classification, consent, encryption, clearing, and no-bypass rules across every domain. |
| 30 | OSC-030 | No normal prompt, tool, route, provider, broker, workflow, recipe, skill, or fallback schema can reconstruct prohibited administration. |
| 31 | OSC-031 | Unknown additive fields/enums and provider loss do not panic or alter unrelated domains; selection never depends solely on Ubuntu release number. |
| 32 | OSC-032 | GNOME X11/Wayland matrices select only proven providers; absent support yields truthful blocker/handoff rather than fabricated access. |
| 33 | OSC-033 | Every code-level test path uses injected fakes/private buses/captured argv/temp storage and performs zero live disruptive host mutation. |
| 34 | OSC-034 | Every list, scan, parse, retry, output, deadline, and concurrency path remains within declared bounds and cancellation semantics. |
| 35 | OSC-035 | After hard cutover, each implemented OS capability has one provider/runtime path and no direct Linux execution compatibility path remains. |
| 36 | OSC-036 | Every included v1/v2 capability has prompt→canonical tool→grant→resource→provider→verification→audit→result fake-backed closure; deferred/BLACK capabilities have none. |

The implementation must additionally preserve approval freshness, least authority, apply-at-most-once behavior, rollback honesty, secret non-observability, deterministic resource ordering, and truthful post-mutation incident reporting as specified in §§2, 4–6, and 13–17.

## Error Handling

The canonical error taxonomy and degraded-state behavior are normative in §§5 and 17. Errors are typed, bounded, redacted, actionable, and never interpreted as success merely because a provider was invoked.

## Testing Strategy

Only non-disruptive code-level tests are in scope. Unit, scripted-provider, fake D-Bus, captured-command, temporary-filesystem, in-memory SQLite, and in-process prompt tests are defined in §18; live OS mutation is explicitly excluded.

## 1. Current Repository Observation vs Planned Target

| Concern | Current observation | Planned target |
|---|---|---|
| Tool surface | OS tools are registered across `system_config`, `power`, files, apps, packages, process, scheduler, interaction, and disk modules | Preserve canonical public tool names where already valid; add cohesive missing tools and route all through injected providers |
| Execution | Several handlers invoke `tokio::process`, `ExecWrapper`, `sh -c`, or VM dispatch directly | All host OS operations enter `HostOsControl` providers or a governed structured-command fallback |
| Safety | `ExecutionGate`, `PolicyEngine`, decisions, HITL, and resource leases exist | Extend this path; do not create provider-local approval prompts or a third policy engine |
| Verification | Some handlers re-query manually; generic verifier is GUI/file oriented | Add typed OS postconditions and evidence; never equate exit code with state change |
| Audit | Safety SQLite audit exists; subprocess audit is best-effort and separate | Reuse the safety audit authority with additive OS-control before/after/evidence fields |
| Compatibility | CLI fallbacks and environment checks are scattered | Central `SessionContext`, capability probes, provider negotiation, and explicit degradation reasons |
| Testing | Parser tests and fakes exist, but direct host calls impede safe testing | All providers injectable; unit tests use scripted fakes and captured argv only |

## 2. Non-Negotiable Design Invariants

1. `kria-core` owns all OS-control semantics, policy metadata, provider selection, verification, and result truth.
2. Every OS mutation is bound to `ExecutionTarget::Host`; remote, VM, Docker, and generated-code environments cannot satisfy it.
3. Existing Tauri commands, WebSocket event names, approval IDs, and tool-stream event fields remain unchanged.
4. Providers never request approval. Approval completes before provider mutation begins.
5. Provider success means a normalized postcondition was verified, except explicitly asynchronous actions that return `Accepted`.
6. No shell interpreter is used by domain providers.
7. Secrets are opaque references and are absent from plans, tool streams, audit parameters, tracing, errors, and snapshots.
8. Provider selection is capability-based and runtime-probed; no Ubuntu release-number branching is permitted.
9. X11-only fallbacks are never attempted in a Wayland session.
10. Unsupported features are absent or reported unavailable; no silent simulation or best-effort success is allowed.
11. Rollback is advertised only when the exact prior state and a reliable inverse are available.
12. All queues, scans, outputs, D-Bus calls, command calls, retries, and verification loops are bounded.
13. Dangerous out-of-scope administration is absent from the capability schema and blocked in normal prompt routing.
14. Unit and contract tests never perform live disruptive OS mutations.

### 2.1 Existing authority reconciliation

KRIA currently has multiple execution-related mechanisms that must not become parallel OS authorities:

- `tools/capability_dispatch.rs`, `CapabilityPlatform`, `DefaultPermissionEngine`, and `GrantStore` remain the extension/marketplace discovery and skill-invocation plane only. Native OS capability descriptors are excluded from that plane. If an extension requests host OS effects, its provider receives no host handle and must submit a scoped invocation back through the canonical registered OS tool and `ExecutionGate`; extension permission/grant state cannot authorize the OS mutation. F0 contract tests prove `CapabilityPlatform::execute` cannot own or invoke a `HostOsControl` operation, and the cutover deletes/blocks any adapter that does.
- `ToolRegistry` is the canonical native model-tool registry, but handlers do not execute OS mutations directly. They submit typed requests to `OsControlRuntime`, which alone can construct `AdmittedMutationContext`.
- `DecisionStore` is hard-migrated from JSONL/in-memory fallback to the existing SQLite durable authority for OS decisions. OS-action decision creation, approval/denial/expiry resolution, and grant issuance are fallible transactions. `default_persistent` cannot silently fall back to memory for an OS action, and a UI approval is ineffective until the resolution commit succeeds.
- Existing target-mismatch/approval overrides in `gui_wiring.rs`, direct handler resume in `resume_executor.rs`, and any path that logs a failed decision update then continues are removed for OS capabilities. Non-OS behavior may remain only behind explicit type separation and bypass-negative tests.

This is a hard authority cutover, not a third permission engine or a dual-run period.

## 3. Planned Module Ownership

```text
crates/kria-core/src/os_control/
├── mod.rs
├── contract.rs                 # HostOsControl, domain ports, request/result contracts
├── capability.rs               # probes, availability, provider selection
├── context.rs                  # SessionContext and HostExecutionContext
├── error.rs                    # canonical taxonomy and remediation
├── runtime.rs                  # admitted execution, observe/apply/verify/rollback
├── receipt.rs                  # observations, evidence, receipts, rollback token metadata
├── audit.rs                    # safety AuditLogger adapter and redaction
├── resource.rs                 # resource declarations and ordering
├── redaction.rs                # parameter/state classifications
├── testing.rs                  # os-control-test fakes, deny-live sentinel, call recorder
├── files/{mod,trash,archive,metadata}.rs
├── applications/{mod,intents,autostart}.rs
├── processes/mod.rs
├── packages/{mod,updates}.rs
├── connectivity/{mod,diagnostics,firewall,vpn}.rs
├── audio/{mod,media}.rs
├── display/{mod,rollback_timer}.rs
├── power/mod.rs
├── hardware/mod.rs
├── firmware/mod.rs
├── bluetooth/mod.rs
├── storage/mod.rs
├── health/{mod,logs,recovery}.rs
├── clipboard/mod.rs
├── notifications/mod.rs
├── search/mod.rs
├── secrets/mod.rs
├── privacy/mod.rs
├── printing/mod.rs
├── scanning/mod.rs
├── backup/mod.rs
├── automation/mod.rs
├── sandbox/mod.rs
└── linux/
    ├── mod.rs
    ├── probe.rs
    ├── dbus.rs
    ├── structured_command.rs
    └── providers/
        ├── logind.rs
        ├── upower.rs
        ├── power_profiles.rs
        ├── network_manager.rs
        ├── bluez.rs
        ├── udisks.rs
        ├── pipewire.rs
        ├── gnome_display.rs
        ├── kscreen_display.rs
        ├── wlroots_display.rs
        ├── xrandr_display.rs
        ├── secret_service.rs
        ├── notifications.rs
        ├── packagekit.rs
        ├── distro_packages.rs
        ├── firewall.rs
        ├── cups.rs
        └── fallback_commands.rs
```

Existing `tools/*.rs` files remain thin model-callable facades. `system_config.rs`, `power.rs`, `file_ops.rs`, `app_lifecycle.rs`, `process.rs`, `packages.rs`, `scheduler.rs`, `disk.rs`, `interaction.rs`, and `communication.rs` delegate to injected domain ports. They no longer own Linux subprocess policy.

Composition roots inject the live provider into `ToolRegistry`/`ToolContext`. Headless tests inject scripted providers. `kria-desktop` and `kria-server` do not duplicate domain implementations.

## 4. Core Contracts

```rust
#[async_trait]
pub trait HostOsControl: Send + Sync {
    async fn capabilities(&self, ctx: &HostExecutionContext)
        -> Result<CapabilitySnapshot, OsControlError>;
    fn files(&self) -> &dyn FileControl;
    fn applications(&self) -> &dyn ApplicationControl;
    fn processes(&self) -> &dyn ProcessControl;
    fn packages(&self) -> &dyn PackageControl;
    fn connectivity(&self) -> &dyn ConnectivityControl;
    fn firewall(&self) -> &dyn FirewallControl;
    fn audio(&self) -> &dyn AudioControl;
    fn media(&self) -> &dyn MediaControl;
    fn display(&self) -> &dyn DisplayControl;
    fn power(&self) -> &dyn PowerControl;
    fn hardware(&self) -> &dyn HardwareControl;
    fn firmware(&self) -> Option<&dyn FirmwareAwareness>;
    fn bluetooth(&self) -> &dyn BluetoothControl;
    fn storage(&self) -> &dyn StorageControl;
    fn health(&self) -> &dyn SystemHealthControl;
    fn clipboard(&self) -> &dyn ClipboardControl;
    fn notifications(&self) -> &dyn NotificationControl;
    fn search(&self) -> &dyn SearchControl;
    fn printing(&self) -> &dyn PrintControl;
    fn privacy(&self) -> &dyn PrivacyControl;
    fn automation(&self) -> &dyn AutomationControl;
    fn sandbox(&self) -> &dyn SandboxGrantControl;
    fn scanning(&self) -> Option<&dyn ScanControl>;
    fn backup(&self) -> Option<&dyn BackupIntegration>;
    fn secrets(&self) -> &dyn CredentialStore;
}

#[async_trait]
pub trait DesiredStateControl<R, O>: Send + Sync {
    async fn observe(&self, ctx: &HostExecutionContext, request: &R)
        -> Result<O, OsControlError>;
    async fn apply(&self, ctx: &AdmittedMutationContext<'_>, request: &R, desired: &O)
        -> Result<ApplyOutcome, OsControlError>;
    async fn verify(&self, ctx: &HostExecutionContext, request: &R, desired: &O)
        -> Result<VerificationReport<O>, OsControlError>;
    async fn rollback(&self, ctx: &AdmittedMutationContext<'_>, token: &RollbackToken)
        -> Result<ApplyOutcome, OsControlError>;
}
```

Domain traits may use specialized methods, but every mutation maps to this lifecycle and every specialized provider mutator MUST accept `&AdmittedMutationContext<'_>` (or an operation-specific non-cloneable wrapper that borrows it) and return `Result<ApplyOutcome, OsControlError>` or a closed operation-specific dispatch fact convertible only to `ApplyOutcome`. `&HostExecutionContext` is observation-only and MUST NOT appear on a provider mutator. Provider ports never return `MutationReceipt`/`MutationResult`; those are runtime/tool-facade outputs. This rule applies to all required and optional ports, including `FirewallControl`, `MediaControl`, `CredentialStore`, `ScanControl`, and `BackupIntegration`; compile-fail contract tests enumerate every mutating port. Providers return normalized observations rather than model prose. `Err(OsControlError)` from `apply` or `rollback` is legal only when the provider proves dispatch/effect did not start. Once mutation may have started, the provider returns the appropriate `ApplyOutcome` variant; the runtime observes, verifies, optionally rolls back, and constructs the only valid `MutationReceipt` state.

### Execution grant and context

```rust
pub struct ExecutionGrant {
    grant_id: GrantId,
    session_id: SessionId,
    action_hash: Digest,
    parameter_hash: Digest,
    target_hash: Digest,
    decision_id: Option<DecisionId>,
    risk: RiskLevel,
    decision: GrantDecision,
    capability_snapshot_revision: SnapshotRevision,
    resource_set_digest: Digest,
    issued_at: SystemTime,
    expires_at: SystemTime,
    nonce: GrantNonce,
}

pub struct HostExecutionContext {
    pub correlation_id: CorrelationId,
    pub action_id: ActionId,
    observation_audit: ObservationAuditAuthority,
    pub session: Arc<SessionContext>,
    pub cancellation: CancellationToken,
    pub deadline: Instant,
    pub redaction: RedactionPolicy,
}

pub struct AdmittedMutationContext<'a> {
    observation: &'a HostExecutionContext,
    grant: &'a ExecutionGrant,
    permit: MutationPermit<'a>,
}

struct MutationPermit<'a> {
    lease_set: &'a AcquiredResourceLeaseSet,
    audit_admission: &'a AuditAdmissionToken,
    resource_set_digest: Digest,
}
```

`HostExecutionContext` authorizes observation only and carries no mutation grant; it is safe to create after read-policy and durable logical-action admission. `MutationPermit` and `AdmittedMutationContext` have private fields, are non-`Clone`, and can be constructed only by `OsControlRuntime` after it verifies that (a) the fresh `ExecutionGrant` matches the action/parameter/target/capability/resource bindings of the existing action admission, (b) the exact canonical resource set named by `ExecutionGrant.resource_set_digest` is currently held, and (c) the same admission's fallible commit returned the matching `AuditAdmissionToken`. The token is created before pre-observation and is not itself grant-bound; mutation authority arises only when runtime seals it together with the later grant and held leases. The permit borrows all three authorities, so apply cannot outlive them. Providers accept `AdmittedMutationContext` for every mutation, making invocation before approval, resource acquisition, or audit admission unrepresentable in safe Rust.

`ExecutionGrant` fields are private outside the execution-gate module and its constructor is crate-private. Only `ExecutionGate::evaluate` for an admitted no-confirmation action or `ExecutionGate::revalidate_resume` for an approved durable decision may issue it. Issuance recomputes strict-schema canonical parameters, explicit parameter/action/host-target/resource-set digests, current risk, session identity, expiry, and capability revision. Resume also re-runs host authority, capability availability, and resource derivation. A handler can receive an observation context but cannot construct, renew, weaken, or retarget a grant; only the runtime can seal it with held leases and durable audit admission into a mutation context. Providers and the structured-command executor validate all bindings before dispatch.

### Apply outcome, terminal lifecycle, and incidents

Providers may return only one of four dispatch facts. Each payload type has private fields and validated constructors on the payload type; provider adapters can call those constructors but cannot construct or relabel runtime receipt states. The enum does not expose interchangeable bags of fields.

```rust
pub enum ApplyOutcome {
    Applied(AppliedDispatch),
    Accepted(AcceptedDispatch),
    Uncertain(UncertainDispatch),
    PartiallyApplied(PartialDispatch),
}

pub struct AppliedDispatch {
    provider_receipt_digest: Option<Digest>,
    warnings: BoundedVec<SafeWarning>,
}
pub struct AcceptedDispatch {
    provider_receipt_digest: Option<Digest>,
    acceptance: AcceptanceEvidence,
    warnings: BoundedVec<SafeWarning>,
}
pub struct UncertainDispatch {
    provider_receipt_digest: Option<Digest>,
    cause: UncertainEffectCause,
    warnings: BoundedVec<SafeWarning>,
}
pub struct PartialDispatch {
    provider_receipt_digest: Option<Digest>,
    completed_steps: NonEmptyBoundedVec<SafeStepId>,
    failed_step: SafeStepId,
    cause: PartialEffectCause,
    warnings: BoundedVec<SafeWarning>,
}

pub enum ActionLifecycle {
    Unchanged,
    Verified,
    Accepted,
    Unverified,
    VerificationFailed,
    RolledBack,
    PartiallyApplied,
}

pub enum UnverifiedDispatch {
    Applied(AppliedDispatch),
    Uncertain(UncertainDispatch),
}
pub enum ContradictedDispatch {
    Applied(AppliedDispatch),
    Uncertain(UncertainDispatch),
}
pub struct VerificationContradiction {
    expected: Digest,
    observed: Option<Digest>,
    code: SafeErrorCode,
}
pub struct RollbackFailure {
    code: SafeErrorCode,
    observed_digest: Option<Digest>,
}
pub enum FailureRollbackState {
    NotAttempted(RollbackAvailability),
    Failed(RollbackFailure),
}

pub enum AuditCompletionState {
    Recorded { record_id: AuditRecordId },
    PendingRecovery {
        admission_id: AuditAdmissionId,
        recovery_key: AuditRecoveryKey,
    },
}

pub struct ReceiptCommon {
    receipt_id: ReceiptId,
    action_hash: Digest,
    target_hash: Digest,
    provider: ProviderId,
    latency_ms: u64,
}

// Private to os_control::receipt; provider and adapter modules cannot name or construct it.
enum RuntimeReceiptState<O> {
    Unchanged {
        observation: RedactedObservation<O>,
    },
    Verified {
        before: RedactedObservation<O>,
        after: RedactedObservation<O>,
        apply: AppliedDispatch,
        verification: SatisfyingVerification<O>,
        rollback: RollbackAvailability,
    },
    Accepted {
        before: Option<RedactedObservation<O>>,
        apply: AcceptedDispatch,
    },
    Unverified {
        before: Option<RedactedObservation<O>>,
        after: Option<RedactedObservation<O>>,
        dispatch: UnverifiedDispatch,
        cause: UnverifiedCause,
        rollback: FailureRollbackState,
    },
    VerificationFailed {
        before: RedactedObservation<O>,
        after: RedactedObservation<O>,
        dispatch: ContradictedDispatch,
        contradiction: VerificationContradiction,
        rollback: FailureRollbackState,
    },
    RolledBack {
        before: RedactedObservation<O>,
        failed_after: Option<RedactedObservation<O>>,
        original: RollbackEligibleFailure,
        rollback_verification: SatisfyingVerification<O>,
    },
    PartiallyApplied {
        before: Option<RedactedObservation<O>>,
        after: Option<RedactedObservation<O>>,
        apply: PartialDispatch,
        rollback: FailureRollbackState,
    },
}

pub struct MutationReceipt<O> {
    common: ReceiptCommon,
    state: RuntimeReceiptState<O>,
    audit_completion: AuditCompletionState,
}

pub struct SafeReceiptSummary {
    receipt_id: ReceiptId,
    action_hash: Digest,
    target_hash: Digest,
    provider: ProviderId,
    lifecycle: ActionLifecycle,
    changed: bool,
    before_digest: Option<Digest>,
    after_digest: Option<Digest>,
    incident_codes: BoundedVec<SafeErrorCode>,
}

pub type MutationResult<O> = Result<MutationReceipt<O>, OsControlError>;
```

`RuntimeReceiptState`, every state constructor, and every `SafeReceiptSummary` field are private to `os_control::receipt`; `OsControlRuntime` receives a private construction authority from that module and is the only caller. `SafeReceiptSummary::from_receipt` is the sole constructor and derives lifecycle/changed/digests from a validated private state, so recovery code cannot forge independent flags. Consumers receive read-only accessors and a serialized `ReceiptView`, never the state enum. `lifecycle()`, `changed()`, `verification()`, `rollback_availability()`, and `safe_summary()` are derived. `AppliedDispatch` is the only dispatch fact accepted by `Verified`; `AcceptedDispatch` is the only fact accepted by `Accepted`; `PartialDispatch` is the only fact accepted by `PartiallyApplied`. `Verified` contains only `RollbackAvailability`, which cannot express an attempted failure; only failure states contain `FailureRollbackState`, whose `Failed` variant carries `RollbackFailure`. Thus `Verified + Uncertain`, `Verified + rollback failure`, `Accepted + Applied`, arbitrary incident placement, forged recovery summaries, and independent lifecycle/changed flags have no constructible type.

The exact terminal rules are:

- Pre-mutation validation, availability, policy, approval, resource, permission, protocol, timeout, cancellation, and audit-admission failures return `OsControlError` and no receipt.
- Provider rejection may return a pre-mutation error only when the adapter proves dispatch/effect did not start. A reported failure after dispatch is `UncertainDispatch` with `ProviderReportedFailureAfterDispatch`; known residue is `PartialDispatch`.
- `AppliedDispatch` followed by fresh satisfying evidence yields `Verified`. Session-ending or explicitly asynchronous loss-of-observability actions yield `Accepted` only from `AcceptedDispatch` carrying provider acceptance evidence.
- Applied or uncertain dispatch without a decisive observation yields `Unverified`; contradictory fresh evidence yields `VerificationFailed`; known multi-step residue yields `PartiallyApplied`.
- Successful verified rollback yields `RolledBack`. Rollback failure remains attached as `FailureRollbackState::Failed` only to the truthful failure state and never becomes a generic error; a `Verified` state can advertise availability but cannot contain rollback failure.
- Terminal-audit append failure preserves the OS-state lifecycle and returns `AuditCompletionState::PendingRecovery`; no durable incident ID is claimed. The runtime persists/reuses the admission recovery key, marks audit health unavailable, blocks subsequent automatic mutations, and reconciles the terminal record idempotently without invoking the provider again.

Tool result adapters retain existing top-level fields (`volume`, `brightness`, `wifi`, `power_plan`, `changed`, `already_in_desired_state`, `action`). All new metadata uses one additive nested object so existing consumers do not collide:

```json
{
  "os_control": {
    "receipt_id": "opaque-id",
    "provider": "provider-id",
    "lifecycle": "verified|unchanged|accepted|unverified|verification_failed|rolled_back|partially_applied",
    "verification": { "verified": true, "source": "...", "reliability": "..." },
    "rollback_available": false,
    "availability": "available|degraded|unavailable",
    "incidents": [],
    "audit_completion": "recorded|pending_recovery",
    "remediation": null
  }
}
```

Task 0.1 freezes this exact additive envelope in the normative contract manifest; Task 1.2 implements and validates the registry/result snapshot.

## 5. Error Contract

`OsControlError` is exclusively pre-mutation or proven-no-effect. Post-mutation failures and uncertain/partial outcomes are represented by `MutationReceipt.incidents`.

```rust
pub enum OsControlError {
    Unsupported { capability: CapabilityId, reason: SafeText },
    Unavailable { provider: Option<ProviderId>, reason: SafeText, retryable: bool },
    InvalidRequest { field: SafeField, reason: SafeText },
    AmbiguousTarget { kind: SafeText, candidates: BoundedVec<SafeCandidate> },
    PermissionDenied { authority: SafeText, remediation: SafeText },
    PolicyDenied { reason: SafeText },
    ApprovalExpired,
    GrantInvalid { reason: GrantInvalidReason },
    TargetChanged,
    ResourceBusy { resource: SafeResource, owner: Option<SafeText> },
    TimedOutBeforeMutation { operation: SafeOperation, timeout_ms: u64 },
    CancelledBeforeMutation,
    ProtocolBeforeMutation { provider: ProviderId, operation: SafeOperation },
    AuditUnavailable,
}
```

Every error serializes through one frozen envelope; absent values are JSON `null`, not omitted, so adapters cannot invent divergent shapes:

```json
{
  "error": {
    "code": "os_control.unavailable",
    "message": "Safe bounded message",
    "retryable": true,
    "remediation": "Safe remediation or null",
    "field": null
  },
  "os_control": {
    "provider": "provider-id-or-null",
    "lifecycle": null,
    "availability": "unavailable",
    "receipt_summary": null
  }
}
```

The code set is closed and versioned by Task 0.1. Raw stderr, D-Bus payloads, command strings, secret references capable of correlation, untrusted control characters, and provider-specific object paths are never returned. Tool adapters may preserve an existing outer transport failure field only if it contains this envelope unchanged.

## 6. Execution Flow

```text
Prompt
  → registry selects one canonical strict-schema tool
  → ExecutionGate readiness + preflight + host ExecutionAuthority
  → capability probe and typed resource derivation
  → read-policy admission
  → append one durable logical-action Admission; obtain observation token/context (fail closed where required)
  → acquire shared read lease + privacy-authorized observe
  → if desired state already holds: append sole Unchanged terminal → ToolEnd
  → otherwise release read lease and evaluate mutation PolicyEngine
  → DecisionStore creates durable InteractionDecision when approval is required
  → HitlGateway presents only a redacted projection and waits; it grants no authority
  → ExecutionGate::revalidate_resume recomputes host/action/parameter/target/risk/capability/resources
  → ExecutionGate issues the short-lived ExecutionGrant
  → acquire deterministic exclusive resource leases
  → runtime matches the existing admission token + fresh grant + live lease digest and seals MutationPermit
  → re-observe under exclusive lease; append sole Unchanged terminal if state converged
  → apply exactly once using private AdmittedMutationContext via D-Bus/portal/structured argv/broker operation
  → once apply may start, represent every outcome in ApplyOutcome/MutationReceipt
  → independent fresh observe and typed postcondition verification
  → optional bounded rollback only when predeclared and prior state is sufficient
  → append the admission's sole integrity-linked terminal completion/incident
  → release resources
  → existing ToolEnd/result stream with frozen envelope
```

There is exactly one logical-action admission. It is committed after read policy/resource derivation and before the first provider observation; it binds action, strict parameters, host target, capability revision, canonical prospective resource digest, session, correlation, and recovery key, but no not-yet-issued grant. `HostExecutionContext` carries the matching observation authority and no grant. If change is required, the later grant must reproduce those bindings; runtime then combines that grant with the same admission token and held write leases into `AdmittedMutationContext` without appending another admission. The pre-observation cannot leak sensitive state to the model. Under-lease re-observation closes the time-of-check/time-of-use gap without consuming or changing the grant. `ExecutionGateOutcome::Proceed` and `ResumeGateOutcome::Ready` carry a grant only for the mutation phase; a bare boolean-ready state is insufficient for apply. Approval expiry, target drift, risk increase, capability revision change, or resource re-derivation mismatch invalidates the admission for mutation, appends its sole non-dispatch terminal, and returns to a new logical request/admission; it never mutates under stale authority. Read-only actions stop after observation and append their terminal. Session-ending actions become non-cancellable after OS acceptance, release ordinary resources, and return `Accepted`.

## 7. Provider Selection

Each operation has an ordered provider set. Selection is per operation, not merely per domain. A provider is eligible only when its probe proves the required interface, method/property, session/system bus, permissions, and semantics.

Provider fallback occurs only before mutation. Once a provider may have mutated state, the runtime verifies that state and does not try a second provider blindly.

```rust
pub struct CapabilityAvailability {
    pub capability: CapabilityId,
    pub status: AvailabilityStatus,
    pub selected: Option<ProviderId>,
    pub fallbacks: Vec<ProviderId>,
    pub display_servers: DisplayServerSupport,
    pub requires_root: bool,
    pub requires_confirmation: ConfirmationPolicy,
    pub reversible: bool,
    pub verifiable: VerificationClass,
    pub reason: Option<String>,
}
```

Cache probe results with short domain-specific TTLs and invalidate on D-Bus owner changes, session changes, device events, package changes, or provider protocol failure.

## 8. Linux and Ubuntu Compatibility Strategy

The target is Ubuntu 24.04 and later, with best-effort compatibility for supported earlier Ubuntu versions. No software can guarantee compatibility with unknown future releases; this design minimizes breakage through runtime negotiation and stable interfaces.

Rules:

1. Do not parse `/etc/os-release` to choose behavior except for diagnostics.
2. Prefer freedesktop/system service contracts: logind, UPower, NetworkManager, BlueZ, UDisks2, Secret Service, Notifications, PackageKit, CUPS/IPP, and portals.
3. Probe interface methods/properties before use; tolerate additive fields and unknown enum values.
4. Pin Rust dependencies exactly while keeping OS service compatibility runtime-based.
5. Keep desktop-specific display adapters isolated behind `DisplayControl`.
6. Preserve a structured CLI fallback only when no stable API exists and Ubuntu packages the utility conventionally.
7. Emit `Unsupported` with provider evidence when no safe adapter exists.

### Session matrix

| Domain | Wayland | X11 | Primary | Notes |
|---|---:|---:|---|---|
| Files/packages/processes/storage | Yes | Yes | Rust/sysinfo/freedesktop | Display-neutral |
| Network/VPN/firewall | Yes | Yes | NetworkManager/firewall provider | Display-neutral |
| Bluetooth | Yes | Yes | BlueZ | Display-neutral |
| Audio | Yes | Yes | PipeWire/WirePlumber | Session audio bus/socket required |
| Power/session | Yes | Yes | logind/UPower/PPD | Display-neutral |
| Clipboard | Yes | Yes | portal/native backend | Wayland access may be focus/security constrained |
| Notifications | Yes | Yes | freedesktop/portal | Desktop-neutral normalized contract |
| Basic brightness | Yes | Yes | GNOME settings daemon/hardware | XRandR gamma is X11-only and not physical brightness |
| Display topology | Adapter-dependent | Yes | Mutter/KScreen/wlroots/XRandR | Capability reported per operation |
| Search/automation/secrets | Yes | Yes | local index/system services | Display-neutral |

Wayland denial is not a reason to invoke X11 tools. XWayland availability is reported separately and may only control XWayland-owned resources.

## 9. Domain Provider Design

### 9.1 Files, Trash, and Archives

Reuse the existing governed environment/file abstractions where they correctly enforce local paths, but move host file semantics behind `FileControl`. Add freedesktop Trash behavior using the user data directory and `.trashinfo` semantics or a maintained FOSS implementation. Permanent delete remains distinct.

Cross-device directory move algorithm:

1. Resolve and bind source/destination identities.
2. Acquire both path leases in canonical order.
3. Stage a recursive copy without following unexpected symlinks.
4. Verify type, count, size, and content hashes according to bounded policy.
5. Atomically expose destination where possible.
6. Delete source only after verification.
7. Report partial state and cleanup token on failure.

Archive extraction uses a staging directory and rejects absolute paths, `..`, escaping symlinks, special devices, excessive entries, excessive expanded bytes, and suspicious compression ratios.

### 9.2 Applications and Processes

Extend `InstalledAppRegistry` and `IntentDispatcher`; do not duplicate `.desktop` parsing. `ApplicationControl` resolves stable IDs and distinguishes graceful app close from process kill. `ProcessControl` uses `sysinfo` for observation and native signals/priority APIs where safe. Process identity includes start time to prevent PID-reuse target substitution.

Default applications and MIME associations use freedesktop configuration with before-state capture. User autostart edits only the current user's XDG autostart entries.

Process observation is content-free by default. `ProcessObservation` contains `ProcessIdentity { pid, start_time }`, bounded redacted executable label, executable identity digest, owner reference, state, CPU, memory, and start time. It never contains environment, cwd, open files, or argv. `CommandMetadataState` is exactly `NotRequested | Unavailable | PermissionDenied | Redacted { argument_count, executable_digest, argv_digest }`. Raw bounded argv, when explicitly requested, is exposed only by the separate RED `get_process_command_metadata` operation; its approval and audit projection contain purpose, process identity, count, and digest but no argument content.

`BoundedCommandMetadata` has retention disposition `EphemeralCurrentTurn`: it may exist only in the protected provider-to-current-tool-result buffer needed to answer the explicit request. It is excluded from SQLite conversation/tool-result history, memory extraction, RAG/indexing, workflow variables, receipts, audit, traces, analytics, crash reports, notifications, and approval/decision payloads. The buffer uses zeroizing bounded elements and is cleared on tool-result consumption, turn completion, cancellation, timeout, or session teardown, whichever occurs first; there is no TTL-based background retention and no history retrieval API. Only the user-visible current response may contain the bounded arguments. Persistence adapters must reject this DTO rather than silently serialize it.

### 9.3 Packages and Updates

`PackageControl` normalizes providers behind one plan:

```rust
pub struct PackagePlan {
    pub operation: PackageOperation,
    pub provider: ProviderId,
    pub requested: Vec<PackageRef>,
    pub installs: Vec<PackageChange>,
    pub upgrades: Vec<PackageChange>,
    pub removals: Vec<PackageChange>,
    pub download_bytes: Option<u64>,
    pub disk_delta_bytes: Option<i64>,
    pub security_relevant: Option<bool>,
    pub reboot_required: Option<bool>,
}
```

Use PackageKit when it gives sufficient transaction and progress semantics; retain typed APT/DNF/Pacman/Zypper/Snap/Flatpak adapters as needed. On Ubuntu, APT, Snap, and Flatpak may coexist and provider identity remains explicit. Package mutation is not represented as rollbackable merely because an inverse command exists.

### 9.4 Connectivity

Primary provider is NetworkManager D-Bus. Secret material is requested through a credential reference and supplied only to the provider operation. Normalize Wi-Fi and Ethernet profiles and stable device IDs.

`NetworkDiagnosticsProvider` executes a bounded state machine rather than exposing dozens of model tools:

```text
Link → Address → Route → Gateway → DNS → Internet → CaptivePortal → OptionalPath
```

Each step has a deadline and produces structured evidence. It stops early only when later checks would be meaningless, not merely when one endpoint is unreachable.

VPN control is limited to activating/deactivating existing profiles. Firewall uses UFW/firewalld high-level state adapters. Raw rules remain absent. The v2 connectivity surface is frozen at the domain boundary rather than hidden in generic profile maps:

```rust
#[async_trait]
pub trait ConnectivityControl: Send + Sync {
    async fn get_state(&self, ctx: &HostExecutionContext) -> Result<NetworkState, OsControlError>;
    async fn get_hotspot_state(&self, ctx: &HostExecutionContext) -> Result<HotspotState, OsControlError>;
    async fn set_hotspot(&self, ctx: &AdmittedMutationContext<'_>, desired: HotspotDesiredState) -> Result<ApplyOutcome, OsControlError>;
    async fn get_proxy_state(&self, ctx: &HostExecutionContext) -> Result<ProxyState, OsControlError>;
    async fn set_proxy_profile(&self, ctx: &AdmittedMutationContext<'_>, desired: ProxyProfileDesiredState) -> Result<ApplyOutcome, OsControlError>;
    async fn list_saved_credentials(&self, ctx: &HostExecutionContext, filter: ConnectivityCredentialFilter) -> Result<BoundedVec<ConnectivityCredentialMetadata>, OsControlError>;
    async fn replace_saved_credential(&self, ctx: &AdmittedMutationContext<'_>, request: ReplaceConnectivityCredential) -> Result<ApplyOutcome, OsControlError>;
    async fn delete_saved_credential(&self, ctx: &AdmittedMutationContext<'_>, request: DeleteConnectivityCredential) -> Result<ApplyOutcome, OsControlError>;
}
```

The same signature rule is normative for every specialized port: reads take `&HostExecutionContext`; mutations take `&AdmittedMutationContext<'_>`. `HostOsControl` is held only by composition and `OsControlRuntime`; tool handlers receive `Arc<OsControlRuntime>`, not raw provider authority. Optional ports are represented by capability availability, but if present they obey the same compile-time mutation boundary.

`HotspotDesiredState` accepts a stable device/profile identity and opaque generated-or-existing secret reference, never plaintext. `ProxyProfileDesiredState` supports only the reviewed system/desktop proxy modes and bounded URI/host exclusions; it exposes no arbitrary environment or config-file write. Saved credential metadata contains purpose, profile binding, timestamps, and reference digest only; replacement/deletion coordinates NetworkManager and `CredentialStore` with explicit partial-state receipts.

### 9.5 Audio

Primary target is PipeWire/WirePlumber. Because stable high-level APIs vary, implementation may begin with structured `wpctl` and retain typed parsers, then add native integration without changing the domain contract. PulseAudio and ALSA are fallback providers.

Audio observations use stable provider IDs where available; human names are display labels only. Microphone unmute receives privacy-sensitive policy. Verification tolerates provider rounding with a specified percentage tolerance.

### 9.6 Display

`DisplayControl` separates physical backlight from compositor topology and gamma. Provider families:

- GNOME/Mutter session D-Bus for Wayland/X11 topology where supported.
- KDE KScreen D-Bus.
- wlroots adapter where a stable management protocol/tool is available.
- XRandR only for X11.
- `brightnessctl`/hardware provider for physical backlight.

Full topology mutations are transactional at the KRIA level: capture snapshot, apply, verify, emit confirmation request, and revert on timeout. The rollback timer is owned by core runtime and survives loss of the immediate tool future within the process.

### 9.7 Power and Session

Use `org.freedesktop.login1` for lock/suspend/hibernate/poweroff/reboot/session operations, UPower for battery, and power-profiles-daemon for profiles. Polkit remains the OS authority after KRIA approval. `interactive=true` may be used only when the action has a valid grant and presentation can surface the authorization dialog.

Session-ending actions return an accepted receipt after D-Bus acceptance. Delayed shutdown uses a KRIA-owned cancellable scheduler entry rather than shell `shutdown +N` where feasible. `PowerControl` additionally freezes `get_battery_health` and `set_battery_charge_thresholds`; threshold mutation accepts validated lower/upper percentages, selects only a probed recognized adapter, captures exact prior thresholds, verifies the normalized values, and returns `Unavailable` on unsupported hardware. It never writes arbitrary sysfs or embedded-controller paths.

### 9.8 Bluetooth and Devices

Use BlueZ D-Bus plus an agent implementation that bridges pairing decisions into existing approval presentation. Scans are time-bounded and deduplicate by stable address/object identity. Bluetooth state events invalidate capability/device observations.

Storage uses UDisks2. Printing uses CUPS/IPP. Sensor discovery uses read-only sysfs/hwmon or a stable provider. No provider writes vendor-specific control files.

### 9.9 Clipboard and Notifications

`ClipboardControl` wraps platform/session backends and reports whether reads require focus or user gesture. History is a separate opt-in service with encrypted SQLite payloads, TTL, size caps, MIME allowlist, source exclusions, and immediate erase.

`NotificationControl` uses freedesktop notifications or portal APIs. Notification actions are converted into authenticated local events routed back through policy; action callbacks never invoke providers directly.

### 9.10 Search

Search authority remains the filesystem; SQLite FTS is a rebuildable projection. The index records path identity, metadata, extraction status, source revision, and content digest. Watchers enqueue bounded idempotent projection updates. Search never broadens its roots based on model suggestion alone.

### 9.11 Secrets, Privacy, and Sandbox

Use Secret Service for secret payloads. KRIA stores opaque references and metadata only. A credential may be revealed to a provider but not to the model. `CredentialStore` freezes bounded `list_metadata`, `store`, `replace`, `delete`, and provider-only `resolve_for_operation` methods. Resolution requires the same valid execution grant, capability, provider purpose, profile/scope binding, and expiry; it returns a non-serializable zeroizing payload wrapper. Connectivity-specific list/replace/delete operations coordinate profile linkage through `ConnectivityControl` rather than exposing a generic secret value. Current-user privacy controls expose only stable, reversible desktop settings.

OpenClaw grants use existing capability-control and audit systems. OS-control grants specify domain operation, path/device/network scope, expiration, source skill, and approval. A skill cannot receive raw HostOsControl or the privileged broker.

### 9.12 Automation

Automation stores typed capability invocations, never shell strings. At run time it re-evaluates availability, policy, target, risk, resources, and secrets. Approval is not permanently inherited unless an explicit bounded grant supports the same action, scope, parameters, and expiry.

### 9.13 System Health, Logs, and Recovery

`SystemHealthControl` normalizes CPU, memory, filesystem, network, battery, GPU, process, thermal, sensor, pressure, failed-service, reboot-required, and storage-health observations. Missing sensors are represented as unavailable fields, not zero values. Collection is bounded by source, deadline, item count, and payload size.

`LogQueryPort` accepts only typed source, monotonic/wall-clock range, severity, unit/application identity, cursor, and limit. On Ubuntu it may use journal APIs or a fixed `journalctl` argv adapter. Log text is untrusted content: control characters are escaped, instruction-like text is never executed, and audit stores only query metadata/digests.

`RecoveryRecipeRegistry` is closed-world. Each versioned recipe declares supported providers, preconditions, risk, resources, typed steps, expected postconditions, compensation, and stop conditions. Recipes may operate only on named desktop subsystems reviewed in code. They cannot accept arbitrary unit names, commands, paths, kernel settings, security policies, or privileged broker operations. Each step creates a child receipt linked to the parent diagnostic action.

### 9.14 Hardware, Firmware, Scanning, and Backup Integrations

`HardwareControl::get_sensors` returns a bounded normalized snapshot of temperatures, fan readings, power/pressure observations, battery details, and explicit unavailable fields from read-only trusted sources. It has no mutation method. Firmware support is read-only through optional `FirmwareAwareness::get_status` using trusted `fwupd` metadata: inventory, update availability, power/reboot prerequisites, and a trusted-utility handoff descriptor. No update/install/flash method exists in the v1/v2 provider or broker contract.

`ScanControl` freezes `list_scanners` and bounded `scan_document` operations with stable scanner identity, declared format/resolution/page/output limits, cancellation, staged destination commit, and no driver configuration. `BackupIntegration` freezes `get_status`, `start`, and `plan_restore_handoff` for recognized installed providers. Start returns the provider job/receipt state; restore produces an exact reviewed plan and trusted handoff rather than executing arbitrary restore commands. The filesystem remains authority and KRIA does not represent itself as a scanner driver or backup engine.

## 10. Canonical Tool and Provider-Port Contract

This section is normative: implementation tasks transcribe and validate it; they do not choose names later. Existing references that use a different name undergo the repository-wide hard cutover in Task 0.1—no alias or compatibility shim remains. Every parameter object is closed (`additionalProperties: false`), every string/collection has the §19 bounds, every mutation is host-only, and every result preserves existing required top-level fields plus the §4 `os_control` envelope.

Compact schema notation is `field:Type`, optional `field?:Type`, and `Enum{...}`. Shared types are strict newtypes: `PathRef`, `AppId`, `ProcessIdentity(pid,start_time)`, `PackageRef(provider,name)`, `NetworkDeviceId`, `NetworkProfileId`, `BluetoothDeviceId`, `AudioEndpointId`, `DisplayId`, `StorageDeviceId`, `PrinterId`, `ScannerId`, `SecretRef`, `ReceiptId`, `Cursor`, `BoundedText`, `Percent(0..100)`, and `DurationMs`. Human labels never substitute for stable IDs on mutation.

### 10.1 Core files, applications, processes, packages, and storage

| Canonical tool and exact input | Provider port operation → normalized output | Risk / phase |
|---|---|---|
| `get_os_capabilities(domain?:DomainId, include_unavailable?:bool)` | `HostOsControl.capabilities` → `CapabilitySnapshot` | GREEN / F1 |
| `read_file(path:PathRef, max_bytes?:u32)` | `FileControl.read` → `BoundedFileContent` | RED / F2 |
| `list_directory(path:PathRef, cursor?:Cursor, limit?:u16)` | `FileControl.list` → `FilePage` | GREEN / F2 |
| `get_file_info(path:PathRef)` | `FileControl.metadata` → `FileMetadata` | GREEN / F2 |
| `calculate_dir_size(path:PathRef, max_entries?:u32)` | `FileControl.size` → `DirectorySize` | GREEN / F2 |
| `search_files(root:PathRef, query:BoundedText, mode:Enum{name,content,metadata}, cursor?:Cursor, limit?:u16)` | `FileControl.search` → `FileSearchPage` | RED when content, otherwise GREEN / F2 |
| `write_file(path:PathRef, content:BoundedContent, create_parents?:bool, expected_digest?:Digest)` | `FileControl.write` → `MutationReceipt<FileMetadata>` | YELLOW/RED by path / F2 |
| `append_file(path:PathRef, content:BoundedContent, expected_digest?:Digest)` | `FileControl.append` → receipt | YELLOW/RED by path / F2 |
| `create_directory(path:PathRef, recursive?:bool)` | `FileControl.create_directory` → receipt | YELLOW/RED by path / F2 |
| `copy_file(source:PathRef, destination:PathRef, overwrite?:bool)` | `FileControl.copy` → receipt | YELLOW/RED / F2 |
| `move_file(source:PathRef, destination:PathRef, overwrite?:bool)` | `FileControl.move` → receipt | RED / F2 |
| `rename_file(source:PathRef, destination_name:BoundedText)` | `FileControl.rename` → receipt | YELLOW/RED / F2 |
| `delete_file(path:PathRef)` | `FileControl.trash` → `MutationReceipt<TrashItem>`; this is the default delete | YELLOW/RED by path / F3 |
| `restore_trash_item(item_id:TrashItemId, resolution?:Enum{fail,rename,replace})` | `FileControl.restore_trash` → receipt | YELLOW/RED / F3 |
| `delete_permanently(path:PathRef, expected_identity:PathIdentityDigest)` | `FileControl.delete_permanently` → receipt | RED / F3 |
| `create_archive(sources:NonEmpty<PathRef>, destination:PathRef, format:ArchiveFormat)` | `FileControl.create_archive` → receipt | YELLOW/RED / F3 |
| `list_archive(path:PathRef, cursor?:Cursor, limit?:u16)` | `FileControl.list_archive` → `ArchiveEntryPage` | GREEN / F3 |
| `extract_archive(archive:PathRef, destination:PathRef, overwrite?:bool)` | `FileControl.extract_archive` → receipt | YELLOW/RED / F3 |
| `set_file_permissions(path:PathRef, mode:BoundedUnixMode)` | `FileControl.set_permissions` → receipt | RED / F3 |
| `set_file_ownership(path:PathRef, owner:ExistingLocalIdentity)` | `FileControl.set_ownership` + `BrokerOperation::SetBoundPathOwnership` → receipt | RED / F3 |
| `list_installed_apps(query?:BoundedText, cursor?:Cursor, limit?:u16)` | `ApplicationControl.list_installed` → `ApplicationPage` | GREEN / F2 |
| `open_application(app_id:AppId)` | `ApplicationControl.launch` → receipt | GREEN / F2 |
| `open_with_application(app_id:AppId, path:PathRef)` | `ApplicationControl.open_file` → receipt | YELLOW / F3 |
| `list_running_apps(cursor?:Cursor, limit?:u16)` | `ApplicationControl.list_running` → `RunningApplicationPage` | GREEN / F2 |
| `graceful_close_application(app_id:AppId, instance_id?:AppInstanceId)` | `ApplicationControl.close` → receipt | YELLOW / F2 |
| `list_processes(filter?:ProcessFilter, cursor?:Cursor, limit?:u16)` | `ProcessControl.list` → `ProcessPage` with content-free `ProcessObservation` | GREEN / F2 |
| `get_process_info(process:ProcessIdentity)` | `ProcessControl.get` → content-free `ProcessObservation` | GREEN / F2 |
| `get_process_command_metadata(process:ProcessIdentity, purpose:BoundedText)` | `ProcessControl.get_command_metadata` → `BoundedCommandMetadata` (argv only; never environment/cwd) | RED / F3 |
| `kill_process(process:ProcessIdentity, signal:Enum{term,kill})` | `ProcessControl.terminate` → receipt | RED / F2 |
| `set_process_priority(process:ProcessIdentity, nice:i8[-20..19])` | `ProcessControl.set_priority` → receipt | RED / F3 |
| `set_default_application(mime:MimeType, app_id:AppId)` | `ApplicationControl.set_default` → receipt | YELLOW / F3 |
| `manage_autostart(app_id:AppId, enabled:bool)` | `ApplicationControl.set_autostart` → receipt | YELLOW / F3 |
| `search_package(query:BoundedText, provider?:PackageProviderId, cursor?:Cursor, limit?:u16)` | `PackageControl.search` → `PackagePage` | GREEN / F2 |
| `get_package_info(package:PackageRef)` | `PackageControl.get` → `PackageObservation` | GREEN / F2 |
| `list_installed_packages(provider?:PackageProviderId, cursor?:Cursor, limit?:u16)` | `PackageControl.list_installed` → `PackagePage` | GREEN / F2 |
| `plan_package_changes(operation:Enum{install,remove,update}, packages:NonEmpty<PackageRef>)` | `PackageControl.plan` → `PackagePlan` | GREEN / F3 |
| `install_package(plan_digest:Digest)` | `PackageControl.apply_plan` + `BrokerOperation::ApplyPackagePlan` → receipt | RED / F3 |
| `uninstall_package(plan_digest:Digest)` | same closed plan operation → receipt | RED / F3 |
| `check_system_updates(provider?:PackageProviderId)` | `PackageControl.assess_updates` → `UpdateAssessment` | GREEN / F3 |
| `apply_system_updates(plan_digest:Digest)` | `PackageControl.apply_updates` + `BrokerOperation::ApplyPackagePlan` → receipt | RED / F4 |
| `get_reboot_required()` | `PackageControl.reboot_required` → `RebootRequirement` | GREEN / F3 |
| `list_storage_devices(cursor?:Cursor, limit?:u16)` | `StorageControl.list` → `StorageDevicePage` | GREEN / F3 |
| `mount_device(device:StorageDeviceId, filesystem?:FilesystemId)` | `StorageControl.mount` → receipt | RED / F3 |
| `unmount_device(device:StorageDeviceId)` | `StorageControl.unmount` → receipt | RED / F3 |
| `eject_device(device:StorageDeviceId)` | `StorageControl.eject` → receipt | RED / F3 |
| `get_storage_health(device?:StorageDeviceId)` | `StorageControl.health` → `StorageHealth` | GREEN / F3 |

### 10.2 Connectivity, audio, display, power, Bluetooth, and hardware

| Canonical tool and exact input | Provider port operation → normalized output | Risk / phase |
|---|---|---|
| `get_network_state()` | `ConnectivityControl.get_state` → `NetworkState` | RED because profile/SSID metadata / F2 |
| `get_wifi_networks(device?:NetworkDeviceId, limit?:u16)` | `ConnectivityControl.scan_wifi` → `WifiNetworkPage` | RED / F2 |
| `toggle_wifi(enabled:bool)` | `ConnectivityControl.set_wifi_enabled` → receipt | YELLOW / F2 |
| `connect_wifi(device:NetworkDeviceId, profile?:NetworkProfileId, network?:WifiNetworkId, credential?:SecretRef)` | `ConnectivityControl.connect_wifi` → receipt | RED / F2 |
| `disconnect_wifi(device:NetworkDeviceId)` | `ConnectivityControl.disconnect_wifi` → receipt | YELLOW / F3 |
| `forget_wifi(profile:NetworkProfileId)` | `ConnectivityControl.forget_profile` → receipt | RED / F3 |
| `activate_network_profile(profile:NetworkProfileId, device?:NetworkDeviceId)` | `ConnectivityControl.activate_profile` → receipt | YELLOW/RED / F3 |
| `diagnose_network(optional_target?:ValidatedHost)` | `ConnectivityControl.diagnose` → `NetworkDiagnosis` | GREEN / F4 |
| `list_vpn_profiles()` | `ConnectivityControl.list_vpn_profiles` → `VpnProfilePage` | RED / F4 |
| `set_vpn_connection(profile:NetworkProfileId, connected:bool)` | `ConnectivityControl.set_vpn` → receipt | RED / F4 |
| `get_hotspot_state(device?:NetworkDeviceId)` | `ConnectivityControl.get_hotspot_state` → `HotspotState` | RED / F5 |
| `set_hotspot(device:NetworkDeviceId, enabled:bool, profile?:NetworkProfileId, credential?:SecretRef)` | `ConnectivityControl.set_hotspot` → receipt | RED / F5 |
| `get_proxy_state()` | `ConnectivityControl.get_proxy_state` → `ProxyState` | GREEN / F5 |
| `set_proxy_profile(mode:Enum{none,automatic,manual}, profile?:RecognizedProxyProfile)` | `ConnectivityControl.set_proxy_profile` → receipt | RED / F5 |
| `list_saved_connectivity_credentials(kind?:Enum{wifi,vpn})` | `ConnectivityControl.list_saved_credentials` → metadata only | RED / F5 |
| `replace_saved_connectivity_credential(profile:NetworkProfileId, credential:SecretRef)` | `ConnectivityControl.replace_saved_credential` → receipt | RED / F5 |
| `delete_saved_connectivity_credential(profile:NetworkProfileId)` | `ConnectivityControl.delete_saved_credential` → receipt | RED / F5 |
| `get_firewall_status()` | `FirewallControl.get_status` → `FirewallState` | GREEN / F4 |
| `set_firewall_enabled(enabled:bool)` | `FirewallControl.set_enabled` + `BrokerOperation::SetFirewallEnabled` → receipt | RED when disabling, YELLOW when enabling / F4 |
| `grant_temporary_app_network_access(app_id:AppId, duration:DurationMs)` | `FirewallControl.grant_temporary` → receipt | RED / F5 |
| `get_audio_state()` | `AudioControl.get_state` → `AudioState` | GREEN / F2 |
| `set_volume(percent:Percent)` | `AudioControl.set_output_level` → receipt | YELLOW / F2 |
| `set_audio_mute(muted:bool)` | `AudioControl.set_output_mute` → receipt | YELLOW / F2 |
| `set_default_audio_output(endpoint:AudioEndpointId)` | `AudioControl.set_default_output` → receipt | YELLOW / F3 |
| `set_microphone_level(percent:Percent)` | `AudioControl.set_input_level` → receipt | RED / F3 |
| `set_microphone_mute(muted:bool)` | `AudioControl.set_input_mute` → receipt | RED when unmuting, YELLOW when muting / F3 |
| `set_default_audio_input(endpoint:AudioEndpointId)` | `AudioControl.set_default_input` → receipt | RED / F3 |
| `list_audio_streams(cursor?:Cursor, limit?:u16)` | `AudioControl.list_streams` → `AudioStreamPage` | GREEN / F5 |
| `set_application_volume(stream:AudioStreamId, percent:Percent)` | `AudioControl.set_stream_level` → receipt | YELLOW / F5 |
| `set_application_mute(stream:AudioStreamId, muted:bool)` | `AudioControl.set_stream_mute` → receipt | YELLOW / F5 |
| `set_audio_device_profile(endpoint:AudioEndpointId, profile:AudioProfileId, port?:AudioPortId)` | `AudioControl.set_profile` → receipt | YELLOW / F5 |
| `list_media_players(cursor?:Cursor, limit?:u16)` | `MediaControl.list_players` → `MediaPlayerPage` | GREEN / F5 |
| `control_media_playback(player:MediaPlayerId, action:Enum{play,pause,toggle,next,previous,stop})` | `MediaControl.control` → receipt | YELLOW / F5 |
| `get_display_state()` | `DisplayControl.get_state` → `DisplayState` | GREEN / F2 |
| `set_brightness(display?:DisplayId, percent:Percent)` | `DisplayControl.set_brightness` → receipt | YELLOW / F2 |
| `set_display_configuration(configuration:DisplayConfiguration)` | `DisplayControl.set_configuration` → receipt with rollback timer | RED / F5 |
| `confirm_display_configuration(receipt_id:ReceiptId)` | `DisplayControl.confirm_configuration` → receipt | YELLOW / F5 |
| `set_night_light(enabled:bool, temperature?:Kelvin)` | `DisplayControl.set_night_light` → receipt | YELLOW / F5 |
| `get_power_plan()` | `PowerControl.get_profile` → `PowerProfileState` | GREEN / F2 |
| `set_power_plan(profile:Enum{power_saver,balanced,performance})` | `PowerControl.set_profile` → receipt | YELLOW / F2 |
| `get_battery_status()` | `PowerControl.get_battery` → `BatteryState` | GREEN / F2 |
| `get_battery_health()` | `PowerControl.get_battery_health` → `BatteryHealth` | GREEN / F3 |
| `lock_screen()` | `PowerControl.lock` → receipt | GREEN / F2 |
| `sleep()` | `PowerControl.suspend` → accepted receipt | RED / F2 |
| `hibernate()` | `PowerControl.hibernate` → accepted receipt | RED / F2 |
| `shutdown_system(delay_seconds?:u32)` | `PowerControl.shutdown` or `schedule_shutdown` → accepted/scheduled receipt | RED / F2 |
| `reboot_system()` | `PowerControl.reboot` → accepted receipt | RED / F2 |
| `logout_session(session?:CurrentSessionId)` | `PowerControl.logout` → accepted receipt | RED / F3 |
| `cancel_scheduled_shutdown(schedule_id:ScheduleId)` | `PowerControl.cancel_scheduled_shutdown` → receipt | YELLOW / F3 |
| `set_battery_charge_thresholds(lower:Percent, upper:Percent)` | `PowerControl.set_charge_thresholds` + `BrokerOperation::SetBatteryChargeThresholds` → receipt | RED / F5 |
| `get_bluetooth_state()` | `BluetoothControl.get_state` → `BluetoothState` | RED because nearby-device metadata / F3 |
| `set_bluetooth_enabled(enabled:bool)` | `BluetoothControl.set_enabled` → receipt | YELLOW / F3 |
| `scan_bluetooth(duration_ms:DurationMs)` | `BluetoothControl.scan` → `BluetoothDevicePage` | RED / F3 |
| `pair_bluetooth_device(device:BluetoothDeviceId)` | `BluetoothControl.pair` → receipt | RED / F3 |
| `connect_bluetooth_device(device:BluetoothDeviceId)` | `BluetoothControl.connect` → receipt | YELLOW/RED / F3 |
| `disconnect_bluetooth_device(device:BluetoothDeviceId)` | `BluetoothControl.disconnect` → receipt | YELLOW / F3 |
| `set_bluetooth_trust(device:BluetoothDeviceId, trusted:bool)` | `BluetoothControl.set_trust` → receipt | RED / F3 |
| `remove_bluetooth_device(device:BluetoothDeviceId)` | `BluetoothControl.remove` → receipt | RED / F3 |
| `get_hardware_sensors(cursor?:Cursor, limit?:u16)` | `HardwareControl.get_sensors` → `SensorPage` | GREEN / F5 |
| `get_firmware_status()` | `FirmwareAwareness.get_status` → `FirmwareStatus` | GREEN / F5 |

### 10.3 Health, clipboard, notifications, search, secrets, printing, scanning, backup, privacy, and automation

| Canonical tool and exact input | Provider port operation → normalized output | Risk / phase |
|---|---|---|
| `get_cpu_usage()` | `SystemHealthControl.cpu` → `CpuState` | GREEN / F2 |
| `get_memory_info()` | `SystemHealthControl.memory` → `MemoryState` | GREEN / F2 |
| `get_disk_space()` | `SystemHealthControl.filesystems` → `FilesystemPage` | GREEN / F2 |
| `get_system_uptime()` | `SystemHealthControl.uptime` → `UptimeState` | GREEN / F2 |
| `get_gpu_info()` | `SystemHealthControl.gpu` → `GpuState` | GREEN / F2 |
| `check_system_health()` | `SystemHealthControl.summary` → `HealthSummary` | GREEN / F2 |
| `diagnose_system(scope?:HealthDomain)` | `SystemHealthControl.diagnose` → `SystemDiagnosis` | GREEN / F4 |
| `get_system_logs(query:LogQuery)` | `SystemHealthControl.query_logs` → `LogPage` | RED / F4 |
| `run_recovery_recipe(recipe_id:RecoveryRecipeId, expected_plan_digest:Digest)` | `SystemHealthControl.run_recipe` → multi-step receipt | RED / F4 |
| `get_clipboard(max_bytes?:u32, mime?:AllowedMime)` | `ClipboardControl.get_current` → `ClipboardPayload` | RED / F2 |
| `set_clipboard(payload:BoundedClipboardPayload)` | `ClipboardControl.set_current` → receipt | RED / F2 |
| `get_clipboard_history(cursor?:Cursor, limit?:u16)` | `ClipboardControl.history` → encrypted-history metadata/payload page | RED / F4 |
| `clear_clipboard_history()` | `ClipboardControl.clear_history` → receipt | RED / F4 |
| `configure_clipboard_history(enabled:bool, ttl_seconds?:u32, max_items?:u16, allowed_mimes?:NonEmpty<AllowedMime>)` | `ClipboardControl.configure_history` → receipt | RED / F4 |
| `send_notification(title:BoundedText, body:BoundedText, urgency?:NotificationUrgency, actions?:Bounded<NotificationAction>)` | `NotificationControl.send` → receipt | YELLOW / F2 |
| `get_notification_state()` | `NotificationControl.get_state` → `NotificationState` | GREEN / F4 |
| `set_do_not_disturb(enabled:bool)` | `NotificationControl.set_dnd` → receipt | YELLOW / F4 |
| `search_desktop(query:BoundedText, scope?:SearchScopeId, cursor?:Cursor, limit?:u16)` | `SearchControl.search` → `DesktopSearchPage` | RED when content-indexed / F4 |
| `get_search_scope()` | `SearchControl.get_scope` → `SearchScope` | GREEN / F4 |
| `configure_search_scope(roots:NonEmpty<PathRef>, exclusions?:Bounded<PathRef>)` | `SearchControl.configure_scope` → receipt | RED / F4 |
| `rebuild_search_index(scope?:SearchScopeId)` | `SearchControl.rebuild` → accepted job receipt | RED / F4 |
| `list_secret_references(purpose?:SecretPurpose, cursor?:Cursor, limit?:u16)` | `CredentialStore.list_metadata` → secret metadata only | RED / F3 |
| `store_secret(purpose:SecretPurpose, scope:SecretScope, protected_input:ProtectedInputHandle)` | `CredentialStore.store` → metadata receipt | RED / F3 |
| `replace_secret(secret:SecretRef, protected_input:ProtectedInputHandle)` | `CredentialStore.replace` → metadata receipt | RED / F3 |
| `delete_secret(secret:SecretRef)` | `CredentialStore.delete` → receipt | RED / F3 |
| `list_printers(cursor?:Cursor, limit?:u16)` | `PrintControl.list` → `PrinterPage` | GREEN / F4 |
| `get_print_queue(printer?:PrinterId, cursor?:Cursor, limit?:u16)` | `PrintControl.queue` → `PrintJobPage` | GREEN / F4 |
| `configure_printer(discovered:DiscoveredPrinterId, options:ReviewedPrinterOptions)` | `PrintControl.configure` + `BrokerOperation::ConfigureDiscoveredPrinter` → receipt | RED / F4 |
| `print_file(printer:PrinterId, path:PathRef, options?:ReviewedPrintOptions)` | `PrintControl.submit` → receipt | RED / F4 |
| `cancel_print_job(job:PrintJobId)` | `PrintControl.cancel_owned` → receipt | RED / F4 |
| `list_scanners(cursor?:Cursor, limit?:u16)` | `ScanControl.list_scanners` → `ScannerPage` | GREEN / F5 |
| `scan_document(scanner:ScannerId, destination:PathRef, format:ScanFormat, resolution_dpi:BoundedDpi, pages?:u16)` | `ScanControl.scan_document` → accepted/verified receipt | RED / F5 |
| `get_backup_status(provider?:BackupProviderId)` | `BackupIntegration.get_status` → `BackupStatus` | GREEN / F5 |
| `start_backup(provider:BackupProviderId, plan_digest:Digest)` | `BackupIntegration.start` → accepted job receipt | RED / F5 |
| `plan_backup_restore_handoff(provider:BackupProviderId, snapshot:BackupSnapshotId, destination?:PathRef)` | `BackupIntegration.plan_restore_handoff` → `TrustedHandoffPlan` | RED / F5 |
| `get_privacy_state()` | `PrivacyControl.get_state` → `PrivacyState` | RED / F4 |
| `set_privacy_control(control:RecognizedPrivacyControl, enabled:bool)` | `PrivacyControl.set_control` + `BrokerOperation::SetPrivacyControl` when needed → receipt | RED / F4 |
| `list_scheduled_tasks(cursor?:Cursor, limit?:u16)` | `AutomationControl.list` → `AutomationPage` | GREEN / F2 |
| `create_scheduled_task(schedule:TypedSchedule, action:CanonicalCapabilityInvocation)` | `AutomationControl.create` → receipt | risk of contained action, never below YELLOW / F4 |
| `modify_scheduled_task(task_id:AutomationId, expected_revision:Revision, patch:TypedAutomationPatch)` | `AutomationControl.update` → receipt | risk of contained action / F4 |
| `delete_scheduled_task(task_id:AutomationId)` | `AutomationControl.delete` → receipt | RED / F4 |
| `run_workflow(workflow_id:WorkflowId, expected_revision:Revision)` | `AutomationControl.run` → multi-step receipt | aggregate action risk / F4 |
| `list_workflows(cursor?:Cursor, limit?:u16)` | `AutomationControl.list_workflows` → `WorkflowPage` | GREEN / F4 |

### 10.4 Normative operation metadata and DTO rules

The rows in §§10.1–10.3 and the rules below form one closed manifest; neither is illustrative. Each row has stable ID `os.<canonical_tool_name>` and exactly one `OperationContract`:

```rust
pub struct OperationContract {
    pub id: OperationId,
    pub tool_name: &'static str,
    pub input_schema: ClosedSchemaId,
    pub output_schema: ClosedSchemaId,
    pub provider_operation: ProviderOperationId,
    pub target: TargetPolicy,
    pub resume: ResumePolicy,
    pub resources: ResourceDerivationId,
    pub risk: RiskFunctionId,
    pub verification: VerificationClass,
    pub rollback: RollbackClaim,
    pub redaction: RedactionProfileId,
    pub requirement: RequirementId,
    pub task: TaskId,
    pub oracle: TestOracleId,
    pub phase: PhaseId,
}
```

The following total rules supply the metadata omitted from the compact row display; there is no implementation-selected default:

| Field | Exact rule |
|---|---|
| `target` | `HostLocalOnly` for every row. Remote, VM, container, and extension-local targets are schema-invalid. |
| `resume` | Reads are `ReevaluateRead`; unchanged mutations return before approval; YELLOW/RED mutations are `RevalidateDurableDecision`; GREEN mutations are `ReevaluateFresh`; accepted session-ending actions are `NeverResumeAfterDispatch`. Every resumed mutation gets a new grant/admission and never reuses a provider dispatch. |
| `resources` | Reads use the domain read key. Mutations use the canonical write keys below. Multi-target operations take the sorted union. Missing stable identity is validation failure, never a global wildcard. |
| `verification` | Reads use `None`. Synchronous mutation rows use `FreshAuthoritativeObservation` of their declared output observation. Percentage controls additionally use the §9 tolerance. Session-ending, asynchronous job, and index-rebuild rows use `ProviderAcceptanceThenJobObservation`. Multi-step rows use `PerStepThenAggregate`. |
| `rollback` | Every mutation row resolves to exactly one closed `RollbackClaim`: `Automatic`, `UserRequestable`, `CompensationOnly`, or `None`, taken from the §13.1 per-operation table. No row uses a conditional phrase and no adapter may infer an inverse. |
| `redaction` | Field-class rules below are exhaustive. A field without a classification fails contract construction. |
| trace links | Each row names exactly one `RequirementId` (a specific `OSC-nnn` acceptance-criterion owner), one `TaskId` (the single implementing task ID such as `3.5`, never a phase or range), and one `TestOracleId` `oracle.<tool_name>`. Reverse-orphan validation rejects ranges, phases, placeholders, or more than one of each. §22 range mappings are informational only and are not the machine trace source. |

The compact-row output word `receipt` is resolved by this total table; a row never leaves its output type to implementation choice:

| Output phrase in rows | Exact result type |
|---|---|
| `receipt` for a mutation whose port has a matching getter observation `O` | `MutationReceipt<O>` |
| `set_hotspot`, `set_proxy_profile` | `MutationReceipt<HotspotState>`, `MutationReceipt<ProxyState>` |
| file mutations (`write_file`…`extract_archive`, permissions/ownership) | `MutationReceipt<FileMetadata>` except `delete_file`→`MutationReceipt<TrashItem>` and `list_archive`/`create_archive` observations named in §10.1 |
| application/process mutations | `MutationReceipt<ApplicationState>` or `MutationReceipt<ProcessObservation>` for the acted identity |
| `accepted receipt` (suspend/hibernate/reboot/logout/shutdown/session-ending) | `MutationReceipt<SessionEndAcceptance>` in the `Accepted` state only |
| `scheduled receipt` / `schedule_shutdown` / `cancel_scheduled_shutdown` | `MutationReceipt<ShutdownSchedule>` |
| `confirm_display_configuration` | `MutationReceipt<DisplayState>` |
| `accepted job receipt` (`rebuild_search_index`, `start_backup`) | `MutationReceipt<JobHandle>` in `Accepted` |
| `accepted/verified receipt` (`scan_document`) | `MutationReceipt<ScanArtifact>` (`Accepted` while scanning, `Verified` after staged commit) |
| `multi-step receipt` (`run_recovery_recipe`, `run_workflow`) | `MutationReceipt<StepAggregate>` with child receipts |
| `metadata only` (credential/secret list) | `Page<ConnectivityCredentialMetadata>` or `Page<SecretMetadata>`; never a value |
| metadata receipts (`store_secret`, `replace_secret`, credential replace/delete) | `MutationReceipt<SecretMetadata>` / `MutationReceipt<ConnectivityCredentialMetadata>` |

Slash-risk rows are resolved by these total functions, never provider discretion:

| Row | Resolved risk rule |
|---|---|
| `search_files` | RED iff `mode=content`, else GREEN |
| `write_file`/`append_file`/`create_directory`/`copy_file`/`rename_file`/`delete_file`/`restore_trash_item`/`create_archive`/`extract_archive` | canonical-path scope classifier: RED for protected/system-adjacent scopes, YELLOW for ordinary user-data scopes; computed before grant issuance |
| `activate_network_profile` | RED iff the target profile is a VPN or changes the default route/gateway; else YELLOW |
| `connect_bluetooth_device` | RED iff the device class is input/audio-input or the device is unpaired/untrusted; else YELLOW |
| `set_firewall_enabled` | YELLOW when enabling, RED when disabling |
| `set_microphone_mute` | YELLOW when muting, RED when unmuting |
| `create_scheduled_task`/`modify_scheduled_task`/`run_workflow` | `max(YELLOW, contained/step action risk)` |

Each resolver reads only closed schema fields or the reviewed classifier output, so risk is deterministic at admission.

Canonical write-resource derivation is exact by provider port:

| Port/domain | Canonical write resource |
|---|---|
| `FileControl` | canonical path identity for every source and destination; archive operations also use destination subtree |
| `ApplicationControl` | `application/<AppId or AppInstanceId>` plus path identity for open-with |
| `ProcessControl` | `process/<pid>/<start_time>` |
| `PackageControl` | `package-db/<provider>` and `update-manager/<provider>` for updates |
| `StorageControl` | `storage/<StorageDeviceId>/<FilesystemId-or-none>` |
| `ConnectivityControl` | sorted `network-device/<id>` and `network-profile/<id>`; radio operations use `network-radio/<adapter>` |
| `FirewallControl` | `firewall/<provider>`; temporary grants additionally use `application/<AppId>` |
| `AudioControl` | `audio-endpoint/<id>` or `audio-stream/<id>`; global default changes also use `audio-default/<input-or-output>` |
| `MediaControl` | `media-player/<MediaPlayerId>` |
| `DisplayControl` | `display/<DisplayId>`; topology uses `display-topology/session` |
| `PowerControl` | `power-profile/system`, `power-session/current`, `shutdown-schedule/<id>`, or `battery-threshold/<adapter>` as named by the request |
| `BluetoothControl` | `bluetooth-adapter/<id>` plus `bluetooth-device/<id>` when present |
| clipboard/notification | `clipboard/current`, `clipboard/history`, `notification-state/session` |
| search/automation | `search-scope/<id>`, `search-index/<id>`, `automation/<id>`, or `workflow/<id>` |
| secret/print/scan/backup/privacy | stable `secret/<ref>`, `printer/<id>`, `scanner/<id>`, `backup/<provider>`, or `privacy/<control>`; output paths add file resources |

Strict schema catalog rules:

- Every request object is the exact object printed in its row, has `additionalProperties:false`, and uses no implicit positional/default field. Optional booleans default to `false` only where the row explicitly names behavior whose absence is false; all other optional fields default to `None`.
- IDs are opaque NFC strings of 1–128 UTF-8 bytes and are never resolved from human labels for mutation. `BoundedText` is 1–1024 bytes; purpose text is 1–256 bytes; `Digest` is lowercase SHA-256 hex; cursors are opaque authenticated values up to 512 bytes. Collection defaults and maxima come from §19 configuration and are always capped by compile-time hard maxima.
- `Page<T>` is `{items:BoundedVec<T>, next_cursor?:Cursor, truncated:bool}`. `MutationReceipt<O>` is exactly the private-state receipt projected through the §4 envelope plus operation-owned top-level compatibility fields. The word `receipt` in compact rows means `MutationReceipt<O>` where `O` is the normalized observation returned by the same port's getter; if no getter exists, the row must name a dedicated result type and may not use generic JSON.
- Every state/observation DTO has `{identity, revision, availability, fields}` semantics: stable typed identity, monotonic provider revision when available, explicit `Available|Degraded|Unavailable`, and closed typed fields. Missing provider values are `Option`/`UnavailableReason`, never zero/empty invention. Unknown future provider enum values normalize to `Unknown(BoundedText)` only where the schema declares that variant.
- Content-bearing fields use bounded byte/text wrappers and are `Content`; secret payload wrappers are non-`Serialize`, zeroizing, and `Secret`; stable IDs and private labels are `SensitiveMetadata`; booleans, percentages, enum settings, counts, and digests are `PublicLocal` unless a row explicitly marks the read RED. No environment map, raw D-Bus value, command output, arbitrary JSON, or unbounded string appears in a canonical DTO.
- Risk functions are closed: `search_files` is RED iff `mode=content`; file mutation risk is computed by the reviewed canonical-path scope classifier before grant issuance; firewall enable is YELLOW and disable RED; microphone mute is YELLOW and unmute RED; automation risk is `max(YELLOW, contained_action_risk)` and workflows use the maximum step risk. All other rows use the single risk printed in the row. A slash-separated printed risk invokes only the named rule here, never provider discretion.

Process schemas are frozen separately because privacy affects admission. `ProcessFilter` contains only optional state/owner/app identity and resource-threshold fields; it has no command-content flag. `ProcessObservation` contains identity, bounded redacted executable label, executable digest, owner reference, state, CPU, memory, start time, and `CommandMetadataState`; environment, cwd, and argv are absent. `BoundedCommandMetadata` contains bounded argv elements plus executable/argv digests and truncation state, never environment or cwd, and is returned only by RED `get_process_command_metadata`.

Task 0.1 records this closed manifest and a legacy-difference report without changing runtime code. Task 1.2 implements it as strict `ToolContractMetadata` and rejects duplicate, missing, placeholder, reverse-orphan, unclassified-field, and non-total risk/resource entries. Every domain task supplies the concrete closed DTO structs and oracle fixture named by its already-frozen manifest entry; changing a field, enum, default, target, risk, resource, verification, rollback, redaction, trace link, or oracle requires a design amendment, not an implementation choice.

No other native OS-control tool name or operation is in v1/v2 scope. `execute_bash`, provider-specific binaries, bus names, raw object paths, device nodes, secret values, arbitrary service/unit names, generic broker calls, and free-form recovery/automation steps are not parameters of these schemas. Task 0.1 may correct a discovered spelling only through an explicit spec amendment before implementation; it may not invent aliases or defer naming decisions.

## 11. Risk and Confirmation Matrix

| Operation class | Default risk | Confirmation | Rollback |
|---|---:|---|---|
| Public/non-sensitive local observation | GREEN | None | N/A |
| Privacy-sensitive read (including clipboard content, private logs/history, nearby-device identity, protected search content) | RED | Always on redacted purpose/scope; never show content in approval | N/A |
| Privacy-sensitive device/control action (including microphone unmute/activation or privacy weakening) | RED | Always | Per §13.1 |
| Reversible user setting | YELLOW | Usually none after clear request | Per §13.1 |
| Device/profile association | YELLOW/RED | Pair/remove/forget requires confirmation | Per §13.1 |
| Package install/update/remove | RED | Always on exact plan | Per §13.1 (`None`) |
| Permanent delete/process kill | RED | Always | Per §13.1 (`None`) |
| Firewall disable/privacy weakening | RED | Always | Per §13.1 |
| Suspend/hibernate/logout/shutdown/reboot | RED except lock | Always according to action policy | Per §13.1 (`None`) |
| BLACK administration | BLACK | Not offered | None |

This matrix is a coarse class summary; §10.4 risk resolvers and §13.1 rollback claims are the machine-checkable per-operation source.

## 12. Structured Command and Privileged Broker

A provider may create an internal `StructuredCommandRequest` only while borrowing a valid `AdmittedMutationContext`; a grant alone is insufficient. The executor validates the private mutation permit before dispatch. The request is host-only and binds capability ID, grant ID, resource-set digest, audit-admission ID, resolved trusted absolute executable identity, exact argv digest, allowlisted environment, working-directory policy, deadline, cancellation, output/line bounds, locale, and redaction map. User/model input cannot choose the executable or add argv positions outside the capability adapter. No shell interpreter, remote/VM/container target, inherited secret environment, or raw command string exists in this contract.

The existing command `PolicyGate` remains defense in depth subordinate to `ExecutionGate`: it validates that the fixed executable/argv matches the already-issued typed grant and may block it, but cannot request a second approval, add a custom rule that substitutes for action approval, or broaden authority. Generic `execute_bash`/code execution remains separately governed and cannot consume an OS-control grant. Timeout or cancellation after dispatch is represented as an uncertain `ApplyOutcome`; no second provider is attempted.

The broker is a small separate process or service activated through Polkit. It accepts only versioned typed requests and validates caller identity, action grant, operation, bounded parameters, target identity, and expiry. It returns structured receipts, not raw command output.

Allowed broker operations are the following closed enum only, used only when the stable session/system service cannot perform the operation with its own Polkit flow:

```rust
pub enum BrokerOperation {
    ApplyPackagePlan {
        provider: PackageProviderId,
        approved_plan_digest: Digest,
        transaction: BoundedPackageTransaction,
    },
    SetBoundPathOwnership {
        path: BrokerBoundPath,
        owner: ExistingLocalIdentity,
        follow_symlinks: False,
    },
    SetFirewallEnabled {
        provider: FirewallProviderId,
        enabled: bool,
    },
    SetPrivacyControl {
        control: RecognizedPrivacyControl,
        enabled: bool,
    },
    ConfigureDiscoveredPrinter {
        printer: DiscoveredPrinterId,
        options: ReviewedPrinterOptions,
    },
    SetBatteryChargeThresholds {
        adapter: ChargeThresholdAdapterId,
        lower_percent: BoundedPercent,
        upper_percent: BoundedPercent,
    },
}
```

The wire protocol is length-prefixed canonical CBOR with a 64 KiB request/response maximum, protocol version `1`, one request per authenticated local connection, and a broker-enforced deadline no greater than the grant deadline. Unknown versions, operation tags, required fields, duplicate map keys, non-canonical encodings, and trailing frames fail before dispatch.

```rust
pub struct BrokerRequestV1 {
    protocol_version: ProtocolVersion<1>,
    request_id: BrokerRequestId,
    caller_binding: CallerChannelBindingDigest,
    operation: BrokerOperation,
    grant_id: GrantId,
    action_hash: Digest,
    parameter_hash: Digest,
    target_hash: Digest,
    resource_set_digest: Digest,
    audit_admission_id: AuditAdmissionId,
    operation_digest: Digest,
    nonce: GrantNonce,
    expires_at: SystemTime,
}

pub enum BrokerResponseV1 {
    NotDispatched {
        binding: BrokerResponseBinding,
        error: BrokerPreDispatchError,
    },
    Dispatched {
        binding: BrokerResponseBinding,
        outcome: BrokerDispatchOutcome,
    },
}

pub struct BrokerResponseBinding {
    protocol_version: ProtocolVersion<1>,
    request_id: BrokerRequestId,
    caller_binding: CallerChannelBindingDigest,
    grant_id: GrantId,
    nonce: GrantNonce,
    expires_at: SystemTime,
    action_hash: Digest,
    parameter_hash: Digest,
    target_hash: Digest,
    resource_set_digest: Digest,
    audit_admission_id: AuditAdmissionId,
    operation_digest: Digest,
}

pub enum BrokerPreDispatchError {
    AuthenticationFailed,
    BindingMismatch,
    ReplayDetected,
    Expired,
    UnsupportedVersion,
    UnsupportedOperation,
    InvalidParameters,
    StalePlan,
    StaleTargetIdentity,
    UnsupportedAdapter,
    PolkitDenied,
    TimeoutBeforeDispatch,
}

pub enum BrokerDispatchOutcome {
    Applied {
        receipt_digest: Digest,
        evidence: BoundedBrokerEvidence,
    },
    Uncertain {
        receipt_digest: Option<Digest>,
        cause: UncertainEffectCause,
        evidence: BoundedBrokerEvidence,
    },
    PartiallyApplied {
        receipt_digest: Option<Digest>,
        completed_steps: NonEmptyBoundedVec<SafeStepId>,
        failed_step: SafeStepId,
        cause: PartialEffectCause,
        evidence: BoundedBrokerEvidence,
    },
}
```

`CallerChannelBindingDigest` is derived from peer credentials and the authenticated local connection by the KRIA client transport; it is not a self-asserted username or PID. The broker independently derives the same value and rejects a request mismatch before Polkit or dispatch. `BrokerResponseBinding` must byte-for-byte echo every request authority/binding field after canonical decoding, including caller binding and expiry; KRIA rejects the response before interpreting its outcome if any binding differs. Replay storage is keyed by caller binding plus nonce and persists through the request expiry window. A replay never dispatches: if a completed response is cached, the broker returns that identical bound response; otherwise it returns `ReplayDetected`. `BoundedBrokerEvidence` contains only operation-specific normalized state-query fields, provider identity, and evidence digest—never stdout, stderr, command text, D-Bus payloads, secrets, or free-form errors. `NotDispatched` maps to `OsControlError`; once dispatch may have occurred the broker can return only `Dispatched`, whose three variants map directly to the narrow §4 dispatch types. Transport loss after broker dispatch is `Uncertain`, never a pre-dispatch error or fallback trigger.

`BrokerBoundPath` contains the approved canonical path plus expected device/inode/owner identity and must match the grant/resource digest immediately before operation. Package transactions are decoded from the approved normalized plan and cannot carry executable/argv/repository/key data. Printer options are a closed set; privacy and threshold controls are enums discovered by capability probe. Mount/eject uses UDisks2’s own typed Polkit authorization and is not reconstructed in this broker. No generic command, shell, arbitrary file write, arbitrary D-Bus call, raw device node, service/unit name, firmware method, repository mutation, or run-as-root variant exists. Unknown protocol variants fail closed.

Provider flow:

```text
KRIA approval → signed/nonce-bound broker request → Polkit → fixed operation → state query → receipt
```

A denied Polkit request remains denied. KRIA does not request, capture, or cache sudo passwords.

## 13. Verification, Evidence, and Rollback

Each mutation defines:

```rust
pub struct Postcondition<O> {
    pub desired: O,
    pub comparator: ComparatorKind,
    pub tolerance: Option<Tolerance>,
    pub deadline_ms: u64,
    pub accepted_without_observation: bool,
}
```

Evidence source authority by domain:

1. Authoritative service state/property or filesystem metadata.
2. Independent provider query for the same normalized state.
3. Structured command query with unambiguous parse.
4. User attestation only for display visibility or inherently subjective outcomes.
5. No evidence: `Unverified`.

```rust
pub struct VerificationEvidence<O> {
    pub source: OsEvidenceSource,
    pub reliability: VerificationReliability,
    pub provider: ProviderId,
    pub normalized_observation: RedactedObservation<O>,
    pub observation_digest: Digest,
    pub provider_revision: Option<SafeRevision>,
    pub observed_at: SystemTime,
    pub freshness_ms: u64,
    pub ambiguous: bool,
    pub safe_details: Option<SafeText>,
}
```

The OS evidence ranking is separate from GUI leaf evidence: authoritative service/property and filesystem metadata outrank independent normalized provider observations, which outrank structured-command query output. Generic shell output is never authoritative OS-state evidence. `safe_details` is redacted at construction and cannot contain raw stdout/stderr, object paths, payloads, secrets, or untrusted control characters.

A provider cannot fall back to another mutator after uncertain apply. It may use another read provider for verification only when semantics are equivalent and evidence records the source.

### 13.1 Per-operation rollback claims

Every mutation row's `RollbackClaim` is fixed here; there is no "where exact" discretion. An operation whose prior state is not reliably restorable is `None` rather than conditional.

| RollbackClaim | Operations |
|---|---|
| `Automatic` | `set_display_configuration` (confirmation timer); staged `write_file`/`append_file`/`create_directory`/`copy_file`/`rename_file`/`create_archive`/`extract_archive` commit failure cleanup |
| `UserRequestable` | `set_volume`, `set_audio_mute`, `set_default_audio_output`, `set_microphone_level`, `set_microphone_mute`, `set_default_audio_input`, `set_application_volume`, `set_application_mute`, `set_audio_device_profile`, `set_brightness`, `set_night_light`, `toggle_wifi`, `activate_network_profile`, `set_power_plan`, `set_process_priority`, `connect_bluetooth_device`, `set_bluetooth_trust`, `set_bluetooth_enabled`, `set_default_application`, `manage_autostart`, `set_privacy_control`, `set_firewall_enabled`, `set_hotspot`, `set_proxy_profile`, `set_battery_charge_thresholds`, `set_do_not_disturb`, `configure_printer`, `confirm_display_configuration`, `delete_file` |
| `CompensationOnly` | `move_file`, `run_recovery_recipe`, `run_workflow`, credential replace/delete coordination in `replace_saved_connectivity_credential`/`delete_saved_connectivity_credential` |
| `None` | `kill_process`, `delete_permanently`, `restore_trash_item`, `install_package`, `uninstall_package`, `apply_system_updates`, `connect_wifi`, `disconnect_wifi`, `forget_wifi`, `set_vpn_connection`, `grant_temporary_app_network_access`, `pair_bluetooth_device`, `disconnect_bluetooth_device`, `remove_bluetooth_device`, `mount_device`, `unmount_device`, `eject_device`, `open_application`, `open_with_application`, `graceful_close_application`, `set_clipboard`, `send_notification`, `control_media_playback`, `store_secret`, `replace_secret`, `delete_secret`, `print_file`, `cancel_print_job`, `scan_document`, `start_backup`, `create_scheduled_task`, `modify_scheduled_task`, `delete_scheduled_task`, `configure_search_scope`, `rebuild_search_index`, `clear_clipboard_history`, `configure_clipboard_history`, `set_file_permissions`, `set_file_ownership`, `cancel_scheduled_shutdown`, and every session-ending accepted action (`sleep`, `hibernate`, `shutdown_system`, `reboot_system`, `logout_session`, `lock_screen`) |

Each bucket cell lists only its member tool names; no bucket cell references a tool that belongs to another bucket, so a validator may extract membership by backticked token. `delete_file` is `UserRequestable` because Trash is reversible through the separate `restore_trash_item` recovery operation, which is itself `None`. A `set_*` operation listed as `UserRequestable` advertises rollback only when the provider actually captured exact prior state; if prior state was unavailable it downgrades the specific receipt to `RollbackAvailability::Unavailable` while keeping the row's static claim for schema purposes.

## 14. Audit and Redaction Design

Hard-migrate `safety::AuditLogger` into the sole fallible OS-action audit authority; the best-effort subprocess channel is deleted for OS control. The current infallible `log`/in-place `update_result` contract is not retained because ignored insertion errors cannot fail closed and mutated completion columns are not covered by the original row hash.

Audit is append-only, integrity-linked, and models one logical action:

1. `admit_action` performs a SQLite transaction and appends one redacted `Admission` record before the first provider observation for the requested action. It returns one non-cloneable `AuditAdmissionToken` bound to session/action/parameters/target/capability/prospective-resource digest and recovery key, but not to a not-yet-issued grant. The runtime lends an observation-only `ObservationAuditAuthority` from that token to `HostExecutionContext`; it cannot authorize mutation. The same admission/token remains associated with a later no-op or mutation; pre-observation never creates a second action. Admission failure returns `AuditUnavailable` before provider access for mutations and privacy-sensitive reads.
2. `append_terminal` idempotently appends the action's sole terminal `Completion` or `Incident` record containing `parent_admission_id` and admission digest. A unique partial index on terminal `parent_admission_id` enforces at most one terminal. Concurrent/replayed appenders read and return the winning terminal when their canonical terminal digest matches; a mismatch is an integrity incident and keeps audit unhealthy.
3. If terminal append fails, the durable admission remains detectably incomplete and the receipt reports `PendingRecovery`. No durable incident ID is invented. The runtime writes no mutable completion fields, blocks later automatic mutations, and never repeats provider dispatch.
4. At composition startup and before audit health can reopen, `reconcile_incomplete_admissions(limit, cursor)` performs a bounded scan. It reconstructs the safe terminal summary from a durable recovery payload committed before dispatch where possible; otherwise it appends `OutcomeUnknownAfterCrash`. Reconciliation uses the same idempotent terminal key and bounded retry policy. Manual/read-only operation may continue only according to policy while audit is unhealthy.
5. Rollback has its own admission/terminal pair linked to the original receipt and still passes policy, resources, verification, and audit.
6. Chain verification covers every admission, completion, incident, recovery, and rollback field; bounded queries require explicit limit/cursor and enforce a hard maximum.

```text
Admitted(no terminal) ──append succeeds──────────────▶ Terminal(recorded)
        │
        ├─append persistence interruption───────────▶ Incomplete(pending_recovery)
        │                                                  │
        └─process exit/crash────────────────────────────────┤
                                                           └─bounded idempotent reconcile──▶ Terminal(recorded)
```

Record fields include:

```text
record_kind, parent_admission_id, recovery_key, correlation_id, session_id,
action_hash, parameter_hash, target_hash, decision_id, risk, provider_id,
lifecycle, before_digest, after_digest, provider_receipt_digest,
verification_source, verification_reliability, rollback_available,
error_or_incident_code, capability_snapshot_revision, duration_ms,
prev_hash, row_hash
```

The migration adds an admission/terminal kind check, a unique terminal-parent index, and an indexed incomplete-admission query. `AuditAdmissionToken` binds admission ID/digest, action/parameter/target/resource digests, capability revision, and recovery key. Recovery payloads contain only `SafeReceiptSummary` fields and redacted digests; secret/content values never become recovery material.

Parameter content is represented by redacted structured metadata plus a digest. HITL receives the same redacted projection, never the raw parameter object. Redaction classes:

| Class | Examples | Audit behavior |
|---|---|---|
| Public local | percentage, boolean, power profile | Store normalized value |
| Sensitive metadata | SSID, device/app/file names | Hash or truncate according to policy |
| Content | clipboard, notification body, file contents, logs | Store only type/size/digest |
| Secret | passwords, tokens, passkeys, VPN material | Never serialize; store purpose/reference digest only |
| Prohibited | raw credential/export, arbitrary root request | Reject before audit parameters |

Raw command strings, stdout/stderr, D-Bus payloads, approval secrets, and untrusted provider text never enter durable audit. Terminal-persistence interruption returns the truthful in-memory receipt with `AuditCompletionState::PendingRecovery`, marks audit health unavailable, and halts further automatic mutation until bounded reconciliation records the sole terminal; it does not return a digest-only error or claim that a terminal incident was already durable.

## 15. Prompt, Tool, and Transport Integration

Routing updates occur in existing router/fallback/turn-gate modules. Tool definitions remain OpenAI-compatible function schemas. Each tool declares:

- Canonical name and description
- Strict parameters with unknown fields denied
- Risk and confirmation policy
- Host-only target policy
- Resume capability
- Resource requirements
- Capability availability key
- Sensitive parameter paths
- Result adapter

These fields live in one `ToolContractMetadata` owned by `ToolDef`; resume metadata is not kept in a parallel map. Its parameter schema is a typed JSON-schema value supporting nested objects, enums, bounds, and `additionalProperties: false` at every closed object. `ToolRegistry::register` returns a typed error on duplicate definition, handler, alias, or inconsistent metadata instead of overwriting. The registry contract snapshot is the source for router/fallback/prompt construction and availability filtering. Composition injects `Arc<OsControlRuntime>` into OS handlers; raw `HostOsControl` remains private behind the runtime. Missing runtime/provider composition produces the frozen `Unavailable` envelope before admission and never exposes `LocalEnvironment` as an OS fallback.

For OS actions, `HitlGateway::ApprovalRequest.parameters: serde_json::Value` and formatted raw-parameter descriptions are replaced by a typed projection:

```rust
pub struct ApprovalProjection {
    pub request_id: ApprovalRequestId,
    pub decision_id: DecisionId,
    pub action_label: SafeText,
    pub risk: RiskLevel,
    pub purpose: SafeText,
    pub affected_resources: BoundedVec<SafeResourceSummary>,
    pub parameter_digest: Digest,
    pub redacted_fields: BoundedMap<SafeField, RedactedApprovalValue>,
    pub rollback: RollbackAvailability,
    pub expires_at: SystemTime,
}
```

The projection is built by the shared redaction authority before HITL registration. `safety/hitl.rs`, `agent/gui_wiring.rs`, and `agent/resume_executor.rs` never receive raw OS parameter JSON or construct descriptions with `params`. An approval response becomes authoritative only after its matching SQLite `InteractionDecision` transition commits; persistence failure denies execution and emits the existing failure/approval events without a grant.

No new Tauri command is required. Existing WebSocket messages remain:

- inbound: `approve`, `deny`, `cancel`
- outbound: `tool_start`, `tool_progress`, `tool_end`, `approval_required`, `approval_result`, `hitl_ack`

Approval presentation remains non-authoritative. Additive tool result metadata is nested to avoid frontend field collision.

## 16. Search, History, and Durable State

SQLite remains authority for KRIA-owned configuration and history. Separate bounded tables may store:

- OS action receipts/audit extensions
- Search index roots and projection records
- Clipboard history only when opted in
- Notification correlation/history only when configured
- Typed automation definitions
- KRIA-created firewall grant ownership/expiry
- Display rollback pending state

Filesystem and OS services remain authority for actual OS state. Search indexes and cached capability snapshots are rebuildable projections.

Sensitive history payloads require encryption using a key reference from Secret Service. If the key is unavailable, history features fail closed while current clipboard/notification operations may continue according to policy.

## 17. Failure and Degraded-State Rules

| Failure | Required behavior |
|---|---|
| Provider absent | Return operation-level unavailable with safe remediation |
| D-Bus service restarts | Invalidate probe/object paths; retry reads once, never uncertain mutation |
| Polkit denied | Return permission denied; no sudo/password fallback |
| Apply timeout | Mark mutation uncertain; observe only; do not mutate via fallback |
| Verification contradiction | Return failed; rollback only if declared safe |
| Device disappears | Return target changed/partial; release leases |
| Approval expires | Discard grant/decision authority; return to preflight and re-evaluate admission, capability, risk and resources; never replan or mutate inside verification |
| Resource busy | Return owner/scope-safe busy result or bounded wait |
| Audit unavailable before mutation | Fail closed |
| Audit terminal persistence interrupted | Return truthful receipt with `pending_recovery`, keep admission detectably incomplete, mark audit unhealthy, halt subsequent automatic mutation, and reconcile idempotently without provider redispatch |
| Secret service locked | Return locked/unavailable; do not request plaintext in chat |
| Unsupported Wayland operation | Return blocker and trusted settings handoff |
| Unknown future enum/property | Preserve unknown safely; do not panic or infer |

## 18. Non-Disruptive Testing Architecture

All completion tests use the dedicated Cargo feature `os-control-test` with `--no-default-features`; live composition uses the mutually exclusive `os-control-live` feature. The crate has `compile_error!` when both are enabled. Real bus, process, Polkit, session, secret, clipboard, notification, and device transport constructors are absent under `os-control-test`; scripted transports are mandatory. Live provider construction additionally requires a non-exported `LiveHostAccessToken` minted only by desktop/server startup composition, so integration tests (whose library is compiled without `cfg(test)`) cannot construct or inject live access. The test feature installs a process-wide panic sentinel on every raw process/bus/session transport and asserts it remains untouched. The focused test manifest records the feature set and rejects commands lacking `os-control-test`.

All test layers are code-level and host-safe.

### Unit tests

- Input validation, normalized DTOs, comparators, risk mapping and redaction.
- Version-tolerant parsers with malformed/localized/oversized fixtures.
- Provider selection and capability matrices.
- Rollback token and lifecycle state machines.
- Resource conflict and ordering.

### Scripted provider tests

`ScriptedHostOsControl` contains per-operation queues of observations/outcomes and a call log. Tests assert exact call order:

```text
capability_check → read_policy → audit_admission → observe_before → idempotency_decision
  → mutation_policy/approval_resume (only when change is required)
  → resource_acquire → seal_mutation_permit → observe_under_lease
  → apply_once → observe_after → verify → audit_terminal → resource_release
```

It supports owner loss, timeout, cancellation, stale state, permission denial, target disappearance, partial mutation, contradiction, rollback failure, and terminal-audit persistence interruption. Mandatory negative tests prove: missing provider returns `Unavailable`; forged, expired, wrong-session, wrong-action, wrong-parameter, wrong-target, and stale-capability grants fail before provider access; provider apply cannot occur before grant/resource/audit admission; ordinary unit tests cannot construct a live provider or reach `LocalEnvironment`; duplicate registry metadata/handlers fail registration; audit admission failure causes zero mutations; terminal failure returns `pending_recovery`, restart reconciliation appends one terminal, concurrent retries preserve cardinality, and no reconciliation path calls a provider; and an uncertain apply is never retried through another provider.

### Fake D-Bus tests

Use typed proxy interfaces or a private test bus/fake transport. Do not call the live session/system bus. Fixtures cover additive fields, unknown enum values, object-manager changes, signal order, and method errors.

### Structured-command tests

Inject a fake `EnvironmentProvider`; capture `CommandRequest`; assert fixed program, exact argv, bounded limits, neutral locale where required, no shell, and redaction. No child process is spawned.

### In-process prompt contract tests

Route representative text to canonical tools, run through registry and fake provider, and assert policy, approval metadata, resources, lifecycle and result envelope. These are not live feature tests and do not start Tauri/Axum.

### Prohibited test behavior

No test in this spec may alter live radio, network profile, VPN, firewall, Bluetooth, audio, microphone, display, package database, update state, mount table, print queue, clipboard, notification server, power/session state, secrets, system services, or hardware.

## 19. Performance and Bounds

Default bounds are configuration constants with validated ranges:

- Capability probe: ≤2 seconds per provider, bounded parallelism.
- Read query: ≤5 seconds unless domain-specific.
- Normal user setting apply: ≤15 seconds.
- Package/update transactions: provider progress plus global configured deadline.
- Network scan/Bluetooth scan: explicit bounded duration.
- Verification: one immediate query plus bounded event/query wait; no mutation retry.
- Lists: paginated/capped; no unbounded process, package, network, device, log, or search results.
- Logs/search/history: byte, row, time-range, and payload limits.
- Audit and receipt payloads: digests and normalized metadata rather than raw content.

Foreground user interactions outrank indexing, diagnostics, and background automation. Search indexing and health observation use bounded concurrency and pause under resource pressure.

## 20. Supply Chain and Dependency Policy

- Reuse current `zbus`, `sysinfo`, `arboard`, `notify-rust`, `rusqlite`, Tokio, serde, thiserror, tracing, and existing archive/file-watch dependencies where fit.
- Pin exact workspace versions before adding new code to satisfy steering.
- Add no Python sidecar requirement for core OS control.
- Add a dependency only after recording license, exact version, purpose, alternatives, and maintenance status.
- OS binaries remain runtime providers, not build dependencies; capability probes report absence.

## 21. Cutover Strategy

1. Freeze canonical contracts and safety.
2. Add provider contracts and fake infrastructure.
3. Inject provider into registry/context.
4. Migrate current handlers one domain at a time behind unchanged tool envelopes.
5. Add missing Phase 1 domains.
6. Add Phase 2/3 domains.
7. Delete direct subprocess/provider-selection code from tool facades.
8. Run code-level gates and update capability documentation.

No dual-run mutation is permitted. During migration, a tool uses either old code or new provider code in a branch, never both. Cut over only after fake-backed parity tests, then delete old code.

## 22. Requirement-to-Design Traceability

| Requirements | Design sections |
|---|---|
| OSC-001–OSC-009 | §§2–7, 11–15 |
| OSC-010–OSC-012 | §9.1, §9.8, §§10, 13 |
| OSC-013–OSC-014 | §§9.2–9.3, 10–13 |
| OSC-015–OSC-017 | §9.4, §§10–13 |
| OSC-018–OSC-021 | §§9.5–9.8, 10–13 |
| OSC-022 | §9.13, §§10, 13–19 |
| OSC-023–OSC-024 | §§9.9–9.10, 16–19 |
| OSC-025–OSC-029 | §§9.11–9.12, 12–19 |
| OSC-030 | §§2, 10–12, 17 |
| OSC-031–OSC-032 | §§7–9 and session matrix |
| OSC-033–OSC-034 | §§18–20 |
| OSC-035–OSC-036 | §§3, 15, 21 |

## 23. Design Completion Conditions

The design is implementation-ready when:

- Every in-scope operation has a canonical tool, provider port, risk, resource, verification, rollback, redaction, and test oracle.
- Existing direct host execution has an explicit migration and deletion task.
- Wayland/X11 behavior is operation-specific and runtime-probed.
- Future Ubuntu compatibility is based on stable interfaces and honest degradation, not version promises.
- All live/disruptive validation is excluded from this spec without weakening code-level correctness.
- Deferred and BLACK scope cannot be reconstructed through generic broker/provider interfaces.
