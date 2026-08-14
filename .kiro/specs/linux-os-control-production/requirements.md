# Requirements Document

## Linux OS Control — Production Desktop Control Plane

**Feature:** `linux-os-control-production`  
**Spec type:** Feature  
**Workflow:** Design-first  
**Phase:** Specification complete; implementation not begun  
**Status:** Planned target; no statement or checkbox in this specification is implementation evidence

## Introduction

This specification turns KRIA's existing Linux controls into a safe, typed, verifiable desktop operating-system control plane. It covers the approved v1 and v2 desktop scope: normal file, application, process, software, connectivity, Bluetooth, audio, display, power, device, diagnostic, privacy, notification, search, automation, and sandbox workflows. It does not turn KRIA into a Linux administration suite.

The implementation must work end to end from a natural-language prompt through routing, policy, approval, resource leasing, provider execution, postcondition verification, audit, and truthful user-visible result. `kria-core` remains the domain authority. Tauri, Axum, and SolidJS remain presentation or transport adapters and must not invent OS state or bypass policy.

Testing in this specification is deliberately limited to code-level validation: unit tests, parser tests, fake-provider contract tests, in-process routing/handler tests, compile checks, lint, and format checks. Tests must not mutate the developer's live Wi-Fi, Bluetooth, audio, display, packages, mounts, power state, firewall, session, or other host configuration. Live and disruptive acceptance campaigns are outside this specification and will be planned separately.

## Authority and Precedence

1. Shipped code and executable evidence define current behavior.
2. This document defines the required future behavior for this feature.
3. `design.md` defines the planned implementation architecture for these requirements.
4. `tasks.md` defines the dependency-ordered implementation plan; checked boxes alone are never evidence.
5. Workspace steering remains binding: `kria-core` owns domain logic, SQLite is the durable authority, dangerous operations pass through safety, and existing Tauri command/event names remain stable.
6. The approved scope from the Linux capability architecture review is normative: desktop workflows are in scope; destructive system administration is not.

## Product Outcomes

1. A user can control approved Ubuntu desktop features through ordinary prompts without requiring generated shell commands.
2. Every mutation is typed, policy-governed, target-bound, bounded, cancellable where meaningful, audited, and verified when the OS exposes a postcondition.
3. KRIA reports `Unsupported`, `Unavailable`, `PermissionDenied`, `Unverified`, or `Accepted` honestly instead of claiming false success.
4. Ubuntu GNOME X11 and Wayland receive equivalent domain behavior wherever the underlying OS service is display-server neutral.
5. Desktop-specific limitations are discovered at runtime and returned as capability metadata, not inferred from Ubuntu version strings.
6. Existing working controls migrate into one cohesive provider architecture without parallel policy paths or duplicated domain authority.
7. Future Ubuntu releases degrade safely through interface probing and provider negotiation rather than breaking because of version checks.
8. Dangerous Linux administration remains unavailable from the normal assistant even if a binary exists on the host.

## Product Principles

1. **Structured capability over shell:** normal prompts resolve to typed tools and typed provider requests.
2. **State over exit code:** success follows authoritative re-observation, not process completion alone.
3. **Least authority:** user-session operations remain unprivileged; privileged operations use a narrow broker.
4. **Desktop completeness, not administration breadth:** implement daily laptop workflows; hand off specialist administration.
5. **Capability negotiation over version detection:** probe interfaces, properties, binaries, permissions, and session services.
6. **Same semantics across providers:** D-Bus, portal, and CLI adapters return one normalized contract.
7. **Honest degradation:** unavailable providers preserve reason, remediation, and safe alternatives.
8. **Privacy by construction:** secret and sensitive state has explicit redaction and retention rules.
9. **Idempotency by default:** setting an already-satisfied state returns unchanged without mutation.
10. **Code evidence before live validation:** implementation readiness is proven with deterministic code tests; host acceptance is separate.

## Scope Classification

### Required for v1

- Capability runtime, discovery, host target binding, approval, verification, audit, rollback metadata, resource leases, and privilege broker.
- Files, Trash, archives, basic permissions, storage discovery, and removable-media lifecycle.
- Applications, processes, package install/remove, Wi-Fi, Ethernet, audio input/output, brightness, Bluetooth, power/session, health, clipboard, notifications, secrets, and skill grants.
- Prompt routing and truthful result presentation for every capability.

### Required or recommended for v1 completion

- Desktop search, default applications, network diagnostics, captive portals, existing VPN profiles, firewall status/toggle, updates, scheduling, allowlisted recovery, logs, storage health, printing, privacy controls, thermal/resource warnings, clipboard history, and DND.

### v2 / nice to have

- Full monitor configuration with timed rollback, per-application audio, MPRIS media control, hotspot, managed temporary firewall grants, supported battery thresholds, hardware sensors, firmware awareness, scanner integration, backup-provider integration, managed proxy, and saved connectivity credential lifecycle.

### Deferred

- Remote fleet, general VM/container management, Thunderbolt authorization, dock/controller configuration, malware-scanner adapters, and actual firmware update execution.

### Out of Scope

- Partitioning, formatting, filesystem resizing, secure erase, full-disk encryption provisioning, GRUB/bootloader/Secure Boot mutation, kernel selection/tuning/modules, full user/group/password/sudo administration, SELinux/AppArmor policy editing, CA/PKI administration, raw iptables/nftables rules, vendor firmware flashing, fan/embedded-controller writes, overclocking, and arbitrary systemd unit creation.

## Glossary

