# OpenClaw — Sandbox Review

> Question set: can a bad skill damage KRIA, escape, read files/secrets, reach network/GPU,
> exhaust RAM/CPU, DoS, fork-bomb, orphan processes, leak containers/volumes/temp, bypass perms?

## 1. Container profile (as-built)

From `pool.rs::create_container_static`:

```
readonly_rootfs: true
network_mode:    "none"
security_opt:    ["no-new-privileges:true"]
cap_drop:        ["ALL"]
tmpfs:           /workspace = size=256M
memory:          256M / 512M / 2G  (Light/Medium/Heavy)
nano_cpus:       0.5 / 1.0 / 2.0
USER:            node (image)
cmd:             node --max-old-space-size=256 src/mcp-bridge.js
```

This is a **strong default profile**. Assessment per question:

| Question | Verdict | Notes |
|----------|---------|-------|
| Damage KRIA host FS? | **Low** | readonly rootfs + only tmpfs writable; no host bind mounts |
| Escape container? | **Medium** | Shared-kernel Docker; cap_drop ALL + no-new-privileges reduce risk, but **no seccomp profile** applied and no gVisor/microVM for Untrusted tier |
| Read arbitrary host files? | **Low** | No bind mounts; readonly rootfs |
| Access secrets? | **Low** | No env secrets injected; net=none prevents exfil |
| Network access? | **None (today)** | `network=none` hardcoded — but this also breaks legit skills |
| GPU access? | **None** | No device mapping; GPU skills impossible |
| Unlimited RAM? | **No** | cgroup memory limit + node heap cap; OOM → exit 137 |
| DoS host CPU? | **Bounded** | nano_cpus per class; but **no global budget vs voice/vision** (HRA gap) |
| Fork forever? | **Partial** | cap_drop + mem limit constrain; **no explicit PID limit (`pids_limit`)** set → fork-bomb within mem budget possible |
| Orphan processes? | **Medium** | `docker attach` child has `kill_on_drop`; in-container PID1 is node; container force-removed on checkin |
| Leak containers? | **Medium** | `events.rs` unused → crash between checkouts leaves containers until next `is_container_healthy` probe; `adopt_existing_containers` prunes on boot only |
| Leak volumes? | **Low** | tmpfs only; vanishes with container |
| Leak temp files? | **Low** | workspace is tmpfs inside container |
| Bypass permissions? | **N/A today** | No permissions are granted to bypass (caps not materialized) |

## 2. Findings

### SBX-1 (High) — No `pids_limit`
A skill can fork within its memory budget; no PID cap. Add `pids_limit` (e.g. 128) per class.

### SBX-2 (High) — No seccomp profile on substrate containers
Repo ships `config/seccomp/kria-seccomp.json` but it is **not applied** to OpenClaw
containers. Shared-kernel escape surface is larger than necessary. Apply a tailored seccomp
profile (`security_opt: seccomp=...`).

### SBX-3 (High) — Container leak on crash (events.rs dead)
`DockerEventSubscriber` (die/oom/kill/stop + reconnect + sequence CAS) is fully implemented
but never instantiated. The pool detects unhealthy containers only lazily at next checkout.
Wire the subscriber into the pool to proactively remove dead containers and re-warm.

### SBX-4 (Medium) — Shared-kernel isolation for Untrusted tier
Docker is fine for Verified/Community-with-review, but for `TrustTier::Untrusted` (and
prompt-generated skills, roadmap 9.3), shared-kernel is weak. Offer a **gVisor
(`runsc`)** runtime or **Firecracker microVM** option selected by trust tier.

### SBX-5 (Medium) — No ulimit / disk-quota beyond tmpfs size
tmpfs is capped at 256M (good), but no `ulimit` (nofile, nproc) set. Add ulimits.

### SBX-6 (Low) — Capabilities not materialized (design tension)
Because no caps are granted, the sandbox is *safe* but *useless* for write/subprocess/network
skills. When grants are added (SEC-4), each grant must be minimal and per-invocation, not a
relaxed base image. Keep the deny-by-default base; layer grants via mounts/egress-proxy/devices
only for the approved set.

## 3. How production systems solve this

- **microVM (Firecracker)** — OpenAI/Fly-style: per-task VM, hardware isolation, ~125ms boot.
  Best for untrusted; heavier.
- **gVisor (`runsc`)** — user-space kernel; strong syscall isolation with container ergonomics.
  Good middle ground; recommend for Community/Untrusted tiers.
- **Per-task egress proxy + CNI allowlist** — default-deny network, explicit domains.
- **cgroups v2 full budget** — pids, memory, cpu.weight, io — centrally scheduled.
- **Read-only base + explicit tmpfs/mount grants** — KRIA already does the base correctly.

## 4. Recommended sandbox tiers

| Trust tier | Runtime | Network | Caps |
|------------|---------|---------|------|
| Verified | Docker + seccomp + pids/ulimit | egress allowlist if declared | minimal grants, materialized |
| Community | gVisor (`runsc`) + seccomp | egress allowlist + per-domain HITL | minimal grants |
| Local/Untrusted | Firecracker microVM (or gVisor) | none unless HITL | none by default |

**Bottom line:** the *base* container is one of the better parts of the codebase. The gaps are
`pids_limit`, seccomp, the unused crash-recovery subscriber, and a trust-tiered runtime for
untrusted skills. None require redesign — they are additions to a correct foundation.
