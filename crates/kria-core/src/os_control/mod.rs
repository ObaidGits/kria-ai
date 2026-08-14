//! `os_control` — native Linux OS-control runtime for the
//! linux-os-control-production specification.
//!
//! This module tree is introduced incrementally by the spec's F0/F1 tasks. The
//! full contract/runtime/provider surface (design §3) is owned by later tasks:
//!
//! * `contract.rs`, `error.rs`, `receipt.rs`, `context.rs` — Task **1.1**
//! * `runtime.rs` — Task **1.7**, `resource.rs` — Task **1.6**, `audit.rs` /
//!   `redaction.rs` — Task **1.8**, `capability.rs` — Task **1.3**, and the
//!   `linux/*` transports + per-domain provider modules across Tasks 1.x–3.x.
//!
//! **Task 0.4** ("Establish code-test safety rules", OSC-033/OSC-034) owns the
//! host-safety scaffolding that must exist *before* any of that runtime code so
//! it is impossible to accidentally introduce live OS mutation into tests:
//!
//! * the mutually exclusive `os-control-test` / `os-control-live` features and
//!   the [`compile_error!`] guard below (design §18);
//! * the [`access`] seam: the non-exported [`access::LiveHostAccessToken`] that
//!   live provider construction requires, and the process-wide deny-live
//!   transport sentinel every raw transport constructor must call;
//! * the [`testing`] module: scripted fakes, a call recorder, fake-receipt
//!   tagging, and centralized temp / in-memory fixtures used by every
//!   completion test (compiled only under `os-control-test`).

// ── Deny-live composition guard (Task 0.4 / design §18) ─────────────────────
// `os-control-test` (deny-live test composition) and `os-control-live` (live
// desktop/server composition) select opposite worlds and must never be linked
// into the same binary. Enabling both is a hard build failure, which guarantees
// no completion-test binary can also carry live provider/token construction.
#[cfg(all(feature = "os-control-test", feature = "os-control-live"))]
compile_error!(
    "features `os-control-test` and `os-control-live` are mutually exclusive: a completion-test \
     (deny-live) build must never link live OS provider/transport construction. Build tests with \
     `--no-default-features --features os-control-test`; enable `os-control-live` only in the \
     desktop/server startup composition roots."
);

pub mod access;

// ── Task 2.1: audio domain provider (AudioControl) ──────────────────────────
// The `AudioControl` desired-state provider (design §3): normalized endpoint
// observation, moved table-driven parsers, provider selection over
// wpctl/pactl/amixer, and mute/privacy classification. The live PipeWire
// transport lives under `linux::providers::pipewire` (deny-live armed); fakes
// are test-only.
pub mod audio;

// ── Task 2.2: display domain provider (DisplayControl) ──────────────────────
// The `DisplayControl` desired-state provider (design §3, §9.6): normalized
// brightness observation, moved table-driven parsers, backend selection over
// the GNOME session D-Bus property / `brightnessctl` / XRandR gamma (X11-only,
// never selected on Wayland — OSC-019.3, OSC-032.3), and physical-backlight vs.
// software-gamma classification. Live adapters live under
// `linux::providers::gnome_display` et al. (deny-live armed); fakes are
// test-only.
pub mod display;

// ── Task 2.3: connectivity domain provider (ConnectivityControl) ────────────
// The `ConnectivityControl` desired-state provider (design §3, §9.4): normalized
// Wi-Fi radio/connection observation, moved table-driven `nmcli` terse-output
// parsers, provider selection over NetworkManager D-Bus / `nmcli`, and the
// duplicate-SSID clarification path (OSC-015). The live NetworkManager
// transport lives under `linux::providers::network_manager` (deny-live armed);
// fakes are test-only.
pub mod connectivity;

// ── Task 2.3: power domain provider (PowerControl) — profile slice ─────────
// The `PowerControl` profile-slice desired-state provider (design §3, §9.7):
// normalized power-profile observation, moved `powerprofilesctl` output parser,
// and provider selection over `power-profiles-daemon` D-Bus / `powerprofilesctl`.
// Suspend/hibernate/shutdown/session operations are Task 2.4's scope. The live
// power-profiles transport lives under `linux::providers::power_profiles`
// (deny-live armed); fakes are test-only.
pub mod power;