- **OS_Control_Runtime:** The governed `kria-core` boundary that admits, executes, verifies, audits, and reports OS capabilities.
- **Host_OS_Provider:** A typed provider that controls the local user’s host OS and cannot target VM, container, or remote environments.
- **Domain_Provider:** A cohesive provider such as Audio, Connectivity, Display, Storage, Power, Bluetooth, or Packages.
- **Capability_Probe:** A bounded runtime check of interface ownership, method/property support, permissions, binaries, and session context.
- **Session_Context:** Normalized X11/Wayland, desktop, user-session, bus, portal, and display information; environment values are hints only.
- **Observation:** Provider-independent state read from an authoritative or explicitly ranked source.
- **Desired_State:** Validated normalized state requested by a tool.
- **Mutation_Receipt:** Before state, attempted operation, provider, outcome, after state, verification, rollback token, and timing.
- **Accepted:** The OS accepted an inherently asynchronous action; completion cannot be observed because the session may terminate.
- **Verified:** A fresh independent observation satisfies the requested postcondition.
- **Rollback_Token:** Opaque bounded data sufficient to restore the exact prior state when reliable rollback exists.
- **Secret_Reference:** Opaque identifier to a credential held by the system secret service.
- **Code_Level_Test:** A unit, parser, fake-provider contract, in-process routing, compile, lint, or format check that does not mutate live OS state.

## Requirements

### OSC-001 — Governed Capability Runtime

**Priority:** P0 — Required  
**User story:** As a user, I want every OS action to use the same safety boundary so that no provider bypasses policy or approval.

#### Acceptance Criteria

1. THE OS_Control_Runtime SHALL process each request through readiness, preflight, host target binding, capability probing, read-policy admission, one durable logical-action admission, bounded pre-observation, and idempotency evaluation; only a required mutation SHALL then pass mutation policy, approval when required, resume revalidation, write-resource acquisition, admission-token/grant/lease sealing, under-lease re-observation, execution, verification, and durable terminal append in that order. No second admission SHALL be created for the mutation phase.
2. THE runtime SHALL reuse `ExecutionGate`, `PolicyEngine`, durable `InteractionDecision`, `HitlGateway`, cancellation, and existing stream events rather than create a second approval path.
3. THE runtime SHALL reject provider mutation unless admission produced a valid execution grant bound to action, parameters, user session, `ExecutionTarget::Host`, capability revision, and canonical resource-set digest, and the runtime then sealed that grant with the currently held matching leases plus committed audit-admission token into a non-forgeable mutation permit.
4. WHEN an action resumes after approval, THE runtime SHALL revalidate target, parameters, risk, capability availability, and resource requirements before mutation.
5. IF risk increases or the target/capability changes before resume, THE runtime SHALL invalidate prior approval and request a new decision.
6. THE runtime SHALL support read, desired-state mutation, asynchronous accepted action, and explicitly prohibited capability classes.
7. THE runtime SHALL not treat a task checkbox, process exit code, emitted event, or provider claim as implementation or success evidence.
8. THE existing `CapabilityPlatform`, extension permission engine, and extension grant store SHALL not register, authorize, or directly execute native OS mutations; extension-requested host effects SHALL re-enter a canonical OS tool through this runtime.
9. OS-action `InteractionDecision` creation and resolution SHALL use SQLite durable authority and SHALL fail closed without in-memory/JSONL fallback or continue-on-persistence-error behavior.

### OSC-002 — Host-Only Target and Structured Execution

**Priority:** P0 — Required

#### Acceptance Criteria

1. THE Host_OS_Provider SHALL be statically bound to the local host and SHALL reject VM, Docker, remote, or ambiguous targets.
2. WHEN a fallback binary is required, THE adapter SHALL invoke a fixed executable with a vector of validated argv values and no shell interpreter.
3. THE adapter SHALL impose time, byte, line, retry, and cancellation bounds.
4. THE adapter SHALL use an absolute trusted executable discovered during provider probing or a validated PATH resolution policy.
5. THE provider SHALL not concatenate user values into commands, D-Bus signatures, file paths, object paths, or environment assignments.
6. THE normal assistant SHALL not invoke `execute_bash`, `execute_python`, or `execute_powershell` to satisfy an approved structured OS capability.
7. RAW shell execution SHALL be disabled by default or explicitly marked Expert Mode, RED, always-confirmed, non-rollbackable, and unavailable to unattended automation.

### OSC-003 — Capability Discovery and Session Context

**Priority:** P0 — Required

#### Acceptance Criteria

1. AT startup and after relevant service-owner changes, THE runtime SHALL build a bounded capability snapshot from OS, desktop, bus, portal, service, binary, method, property, and permission probes.
2. THE snapshot SHALL identify X11, Wayland, desktop family, session bus, system bus, portals, selected provider, fallback providers, supported operations, degradation reason, and authorization requirements.
3. THE runtime SHALL treat `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY`, `DISPLAY`, and `XDG_CURRENT_DESKTOP` as hints and SHALL confirm provider availability independently.
4. THE runtime SHALL not branch behavior on Ubuntu release numbers.
5. WHEN an interface changes or disappears, THE runtime SHALL invalidate the affected probe and renegotiate only that domain.
6. THE tool registry SHALL expose only usable tools or SHALL return a structured unavailable response before planning, according to one consistent registry policy.
7. THE capability snapshot SHALL contain no secret values and SHALL be safe for diagnostics.

### OSC-004 — Risk, Confirmation, and Least Privilege

**Priority:** P0 — Required

#### Acceptance Criteria

1. THE policy model SHALL retain GREEN, YELLOW, RED, and BLACK semantics.
2. READ-only local observations SHALL normally be GREEN unless privacy sensitivity requires stronger policy.
3. Bounded reversible user-level changes SHALL normally be YELLOW.
4. Destructive, privileged, privacy-sensitive, session-ending, update, package-removal, credential, and difficult-to-reverse actions SHALL be RED.
5. Out-of-scope administration SHALL be BLACK and absent from normal capability schemas.
6. Providers SHALL never prompt for approval or weaken a policy decision.
7. THE privileged broker SHALL expose fixed typed operations and SHALL not expose arbitrary command, arbitrary file-write, or generic run-as-root methods.
8. IF Polkit or required authorization is unavailable or denied, THE action SHALL fail without fallback to password capture or broader authority.

### OSC-005 — Verification and Truthful Completion

**Priority:** P0 — Required

#### Acceptance Criteria

1. EVERY synchronous mutation SHALL define a provider-independent postcondition before execution.
2. AFTER mutation, THE provider SHALL re-observe through the strongest available independent source within a bounded deadline.
3. IF the postcondition is satisfied, THE result SHALL be `Verified` and include normalized evidence.
4. IF the OS accepted an action that terminates or suspends observability, THE result SHALL be `Accepted`, never `Verified` or `Completed`.
5. IF verification is unavailable or ambiguous, THE result SHALL be `Unverified` even when the apply call returned success.
6. IF verification contradicts the desired state, THE result SHALL be `VerificationFailed` and SHALL trigger rollback only when the capability declares a safe rollback.
7. THE verifier SHALL not retry mutation, replan, or broaden authority.
8. Evidence SHALL include source, reliability, observation timestamp, freshness, provider, and redacted detail.

### OSC-006 — Rollback and Idempotency

**Priority:** P0 — Required

#### Acceptance Criteria

1. BEFORE a reversible mutation, THE provider SHALL capture the exact normalized prior state.
2. IF desired state already holds, THE operation SHALL return unchanged without mutation or approval escalation beyond the read required to establish idempotency.
3. A capability SHALL advertise rollback only if a reliable inverse and sufficient prior state exist.
4. Rollback tokens SHALL be opaque, bounded, session-scoped, expiring, and excluded from model-visible prose.
5. Rollback SHALL pass through policy and verification and SHALL be audited as a separate action linked to the original receipt.
6. Permanent deletion, process kill, shutdown, reboot, routine updates, and other non-reversible actions SHALL never claim rollback.
7. Multi-step operations SHALL either compensate completed reversible steps in reverse order or report partial completion precisely.

### OSC-007 — Durable Audit and Sensitive-Data Redaction

**Priority:** P0 — Required

#### Acceptance Criteria

1. EVERY admitted OS action SHALL create one durable logical safety-audit action in the existing SQLite authority, represented by one append-only admission record and at most one linked terminal completion or incident record; rollback SHALL be a separately linked logical action.
2. FOR every admission whose execution reaches a terminal runtime outcome, THE audit authority SHALL durably append exactly one linked terminal record, either during the request or through bounded idempotent reconciliation after a completion-persistence interruption or process restart.
3. THE linked audit records SHALL collectively include correlation/session IDs, action, parameter digest, target, risk, decision, provider, before/after state digests, verification status, rollback availability, duration, and error or incident class.
4. THE audit SHALL never store Wi-Fi/VPN passwords, secret values, clipboard contents, notification bodies, microphone data, private search contents, or raw authentication material.
5. Sensitive identifiers such as SSIDs, device names, filenames, and application titles SHALL follow configurable redaction or hashing policy.
6. Provider tracing SHALL use the same redaction classification as durable audit.
7. IF durable audit admission fails for a mutating action, THE runtime SHALL fail closed before mutation.
8. IF terminal-record persistence fails after execution may have started, THE runtime SHALL preserve the truthful OS outcome, mark the admission detectably incomplete, return `pending_recovery` audit state, block subsequent automatic mutations, and reconcile without dispatching the provider again.
9. THE audit schema SHALL enforce one terminal record per admission, and concurrent/replayed terminal appends SHALL be idempotent by admission identity.
10. Audit queries, incomplete-admission scans, reconciliation batches, and integrity checks SHALL remain bounded and preserve the existing hash-chain behavior.

### OSC-008 — Resource Coordination and Cancellation

**Priority:** P0 — Required

#### Acceptance Criteria

1. THE runtime SHALL declare resource scopes for filesystem paths, package database, network radio/profile, Bluetooth adapter/device, audio endpoint, display topology, storage device, power/session state, firewall, clipboard, notifications, printer, update manager, secret item, and sandbox grant.
2. Conflicting writes SHALL acquire exclusive leases; observations MAY share read leases.
3. Resource ordering SHALL be deterministic to prevent deadlocks in multi-domain actions.
4. Cancellation before mutation SHALL leave state unchanged.
5. Cancellation after a non-atomic mutation SHALL invoke safe compensation when declared or report partial state.
6. Session-ending actions SHALL become non-cancellable after the OS accepts them and SHALL report that boundary.
7. No unattended workflow SHALL hold a user-visible device or global subsystem lease indefinitely.

### OSC-009 — Prompt Routing and Stable Tool Contracts

**Priority:** P0 — Required

#### Acceptance Criteria