// ── Task 2.5: process domain provider (ProcessControl) ──────────────────────
// The `ProcessControl` desired-state provider (design §3, §9.5): native
// `kill(2)`/`setpriority(2)` syscall mutations for `kill_process` and
// `set_process_priority` — never a subprocess. PID-reuse-safe identity
// binding and the graceful-vs-forced signal split. The live adapter lives
// under `linux::providers::process_control` (deny-live armed); fakes are
// test-only.
/// Printing: discovery, queue, submission, cancellation, configuration.
/// Firewall status, enable/disable, temporary app access.
pub mod firewall;
/// Camera, microphone and location privacy controls.
pub mod privacy;
/// Desktop search: query, scope, rebuild.
/// System health: diagnosis, logs, recovery recipes.
/// Backup integration and document scanning.
pub mod backup;
/// Firmware awareness and hardware sensors (read-only).
pub mod hardware;
pub mod health;
pub mod search;
pub mod print;
pub mod processes;

// ── Task 2.5: application graceful-close provider (ApplicationCloseControl) ─
// The graceful-close slice of `ApplicationControl` (design §3, §9.3):
// `graceful_close_application` sends SIGTERM to matching processes — a
// distinct, lower-risk-tier operation from `processes::kill_process`'s
// PID-targeted forced kill. The live adapter lives under
// `linux::providers::application_control` (deny-live armed); fakes are
// test-only.
pub mod applications;

// ── Task 2.5: clipboard domain provider (ClipboardControl) ──────────────────
// The `ClipboardControl` desired-state provider (design §3, §9.10): a
// content-digest-bound normalized observation for `set_clipboard`, with
// `get_clipboard` as a pure read outside the mutation lifecycle. The live
// adapter lives under `linux::providers::clipboard` (deny-live armed); fakes
// are test-only.
pub mod clipboard;

// ── Task 2.5: notification domain provider (NotificationControl) ────────────
// The `NotificationControl` desired-state provider (design §3, §9.10):
// replaces the direct `notify-send` subprocess/`notify_rust` fallback with a
// freedesktop-portal-style provider seam. The live adapter lives under
// `linux::providers::notifications` (deny-live armed); fakes are test-only.
pub mod notifications;

// ── Task 2.5: automation listing provider (AutomationControl.list slice) ───
// The read-only `list_scheduled_tasks` slice of `AutomationControl` (design
// §3, §9.13): replaces the direct `crontab -l`/`systemctl --user
// list-timers` subprocess calls in `tools/scheduler.rs`. `create_scheduled_task`/
// `delete_scheduled_task` remain Task 4.5's scope (typed schedule/action
// authority). The live adapter lives under `linux::providers::automation`
// (deny-live armed); fakes are test-only.
pub mod automation;

// ── Task 3.1: files domain (TrashControl, ArchiveControl, OwnershipControl) ─
// Trash lifecycle (freedesktop.org spec), bounded zip archives, and
// broker-backed ownership changes (design §3, §9.1, §10.1). Closes the
// OSC-011 gap: previously there was no Trash tool, no permanent-delete as a
// distinct action, and no archive support at all. `tools/file_ops.rs`'s
// existing read/write/list/copy/rename/move handlers already use plain
// `std::fs` (no shell/subprocess), so this module adds only the missing
// Trash/archive/ownership seams rather than migrating an existing subprocess
// call site.
pub mod files;

// ── Task 3.2: storage domain (StorageControl) ───────────────────────────────
// Typed discovery, mount, unmount, eject, and health for removable/fixed
// storage over UDisks2 (design §3, §9.1, §10.1, §12). Mount/unmount/eject
// dispatch directly to UDisks2's own typed Polkit authorization — never
// through `broker::BrokerOperation` (design §12) — and there is no
// force/format/partition/resize/secure-erase/encryption-provisioning
// operation anywhere in this module (OSC-012.4, OSC-012.6, OSC-030). The
// live adapter lives under `linux::providers::udisks` (deny-live armed);
// fakes are test-only.
pub mod storage;

// ── Task 3.4: packages domain (PackageControl) ──────────────────────────────
// Package planning, install/remove, and update assessment over PackageKit
// (primary) with typed apt/dnf/pacman/zypper/snap/flatpak adapters as
// fallback (design §3, §9.3, §10.1, §12). Closes the OSC-014 gap: exact
// preflight plans, an install-vs-update-vs-remove-vs-no-change semantic
// split (fixing the legacy installed-package no-op bug for updates), and
// privileged transactions dispatched exclusively through
// `BrokerOperation::ApplyPackagePlan` bound to the approved plan digest —
// never a direct pkexec/sudo subprocess. The live adapter lives under
// `linux::providers::packagekit` (deny-live armed); fakes are test-only.
pub mod packages;