1. A natural-language request for an approved OS operation SHALL resolve to a canonical registered tool without generated shell.
2. Tool names, argument schemas, risks, target policy, resume capability, and result envelopes SHALL be synchronized across registry, router, fallback parser, prompt construction, tests, and documentation.
3. Existing public tool names and result fields SHALL remain stable unless the implementation performs one repository-wide hard cutover with all references updated atomically.
4. Existing `tool_start`, `tool_end`, `tool_progress`, `approval_required`, `approval_result`, and `hitl_ack` event names and fields SHALL remain compatible.
5. Additive result fields MAY include provider, lifecycle status, verification, availability, rollback availability, and remediation.
6. Ambiguous prompts SHALL ask for clarification rather than infer a device, network, destructive target, package, file, or credential.
7. BLACK capabilities SHALL not be routed from normal prompts.

### OSC-010 — File and Directory Lifecycle

**Priority:** P0 — Required

#### Acceptance Criteria

1. KRIA SHALL retain governed read, write, create, list, search, metadata, size, copy, move, and rename operations.
2. Writes SHALL be atomic where supported, use parent creation only when requested, and verify content/metadata without returning sensitive content as evidence.
3. Cross-filesystem moves SHALL support files and directories through copy-verify-delete semantics with partial-failure reporting.
4. Path handling SHALL prevent traversal, unexpected symlink following, protected-system-path mutation, and target substitution between approval and apply.
5. Permission changes SHALL be bounded to explicit paths and validated modes; ownership changes SHALL require privilege and RED approval.
6. File operations SHALL use the governed write boundary and resource lease for each affected path.
7. Provider tests SHALL use temporary directories only.

### OSC-011 — Trash, Archives, and File Recovery

**Priority:** P0 — Required

#### Acceptance Criteria

1. User-requested deletion SHALL move eligible files to the desktop Trash by default.
2. Permanent deletion SHALL be a distinct RED action with explicit wording and no rollback claim.
3. Trash operations SHALL preserve original path and collision-safe restore metadata where available.
4. Restore SHALL fail safely when the original target is occupied and SHALL require a user-selected resolution.
5. Archive create, list, inspect, and extract SHALL enforce path, entry-count, expanded-byte, compression-ratio, and traversal limits.
6. Archive extraction SHALL stage output and verify destination boundaries before commit.
7. Unsupported archive formats SHALL return structured unsupported results and MAY hand off to a trusted application.

### OSC-012 — Storage and Removable Media

**Priority:** P0 — Required

#### Acceptance Criteria

1. KRIA SHALL discover mounted filesystems, removable devices, capacity, free space, filesystem type, mount state, and safe device identifiers.
2. Mount, unmount, and eject SHALL prefer UDisks2 and SHALL use typed device identifiers rather than caller-supplied device-node commands.
3. Busy unmount/eject SHALL report blocking state without force by default.
4. Force unmount SHALL not be implemented in normal v1/v2 capability scope.
5. Storage health SHALL report available SMART/health evidence without performing repair or destructive tests.
6. Partition, format, resize, secure erase, and encryption-provisioning requests SHALL be BLACK and handed off to trusted system utilities without automation.
7. All storage mutations SHALL verify mount topology after the action.
8. v2 backup support SHALL integrate only with recognized existing backup providers for discovery, status, start, and restore handoff; KRIA SHALL not implement an independent backup engine or claim backup authority.

### OSC-013 — Applications, Intents, and Processes

**Priority:** P0 — Required

#### Acceptance Criteria

1. KRIA SHALL discover installed desktop applications from authoritative desktop entries and normalize stable application IDs.
2. Launch, open-with-file, close, and process termination SHALL distinguish application intent from process identity.
3. Graceful close SHALL be preferred; forced process kill SHALL be a distinct RED or escalated action and SHALL not claim rollback.
4. Default process inspection SHALL include PID-plus-start-time identity, bounded redacted executable identity, owner reference, state, CPU, memory, and start time, and SHALL exclude argv, environment, current directory, open files, and other command content.
5. Command arguments MAY be returned only by a separate explicitly requested RED capability with a bounded purpose, mandatory approval, strict truncation, and content-free approval/audit projection; environment and current directory SHALL never be returned by that capability.
6. Process schemas SHALL contain an explicit command-metadata state (`NotRequested`, `Unavailable`, `PermissionDenied`, or redacted digest/count metadata) rather than conditionally adding sensitive fields.
7. Raw command-argument results SHALL be ephemeral to the explicitly requested current turn, SHALL be cleared on consumption/turn completion/cancellation/timeout/session teardown, and SHALL be rejected by conversation history, tool-result persistence, memory, search, workflow, receipt, audit, trace, analytics, and crash-report sinks.
8. Priority changes SHALL validate allowed ranges, capture prior priority, verify the result, and advertise rollback.
9. Default application, MIME association, and user autostart mutations SHALL preserve prior associations for rollback.
10. KRIA SHALL not implement arbitrary cgroup administration, CPU affinity, or resource-limit policy in v1/v2.

### OSC-014 — Software Packages and Routine Updates

**Priority:** P0/P1 — Required install/remove; required assessment and recommended update apply

#### Acceptance Criteria

1. Package operations SHALL normalize manager, repository source, package identity, installed version, candidate version, origin, size, dependencies summary, and reboot implications.
2. Search and inspection SHALL be read-only and bounded across available providers.
3. Install, remove, and update SHALL produce an exact preflight plan before approval.
4. Package mutation SHALL use PackageKit where semantics are sufficient or a typed distro adapter; provider choice SHALL be explicit.
5. The runtime SHALL distinguish installing an absent package, updating an installed package, removing a package, and performing no change.
6. Routine update assessment SHALL identify security relevance and reboot requirements without claiming unavailable metadata.
7. Applying updates and removals SHALL be RED, verified by fresh package state, and SHALL not claim automatic downgrade rollback.
8. Release upgrades, repository-key administration, repository mutation, kernel selection, and package-manager repair SHALL be out of normal scope.

### OSC-015 — Connectivity: Wi-Fi and Ethernet

**Priority:** P0 — Required

#### Acceptance Criteria

1. KRIA SHALL observe Wi-Fi radio state, adapters, nearby networks, active profile, signal, security class, connectivity, and saved-profile identity through NetworkManager when available; SSID/profile identity reads and scans SHALL be RED privacy-sensitive actions.
2. KRIA SHALL enable/disable Wi-Fi, connect/disconnect profiles, forget a saved profile, and select an existing Ethernet profile through typed operations.
3. Passwords SHALL enter providers only through a Secret_Reference or ephemeral protected input and SHALL never enter plans, logs, result payloads, or audit.
4. Connecting to an already-active desired profile SHALL be unchanged.
5. A connection change SHALL capture the prior active profile when available and MAY offer rollback by reactivating it.
6. Ambiguous duplicate SSIDs or devices SHALL require stable candidate selection.
7. Static IP, DNS, route, bridge, and low-level NetworkManager profile editing SHALL not be exposed in v1 core.
8. Connectivity operations SHALL be display-server neutral on X11 and Wayland.
9. v2 MAY expose a bounded hotspot desired state with generated credentials stored by Secret_Reference and exact disable/restore behavior.
10. v2 MAY select or clear recognized desktop proxy profiles, but SHALL not expose arbitrary route, bridge, NAT, or raw NetworkManager property editing.

### OSC-016 — Network Diagnostics, VPN, and Captive Portals

**Priority:** P1 — Required diagnostics; recommended VPN

#### Acceptance Criteria

1. KRIA SHALL expose one cohesive network diagnosis capability rather than independent model-facing ping, traceroute, DNS, and speed commands.
2. Diagnosis SHALL progress through link, address, route, gateway, DNS, internet reachability, captive portal, and optional bounded path evidence.
3. Diagnostic probes SHALL be read-only, bounded, privacy-aware, and shall not transmit private project/user content.
4. KRIA SHALL connect and disconnect already-configured VPN profiles without exposing credentials.
5. Creating arbitrary VPN profiles, managing certificates, and generating WireGuard/OpenVPN configuration SHALL be outside v1/v2 normal scope.
6. Captive-portal detection SHALL return a handoff URL only after validating the scheme and local desktop opener.
7. Diagnostic results SHALL distinguish local-link, address, gateway, DNS, portal, remote-host, and general-internet failures.

### OSC-017 — Firewall Posture

**Priority:** P1 — Recommended

#### Acceptance Criteria

1. KRIA SHALL inspect whether a supported host firewall is installed, active, and managed by a recognized high-level provider.
2. KRIA MAY enable or disable the firewall through a high-level provider; disabling SHALL be RED and always confirmed.
3. v2 MAY create time-bounded application-level grants only when the provider can label ownership, expiry, and exact inverse.
4. KRIA SHALL modify or revoke only temporary grants created by KRIA.
5. Raw iptables, nftables, firewalld rule syntax, port-forwarding, NAT, zones, and arbitrary rule editing SHALL be BLACK.
6. Firewall verification SHALL re-query effective high-level state and managed grants.

### OSC-018 — Audio and Media

**Priority:** P0/P2 — Core audio required; advanced media v2

#### Acceptance Criteria

1. KRIA SHALL observe and set default output volume, mute, selected output, default input volume, microphone mute, and selected input.
2. Audio observations SHALL normalize endpoint IDs, names, availability, level, mute, default status, and provider.
3. Microphone input selection, level increase, activation, or unmute SHALL be RED, privacy-sensitive, visible in audit without content capture, and always confirmed from a content-free projection.
4. Audio mutations SHALL capture prior endpoint/default/level/mute state and verify through PipeWire/WirePlumber or the selected provider.
5. If PipeWire APIs are unavailable, structured `wpctl`, `pactl`, then `amixer` fallbacks MAY be used with explicit degraded capability metadata.
6. v2 MAY control per-application streams, profiles/ports, and MPRIS playback through separate typed operations.
7. Audio control SHALL be independent of X11/Wayland except for user-session service availability.

### OSC-019 — Display and Brightness

**Priority:** P0/P2 — Brightness required; full topology v2

#### Acceptance Criteria

1. KRIA SHALL observe displays, stable connector/provider IDs, physical/internal status, brightness capability, active state, geometry, mode, scale, orientation, primary state, and provider where available.
2. Physical brightness SHALL be distinguished from software gamma.
3. XRandR gamma fallback SHALL be labeled degraded and SHALL never be attempted on Wayland.
4. v2 display mode, refresh, orientation, layout, primary, scale, and night-light changes SHALL capture a complete prior topology.
5. Risky topology changes SHALL start a rollback timer and restore prior topology unless the user confirms visibility before expiration.
6. Provider-specific Mutter, KScreen, wlroots, and XRandR behavior SHALL normalize into one desired-state contract.
7. If the compositor does not expose safe topology mutation, KRIA SHALL report unsupported and MAY open trusted display settings.
8. Display verification SHALL come from fresh compositor/display state, not screenshots or assumed command success.

### OSC-020 — Power, Battery, and Session Lifecycle

**Priority:** P0/P2 — Core lifecycle required; advanced battery v2

#### Acceptance Criteria