// ── Task 1.1: base contracts, grants, receipt sums, and canonical errors ────
// Provider-independent DTO foundations, the pre-mutation error taxonomy, the
// narrow dispatch/receipt sum types, and the observation/mutation execution
// contexts (design §4, §5). The mutation-context constructor is deliberately
// unclaimed until Task 1.7 (runtime sealing).
pub mod context;
pub mod contract;
pub mod error;
pub mod receipt;

// ── Task 1.2: strict registry metadata + OsControlRuntime injection ─────────
// `manifest` is the strict typed projection of the frozen §§10.1–10.4 manifest
// (the single embedded fixture); `runtime` is the injectable composition seam
// that keeps raw `HostOsControl` private. Full lifecycle sealing is Task 1.7.
pub mod manifest;
pub mod runtime;

// ── Task 1.3: SessionContext + capability probing ───────────────────────────
// `capability` owns the injectable `SessionProbe` seam, env-hint normalization,
// the deterministic operation-level `CapabilitySnapshot`, and the per-domain
// caching prober. `linux` owns the live D-Bus transport (deny-live armed) and
// the scripted probe matrix that drives every capability-probing test.
pub mod capability;
pub mod linux;

// ── Task 1.5: the typed Polkit privilege broker ─────────────────────────────
// The closed six-operation request boundary and the effect-aware response
// protocol (design §12). `broker` owns the canonical length-prefixed CBOR
// codec, the typed `BrokerRequestV1`/`BrokerResponseV1`, persistent nonce replay
// semantics, the Polkit + fixed-native-operation seams (fakes + deny-live
// stubs), the request/response client, and the Polkit action/policy packaging.
pub mod broker;

// ── Task 1.6: deterministic OS resource leasing ─────────────────────────────
// `resource` owns the typed OS resource kinds/scopes (OSC-008.1), the
// manifest-driven canonical derivation, the single canonical resource-set
// digest shared with `ExecutionGate`, and the non-cloneable
// `AcquiredResourceLeaseSet` that Task 1.7 runtime sealing consumes.
pub mod resource;

// ── Task 1.8: durable audit admission, redaction, and reconciliation ────────
// `redaction` owns the single shared sensitivity registry (design §14) used by
// durable audit, HITL projection, and provider tracing. `audit` owns the
// fallible one-admission / idempotent-one-terminal SQLite authority, bounded
// incomplete-admission reconciliation, and the `AuditAdmissionToken` producer.
pub mod audit;
pub mod redaction;

// ── Task 1.10: Secret Service + scoped skill (sandbox) grants ───────────────
// `secrets` owns the opaque `SecretRef`/metadata DTOs, the non-serializable
// zeroizing `SecretPayload` wrapper, and the provider-only `CredentialStore`
// resolution/store/delete contract with fail-closed locked/unavailable
// behaviour (OSC-025/OSC-029). `sandbox` owns the scoped, expiring,
// per-domain-operation skill grant schema with revocation and deny-by-default
// authorization (OSC-026). The live Secret Service transport lives under
// `linux::providers::secret_service` (deny-live armed); fakes are test-only.
pub mod sandbox;
pub mod secrets;

/// Bluetooth adapter and device lifecycle over BlueZ (Task 3.7, OSC-021).
/// Discovery is privacy-sensitive: `get_bluetooth_state` and `scan_bluetooth` are
/// RED reads, and pair / trust / remove are RED mutations requiring approval.
pub mod bluetooth;

// The governed-call bundle: carries the agent's admission artifacts (grant,
// leases, audit token, observation context) to a canonical OS handler, which is
// what lets a handler perform a governed mutation at all.
pub mod governed;

// The production capability catalog: per canonical operation, which providers
// could serve it and what each needs to be eligible. Feeds `CapabilityProber`
// and derives the live probe plan.
pub mod catalog;

// Centralized deny-live test surface (Task 0.4, OSC-033/OSC-034): fakes,
// fixtures, call recorder and the `assert_no_live_access` posture check. Present
// only in the deny-live test composition, so no live build can reach it.
#[cfg(feature = "os-control-test")]
pub mod testing;

// The LIVE host composition root — the only place live transports are built and
// the only place `LiveHostAccessToken` is minted. Present only under
// `os-control-live`, which is mutually exclusive with `os-control-test`.
#[cfg(feature = "os-control-live")]
pub mod live;


pub use capability::{
    BusKind, BusStatus, CapabilityAvailability, CapabilityProber, CapabilityRequirement,
    CapabilitySnapshot, ConfirmationPolicy, DesktopFamily, DisplayServer, DisplayServerSupport,
    Domain, EnvHints, PortalStatus, ProviderCandidate, ProviderNeeds, ServiceOwner, SessionProbe,
};
pub use context::{
    AdmittedMutationContext, AuditAdmissionToken, ExecutionGrant, HostExecutionContext,
    MutationPermit, ObservationAuditAuthority, RedactionPolicy, SessionContext,
};
pub use contract::{
    ActionId, AuditAdmissionId, AuditRecordId, AuditRecoveryKey, AvailabilityStatus, BoundedVec,
    CapabilityId, ComparatorKind, CorrelationId, DecisionId, DesiredStateControl, Digest,
    GrantDecision, GrantId, GrantNonce, NonEmptyBoundedVec, OsEvidenceSource, ProviderId,
    ReceiptId, SafeCandidate, SafeErrorCode, SafeField, SafeOperation, SafeResource, SafeRevision,
    SafeStepId, SafeText, SafeWarning, SessionId, SnapshotRevision, Tolerance, VerificationClass,
    VerificationReliability,
};
pub use error::{GrantInvalidReason, OsControlError};
pub use linux::dbus::LiveDbusTransport;
pub use linux::probe::{LiveProbePlan, LiveSessionProbe};
pub use linux::structured_command::{
    classify_post_dispatch, compute_argv_digest, CommandPlan, CommandPolicy, CommandPolicyDecision,
    PostDispatchInterruption, RedactionMap, StructuredCommandRequest, StructuredCommandSummary,
    TrustedExecutable, ALLOWED_ENV_KEYS, FIXED_LOCALE, MAX_ARGV, REDACTED_PLACEHOLDER,
};
pub use receipt::{
    AcceptanceEvidence, AcceptedDispatch, ActionLifecycle, AppliedDispatch, ApplyOutcome,
    AuditCompletionState, ContradictedDispatch, FailureRollbackState, MutationReceipt,
    MutationResult, PartialDispatch, PartialEffectCause, ReceiptCommon, RedactedObservation,
    RollbackAvailability, RollbackEligibleFailure, RollbackFailure, RollbackToken,
    SafeReceiptSummary, SatisfyingVerification, UncertainDispatch, UncertainEffectCause,
    UnverifiedCause, UnverifiedDispatch, VerificationContradiction, VerificationReport,
};
pub use resource::{
    AcquiredResourceLeaseSet, OsLeaseContext, OsResource, OsResourceCoordinator, OsResourceKind,
};

pub use access::{
    deny_live_transport, live_composition_count, sentinel_is_armed, sentinel_trip_count,
    LiveHostAccessToken, RawTransportKind,
};

pub use manifest::{
    frozen_contract, frozen_contracts, frozen_tool_names, ManifestRisk, ManifestVerificationClass,
    ProviderOperation, ResourceDerivation, ResumePolicy, RiskFunction, RiskRule, RollbackClaim,
    TargetPolicy, ToolContractMetadata, FROZEN_OPERATION_COUNT,
};
pub use runtime::{
    evidence_is_fresh, observation_satisfies, strongest_os_evidence, HostOsControl, MutationPlan,
    NormalizedObservation, OsControlRuntime, RollbackPlan, RuntimeSealAuthority, SealBinding,
};

pub use broker::{
    build_broker_request, dispatch_via_broker, parse_policy_actions, polkit_action_id,
    BoundedBrokerEvidence, BoundedPackageTransaction, BoundedPercent, BrokerBoundPath,
    BrokerDispatchOutcome, BrokerOperation, BrokerPreDispatchError, BrokerRequestV1,
    BrokerResponseBinding, BrokerResponseV1, BrokerTransport, BrokerTransportError, CallerContext,
    ChargeThresholdAdapterId, DiscoveredPrinterId, ExistingLocalIdentity, FirewallProviderId,
    InMemoryNonceStore, LocalBroker, NativeBrokerOperations, PackageProviderId, PackageStep,
    PackageStepAction, PeerCredentials, PolkitAuthorizer, PolkitDecision, RecognizedPrivacyControl,
    ReviewedPrinterOptions, BROKER_ACTION_IDS, BROKER_POLKIT_POLICY,
};