1. KRIA SHALL support lock, suspend, hibernate when available, shutdown, reboot, delayed shutdown, cancellation of KRIA-scheduled shutdown when available, logout, power-profile read/set, battery state, and reboot-required state.
2. logind SHALL be the preferred provider for session and power lifecycle.
3. Lock SHALL verify locked state when observable; suspend, hibernate, shutdown, reboot, and logout SHALL report `Accepted` after OS acceptance.
4. Session-ending and power actions SHALL use RED confirmation according to policy and SHALL not claim rollback after acceptance.
5. Hibernate availability SHALL be probed; missing swap/kernel/platform support SHALL return unavailable without fallback claims.
6. Power-profile changes SHALL capture and verify the prior/current profile.
7. Battery observations SHALL normalize percentage, charging, energy, health/capacity and cycle count when exposed.
8. v2 MAY set vendor-supported charge thresholds only through a recognized adapter with exact prior-state rollback.
9. KRIA SHALL not manipulate CPU voltage, overclocking, fan curves, embedded controllers, or arbitrary power sysfs files.

### OSC-021 — Bluetooth and Common Devices

**Priority:** P0/P2 — Bluetooth required; peripherals recommended

#### Acceptance Criteria

1. KRIA SHALL use BlueZ to observe adapters, radio state, discovery state, devices, paired/trusted/connected state, class, services, and battery when exposed; nearby-device identity reads and scans SHALL be RED privacy-sensitive actions.
2. KRIA SHALL enable/disable Bluetooth, scan for a bounded interval, pair, confirm pairing, connect, disconnect, trust where required, and remove devices.
3. Pairing secrets or passkeys SHALL use the existing approval/presentation path and SHALL not be persisted by KRIA.
4. Ambiguous devices SHALL require stable device identity selection; display name alone is insufficient.
5. Removal and trust changes SHALL be RED and verified.
6. Printer support SHALL use CUPS/IPP for discovery, queue inspection, submit, cancel owned jobs, and bounded printer setup when supported.
7. v2 MAY add scanner integration and hardware sensor discovery.
8. Thunderbolt authorization, dock firmware/control, game-controller configuration, and vendor device administration SHALL remain deferred.

### OSC-022 — System Health, Logs, and Recovery

**Priority:** P0/P1 — Required health; recommended diagnostics/recovery

#### Acceptance Criteria

1. KRIA SHALL retain bounded CPU, memory, filesystem, network, battery, uptime, process, and GPU observations.
2. KRIA SHALL add resource-pressure, thermal, fan, sensor, failed-service, recent-error, storage-health, and reboot-required observations when safe providers expose them.
3. Logs SHALL be queried by bounded time, source, severity, and count; untrusted content SHALL be escaped and treated as data.
4. A diagnostic capability SHALL correlate observations without inventing causality.
5. Recovery recipes SHALL be fixed, versioned, allowlisted sequences for desktop subsystems and SHALL not accept arbitrary commands.
6. Each recovery recipe SHALL declare preconditions, risk, resources, steps, verification, compensation, and unsupported states.
7. KRIA SHALL not create/edit arbitrary systemd units or apply kernel/security-policy changes as recovery.
8. v2 firmware awareness SHALL be read-only: discover firmware versions, query trusted `fwupd` update availability, report power/reboot prerequisites, and hand off to a trusted updater; this specification SHALL not execute firmware flashing.

### OSC-023 — Clipboard and Notifications

**Priority:** P0/P1 — Required core; recommended history/DND

#### Acceptance Criteria

1. KRIA SHALL read and write the current clipboard through a provider compatible with supported X11 and Wayland session constraints.
2. Clipboard reads SHALL be RED, privacy-sensitive, user-intent-bound, bounded, approved from a content-free purpose/scope projection, and excluded from audit content.
3. Clipboard writes SHALL capture prior content only when rollback policy permits sensitive in-memory retention.
4. Clipboard history SHALL be opt-in, encrypted at rest, bounded by count/bytes/TTL, exclude configured applications/types, and support immediate clear.
5. KRIA SHALL send desktop notifications with bounded title/body, urgency, actions, correlation ID, and expiry where supported.
6. Notification actions SHALL re-enter KRIA as authenticated local events and SHALL not directly execute mutations.
7. Notification history and Do Not Disturb SHALL preserve provider limitations honestly.
8. Clipboard or notification unavailability SHALL never cause GUI automation fallback.

### OSC-024 — Local Desktop Search

**Priority:** P1 — Required for v1 completion

#### Acceptance Criteria

1. KRIA SHALL maintain a local, rebuildable SQLite/FTS search index for user-authorized roots.
2. Search SHALL support filename, metadata, type, modified time, and bounded text content where extraction is permitted.
3. Indexing SHALL honor filesystem permissions, explicit exclusions, secret/private locations, file-size/type limits, symlink policy, and resource budgets.
4. Index expansion SHALL require explicit approval; default scope SHALL not include the entire root filesystem.
5. Search results SHALL preserve path provenance and SHALL not imply content was indexed when only metadata exists.
6. File watcher events SHALL update the projection idempotently; the index SHALL remain disposable and rebuildable.
7. Search content SHALL remain local and SHALL not be transmitted externally.

### OSC-025 — Secrets and Credentials

**Priority:** P0 — Required

#### Acceptance Criteria