pub use audit::{
    AdmissionRequest, AuditFault, AuditHealth, OsAuditStore, ReconcileReport, RequestSensitivity,
    TerminalAppendOutcome, TerminalRecord, MAX_SCAN_LIMIT, OUTCOME_UNKNOWN_AFTER_CRASH,
    TERMINAL_DIGEST_CONFLICT,
};
pub use redaction::{
    canonical_json, classify_field, parameter_digest, redact_parameters, redact_value,
    ApprovalProjection, DataClass, RedactedParameters, RedactedValue, SafeResourceSummary,
};

pub use secrets::{
    now_unix as secret_now_unix, purpose_scope_mismatch, service_unavailable, unknown_reference,
    CredentialStore, ProtectedInputHandle, SecretMetadata, SecretMetadataPage, SecretPayload,
    SecretPurpose, SecretRef, SecretResolutionRequest, SecretScope, SecretServiceState,
};

pub use sandbox::{
    is_known_capability, FilesystemLimit, GrantRequest, NetworkLimit, SandboxDenyReason,
    SandboxGrant, SandboxGrantAuthority, SandboxGrantControl, SandboxGrantId, SandboxScope,
    SkillIdentity, SkillOperationRequest,
};

pub use linux::providers::secret_service::LiveSecretService;

pub use audio::{
    endpoint_data_class, is_privacy_sensitive, AudioBackend, AudioControl, AudioControlPort,
    AudioEndpointKind, AudioEndpointState, AudioFocus, AudioOp, AudioRequest, AudioTransport,
    AUDIO_TOLERANCE_DEFAULT, AUDIO_TOLERANCE_MAX,
};
pub use linux::providers::pipewire::LivePipewireAudio;

pub use display::{
    display_state_result, select_brightness_backend, set_brightness_result, BrightnessBackend,
    DisplayControl, DisplayControlPort, DisplayOp, DisplayRequest, DisplayState, DisplayTransport,
    DISPLAY_TOLERANCE_DEFAULT, DISPLAY_TOLERANCE_MAX,
};
pub use linux::providers::gnome_display::LiveGnomeDisplay;

pub use connectivity::{
    connect_wifi_result, toggle_wifi_result, wifi_networks_result, ConnectWifiOp,
    ConnectivityBackend, ConnectivityControl, ConnectivityControlPort, ConnectivityFocus,
    ConnectivityOp, ConnectivityRequest, ConnectivityState, ConnectivityTransport,
    RawWifiNetwork,
};
pub use linux::providers::network_manager::LiveNetworkManager;

pub use power::{
    power_plan_result, set_power_plan_result, PowerControl, PowerControlPort, PowerProfile,
    PowerProfileBackend, PowerProfileOp, PowerProfileRequest, PowerProfileState,
    PowerProfileTransport,
};
pub use linux::providers::power_profiles::LivePowerProfiles;

pub use power::session::{
    lock_screen_result, session_ending_result, shutdown_result, PowerSessionBackend,
    PowerSessionControl, PowerSessionControlPort, PowerSessionOp, PowerSessionRequest,
    PowerSessionState, PowerSessionTransport,
};
pub use linux::providers::logind::LiveLogind;

pub use processes::{
    kill_process_result, process_permission_denied_error, set_process_priority_result,
    unknown_process_identity_error, BoundedArgvElement, BoundedCommandMetadata,
    CommandMetadataState, ProcessControl, ProcessControlPort, ProcessFilter, ProcessFocus,
    ProcessIdentity, ProcessLifecycleState, ProcessObservation, ProcessOp, ProcessPage,
    ProcessRequest, ProcessState, ProcessTransport, MAX_ARGV_ELEMENTS, MAX_ARGV_ELEMENT_BYTES,
    MAX_ARGV_TOTAL_BYTES, MAX_PROCESS_PAGE, PROCESS_PROVIDER_ID,
};
pub use linux::providers::process_control::LiveProcessControl;

pub use applications::{
    graceful_close_result, list_installed_apps_result, manage_autostart_result,
    set_default_application_result, ApplicationCloseControl, ApplicationCloseControlPort,
    ApplicationCloseRequest, ApplicationCloseState, ApplicationCloseTransport, AssociationFocus,
    AssociationOp, AssociationRequest, AssociationState, DesktopAssociationControl,
    DesktopAssociationControlPort, DesktopAssociationTransport, InstalledApplication,
    InstalledApplicationPage, RealDesktopAssociationTransport, APPLICATION_CLOSE_PROVIDER_ID,
    DESKTOP_ASSOCIATION_PROVIDER_ID,
};
pub use linux::providers::application_control::LiveApplicationControl;

pub use clipboard::{
    get_clipboard_result, set_clipboard_result, ClipboardControl, ClipboardControlPort,
    ClipboardRequest, ClipboardState, ClipboardTransport, CLIPBOARD_PROVIDER_ID,
};
pub use linux::providers::clipboard::LiveClipboard;

pub use notifications::{
    portal_acceptance_evidence, send_notification_result, NotificationControl,
    NotificationControlPort, NotificationRequest, NotificationState, NotificationTransport,
    NOTIFICATION_PROVIDER_ID,
};
pub use linux::providers::notifications::LiveNotifications;

pub use automation::{
    list_scheduled_tasks_result, AutomationControl, AutomationControlPort, AutomationListing,
    AutomationTransport, AUTOMATION_PROVIDER_ID,
};
pub use linux::providers::automation::LiveAutomation;

pub use files::{
    canonical_path_identity, create_archive_result, extract_archive_result,
    list_archive_result, occupied_restore_target_error, restore_trash_item_result,
    set_file_ownership_result, trash_file_result, unknown_trash_item_error, ArchiveControl,
    ArchiveControlPort, ArchiveEntry, ArchiveEntryPage, ArchiveFormat, ArchiveMutationResult,
    ArchiveOp, ArchiveRequest, ArchiveState, ArchiveTransport, OwnershipControl,
    OwnershipControlPort, OwnershipRequest, OwnershipState, RealArchiveTransport,
    RealOwnershipTransport, RealTrashTransport, RestoreMoveOutcome, RestoreResolution,
    TrashControl, TrashControlPort, TrashItem, TrashItemId, TrashMoveOutcome, TrashOp,
    TrashRequest, TrashState, TrashTransport, ARCHIVE_PROVIDER_ID, MAX_ARCHIVE_ENTRIES,
    MAX_ARCHIVE_EXPANDED_BYTES, MAX_ARCHIVE_INPUT_ENTRIES, MAX_COMPRESSION_RATIO,
    MAX_ENTRY_EXPANDED_BYTES, OWNERSHIP_PROVIDER_ID, TRASH_PROVIDER_ID,
};
pub use files::{validate_entry_bounds, validate_entry_path, ArchiveFocus};
pub use linux::providers::files::LiveFileOwnershipBroker;

pub use storage::{
    device_busy_error, eject_device_result, get_storage_health_result,
    list_storage_devices_result, mount_device_result, unmount_device_result, FilesystemId,
    HealthAvailability, MountLabel, StorageControl, StorageControlPort, StorageDeviceId,
    StorageDeviceInfo, StorageDevicePage, StorageFocus, StorageHealthReport, StorageMountState,
    StorageOp, StorageRequest, StorageTransport, MAX_STORAGE_DEVICE_PAGE, STORAGE_PROVIDER_ID,
};
pub use linux::providers::udisks::LiveUdisks;

// Note: `os_control::packages::PackageProviderId` (the domain-level provider
// enum: PackageKit/Apt/Dnf/Pacman/Zypper/Snap/Flatpak) is deliberately *not*
// re-exported at this module root — it would collide with the already-frozen
// wire-protocol `broker::PackageProviderId` (Apt/Snap/Flatpak only, design
// §12). Callers reach it via `os_control::packages::PackageProviderId`.
pub use packages::{
    check_system_updates_result, get_package_info_result, get_reboot_required_result,
    install_package_result, list_installed_packages_result, plan_package_changes_result,
    search_package_result, uninstall_package_result, PackageChange,
    PackageChangeClassification, PackageControl, PackageControlPort, PackageEntry,
    PackageObservation, PackageOperation, PackagePage, PackagePlan, PackageRef, PackageRequest,
    PackageTransactionState, PackageTransport, RebootRequirement, UpdateAssessment,
    MAX_PACKAGE_PAGE, MAX_PLAN_PACKAGES, PACKAGE_PROVIDER_ID,
};
pub use linux::providers::packagekit::LivePackageKit;