1. KRIA SHALL integrate with the freedesktop Secret Service or return unavailable; it SHALL not persist plaintext fallback secrets.
2. Tools and plans SHALL exchange Secret_References, metadata, purpose, scope, and expiry rather than secret values.
3. A provider MAY resolve a secret only for an admitted action whose grant names the required purpose and scope.
4. Secret values SHALL not implement `Debug`, serialize through normal DTOs, clone unnecessarily, or appear in tracing/audit/errors.
5. Secret buffers SHALL be kept for the minimum operation duration and cleared where the language/library permits.
6. Store, replace, reveal-to-user, export, and delete SHALL be RED and separately authorized according to policy.
7. A locked or unavailable secret service SHALL produce actionable failure without password interception.

### OSC-026 — Skill Sandbox and Capability Grants

**Priority:** P0 — Required

#### Acceptance Criteria

1. OpenClaw skills SHALL request typed domain-operation grants with explicit resource scope, duration, source identity, and network/filesystem limits.
2. Skills SHALL not receive direct access to Host_OS_Provider, Privilege_Broker, session bus, system bus, host D-Bus sockets, arbitrary device nodes, or raw shell.
3. Grant creation or escalation SHALL require policy evaluation and approval.
4. Each invocation SHALL revalidate grant scope, expiry, skill identity, action parameters, and current risk.
5. Revocation SHALL take effect before subsequent provider calls and cancel queued actions where safe.
6. Skill outcomes SHALL pass through the same verification and audit contracts as native tools.
7. Sandbox configuration SHALL remain deny-by-default when a capability is unknown.

### OSC-027 — Structured Automation

**Priority:** P1 — Required for v1 completion

#### Acceptance Criteria

1. Scheduled and event-driven workflows SHALL store typed capability ID, validated parameters, target, conditions, and policy metadata, never shell strings.
2. At execution time, automation SHALL repeat capability discovery, target binding, policy, approval/grant, resource, and secret checks.
3. Approval SHALL not be inherited indefinitely; reusable grants SHALL be explicit, scoped, revocable, and expiring.
4. Automation SHALL define cancellation and compensation for each step and report partial completion.
5. Power, permanent deletion, package/update, firewall weakening, credential, microphone-privacy, and other RED actions SHALL not run unattended without an explicit approved grant that permits unattended execution.
6. Event subscriptions SHALL be bounded, deduplicated, and resilient to provider restarts.
7. Existing cron jobs MAY be listed and basic user cron retained, but new KRIA automation SHALL use the typed workflow authority.

### OSC-028 — Recovery and Undo

**Priority:** P1 — Recommended

#### Acceptance Criteria

1. KRIA SHALL expose undo only for receipts declaring valid rollback.
2. Display topology SHALL use timed automatic rollback rather than relying solely on user-requested undo.
3. Recovery recipes SHALL be fixed code/data definitions reviewed with their provider implementation.
4. A recovery request SHALL diagnose prerequisites before proposing mutation.
5. Recipes SHALL not include arbitrary shell, arbitrary service names, kernel changes, security-policy changes, destructive storage, or user administration.
6. IF a recipe cannot verify recovery, it SHALL report unverified and stop rather than repeat destructively.
7. KRIA SHALL preserve the original failure and rollback failure separately.

### OSC-029 — Privacy Controls and Data Retention

**Priority:** P0/P1 — Required policy; recommended controls

#### Acceptance Criteria

1. Each domain SHALL classify observed values as public-local, sensitive metadata, secret, content, or prohibited.
2. Clipboard history, notification history, search index content, logs, nearby device data, SSIDs, process command lines, and application titles SHALL have explicit retention limits.
3. Users SHALL be able to inspect and clear KRIA-owned clipboard, notification, search, and OS-action records subject to audit integrity constraints.
4. Current-user microphone, camera, location, and related privacy controls MAY be exposed only through reversible recognized providers.
5. KRIA SHALL not silently activate microphone/camera devices while changing privacy settings.
6. Data unavailable due to privacy restrictions SHALL remain unavailable and SHALL not trigger a bypass through GUI or shell.

### OSC-030 — Explicit Prohibited Scope

**Priority:** P0 — Required safety boundary

#### Acceptance Criteria

1. THE normal assistant SHALL not expose partitioning, formatting, filesystem resizing, secure erase, full-disk encryption provisioning, bootloader/Secure Boot mutation, kernel installation/selection/tuning/modules, full user/group/password/sudo administration, SELinux/AppArmor policy editing, CA/PKI administration, raw firewall rules, vendor firmware flashing, fan/embedded-controller writes, overclocking, or arbitrary systemd-unit creation.
2. Requests for prohibited operations SHALL return a concise boundary explanation and MAY offer read-only diagnostics or opening a trusted specialist utility.
3. Opening a specialist utility SHALL not grant KRIA control over the prohibited operation.
4. Providers, tools, routes, aliases, automation, recovery recipes, skills, and privileged broker schemas SHALL contain no generic primitive that reconstructs prohibited scope.
5. Generic Expert Mode shell remains separately governed and SHALL never be selected automatically for prohibited requests.

### OSC-031 — Ubuntu and Future-Version Compatibility

**Priority:** P0 — Required

#### Acceptance Criteria

1. KRIA SHALL target supported Ubuntu desktop releases beginning with 24.04 and SHALL avoid behavior selected solely by distribution version.
2. Provider choice SHALL rely on runtime service ownership, interface/method/property probes, desktop/session context, and semantic verification.
3. D-Bus decoders SHALL tolerate additive properties and unknown enum values without panic.
4. CLI parsers SHALL be version-tolerant, locale-controlled where possible, bounded, and fail closed on ambiguity.
5. Dependencies SHALL be exact-version pinned in the workspace and locked.
6. If a future Ubuntu release removes or changes a provider, only the affected capability SHALL degrade; unrelated domains SHALL remain available.
7. Capability-unavailable responses SHALL identify the failed interface/provider and safe remediation without promising universal future compatibility.

### OSC-032 — X11 and Wayland Support

**Priority:** P0 — Required

#### Acceptance Criteria

1. Network, Bluetooth, audio, packages, files, processes, storage, power, health, search, secrets, automation, and printing SHALL use display-server-neutral semantics.
2. Display and clipboard providers SHALL declare explicit X11 and Wayland support per operation.
3. X11-only providers such as XRandR or xdotool SHALL not execute in a native Wayland path.
4. XWayland availability SHALL not be treated as full Wayland desktop authority.
5. GNOME Wayland unsupported operations SHALL return a blocker and trusted-settings handoff rather than false success.
6. Provider contract tests SHALL cover GNOME X11 and GNOME Wayland capability matrices; KDE/wlroots adapters SHALL remain optional and independently discoverable.
7. Session environment variables SHALL not be fabricated to force provider access.

### OSC-033 — Non-Disruptive Code-Level Testing

**Priority:** P0 — Required

#### Acceptance Criteria

1. All domain handlers and providers SHALL be dependency-injected and testable without live OS mutation.
2. Unit tests SHALL use scripted provider fakes, fake clocks, fake authorization, fake D-Bus proxy interfaces or private test buses, captured command requests, temporary filesystems, and in-memory SQLite.
3. Tests SHALL not invoke live suspend, hibernate, logout, shutdown, reboot, Wi-Fi, VPN, Bluetooth, firewall, package, update, display topology, audio routing, microphone, mount/eject, printer, clipboard, notification, or secret mutations.
4. Tests SHALL not require an active X11/Wayland display, system bus, session bus, NetworkManager, BlueZ, PipeWire, UDisks2, logind, Polkit, CUPS, or Secret Service.
5. Parser tests SHALL include malformed, truncated, oversized, localized, additive-field, unknown-enum, timeout, and adversarial inputs.
6. Handler tests SHALL assert routing, target, risk, approval metadata, resource declarations, provider call, verification, redaction, and result envelope.
7. Separate live/manual acceptance testing SHALL not be a completion criterion in this specification.
8. ALL completion-test binaries SHALL be compiled in a deny-live composition that is mutually exclusive with live provider construction, including integration tests whose library dependency is not compiled with `cfg(test)`.

### OSC-034 — Code Quality and Bounded Operation

**Priority:** P0 — Required

#### Acceptance Criteria

1. All provider traits and DTOs SHALL be documented, typed, `Send + Sync` where shared, and free of provider-specific leakage at the tool boundary.
2. Each scan/list operation SHALL define maximum entries, bytes, duration, concurrency, and cancellation behavior.
3. Retries SHALL be bounded, operation-specific, and never repeat uncertain mutations.
4. No provider SHALL panic on OS data, unknown properties, disappearing devices, stale object paths, invalid UTF-8, or service restarts.
5. Every new dependency SHALL be FOSS, exact-pinned, justified, and used only when existing dependencies cannot satisfy the need.
6. Focused `cargo fmt --check`, relevant unit tests, `cargo check -p kria-core`, and relevant Clippy checks SHALL pass before a task is marked complete.
7. Full workspace/E2E/live hardware suites are outside this spec unless separately authorized.

### OSC-035 — Existing-Code Integration and Hard Cutover

**Priority:** P0 — Required

#### Acceptance Criteria

1. Existing OS tool facades SHALL delegate to the new provider runtime rather than coexist with direct Linux execution.
2. Current useful parsers SHALL move into fallback adapters with tests instead of being duplicated.
3. `ToolRegistry` and `ToolContext` SHALL receive the host provider through existing injection patterns.
4. `system_config.rs` SHALL retain environment-variable behavior separately while delegating OS state controls.
5. `power.rs` SHALL contain no Linux `sh -c`, direct shutdown/reboot shell, or VM dispatch path after cutover.
6. Tauri/Axum SHALL remain thin and SHALL consume existing approval and tool-stream contracts.
7. Superseded helpers, aliases, tests, comments, and dependencies SHALL be deleted after parity; no deprecated compatibility shim is required in this pre-production codebase.

### OSC-036 — End-to-End Code Completion

**Priority:** P0 — Release gate

#### Acceptance Criteria

1. Every Required and Recommended v1 capability SHALL be reachable from a representative natural-language prompt through canonical routing and a registered tool.
2. Every route SHALL pass through target binding, policy, resource declarations, injected provider, verification, audit redaction, and stable result adaptation in an in-process fake-backed test.
3. Every v2 capability included in the task plan SHALL meet the same code contract before marked complete.
4. Every Deferred capability SHALL remain unimplemented behind an explicit boundary, and every Out-of-Scope capability SHALL remain BLACK.
5. The specification is implementation-complete only when requirement-to-design-to-task traceability has no missing Required/Recommended item and all listed code-level tests pass.
6. Live feature validation, disruptive testing, and hardware acceptance SHALL be tracked separately and SHALL not be falsely claimed by this specification.

## Completion Definition

This specification is ready for implementation when:

- All OSC requirements map to concrete modules, providers, risks, verification, rollback, and tasks.
- The tasks define dependency order, failure behavior, non-disruptive tests, and completion proof.
- Existing tool/event contracts and current code migration are explicit.
- Ubuntu compatibility is based on capability negotiation, not impossible promises about unknown future releases.
- No daily desktop domain depends on generated shell.
- Deferred and prohibited scope cannot leak through generic provider or broker primitives.
