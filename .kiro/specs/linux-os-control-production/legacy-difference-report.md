# Legacy-Difference Inventory — Task 0.1

**Feature:** `linux-os-control-production`
**Task:** 0.1 — Freeze the canonical capability and tool contract inventory
**Status:** Inventory only. This report changes **no** runtime code, registrations,
handlers, or Tauri/WebSocket command/event names. It records how today's live tool
surface differs from the frozen §§10.1–10.4 manifest so the owning later tasks can
perform the hard cutover deliberately.

## Scope and method

- **Normative target:** the 149-operation closed manifest in
  `operation-contracts.json` (the §10.4 projection of design §§10.1–10.3),
  `manifestVersion: 1`.
- **Live surface examined (read-only):** `crates/kria-core/src/tools/registry.rs`
  and the sibling tool modules under `crates/kria-core/src/tools/*`, plus the
  router/fallback/policy references that consume tool names, risk tiers, and resume
  metadata.
- **Method:** static scan of `ToolDef { name: "…" }` declarations plus targeted
  reading of the OS-relevant modules (`power.rs`, `system_config.rs`, `process.rs`,
  `packages.rs`, `disk.rs`, `mount_manager.rs`, `system_info.rs`, `file_ops.rs`,
  `shell.rs`, `exec.rs`). The scan is best-effort for discovery; every classified
  entry below was confirmed by reading the module. Non-OS tool families (browser,
  vision/screen, GUI automation, knowledge/RAG, documents, internet, news, weather,
  i18n, image generation, Google Workspace, developer/IDE, snippets, workflow
  sessions) are **out of scope** for this spec and are intentionally left unchanged.

Each difference is classified as one of:

- **match** — a canonical tool name already exists live with the same spelling and
  is a *candidate* for reuse. This is **not** a claim that its schema, risk resolver,
  provider wiring, verification, or metadata already conform; those remain owned by
  Task 1.2 and the domain tasks.
- **replace** — a live name, schema, tier, resume, router, or execution path that
  must be renamed, reshaped, or rerouted to the canonical contract.
- **delete** — a live path (raw shell / direct OS execution / non-canonical alias)
  that is superseded and removed after provider parity.

No live difference is silently normalized: name/spelling corrections require a spec
amendment (none were required — see "Naming" below), and structural conformance is
enforced later by the strict registry validator (Task 1.2).

## 1. Name-level inventory (149 canonical tools)

### 1.1 match — canonical name already present live (45)

These names already appear in the live registry and are reuse candidates. Schema,
risk, provider, verification, rollback, redaction, and trace metadata are **not**
asserted here.

```
calculate_dir_size      check_system_health     connect_wifi            copy_file
create_directory        create_scheduled_task   delete_file             delete_scheduled_task
get_battery_status      get_clipboard           get_cpu_usage           get_disk_space
get_file_info           get_gpu_info            get_memory_info         get_package_info
get_power_plan          get_system_uptime       get_wifi_networks       hibernate
install_package         kill_process            list_directory          list_installed_packages
list_running_apps       list_scheduled_tasks    lock_screen             move_file
open_application        read_file               reboot_system           rename_file
search_files            search_package          send_notification       set_brightness
set_clipboard           set_power_plan          set_process_priority    set_volume
shutdown_system         sleep                   toggle_wifi             uninstall_package
```

Notes on match entries that still require a **replace** at the schema/execution
level (name matches, contract does not):

- `shutdown_system` — live input is `delay_minutes`; canonical input is
  `delay_seconds?:u32`. **replace (schema)** — owned by the power domain task (2.4)
  under Task 1.2 registry rules.
- `sleep`, `hibernate`, `reboot_system`, `shutdown_system`, `lock_screen` — live
  implementations in `power.rs` shell out via `sh -c` / direct commands. The names
  match but the execution path is **delete** (see §3); provider-backed
  re-implementation is owned by the power domain task (2.4) and the direct-path
  deletion by Task 2.6.
- `set_volume`, `set_brightness`, `set_power_plan`, `toggle_wifi`, `connect_wifi`,
  `kill_process`, `set_process_priority`, `install_package`, `uninstall_package`,
  `create_directory`, `copy_file`, `move_file`, `rename_file`, `delete_file`,
  `write_file`, `read_file`, etc. — name matches only; the flat `ParamDef` schema,
  single `default_tier` risk, and direct/`LocalEnvironment` execution are all
  **replace** items (see §2) owned by the domain tasks and Task 1.2.

### 1.2 absent — canonical names with no live counterpart (104)

The remaining 104 canonical names have no matching live registration and are new
additions owned by their domain tasks (per each manifest row's `taskId`). They are
not enumerated individually here; the machine list is the set difference
`canonical − live` computed by the freeze test's inputs.

## 2. Structural differences (schema / tier / resume / router / provider)

These are repository-wide **replace** items owned by Task 1.2 (strict registry
metadata) and the domain tasks; Task 0.1 only records them.

| Live construct | Canonical target | Class | Owning task |
|---|---|---|---|
| Flat `ParamDef { name, param_type, description, required, default }` | Closed nested/enum/bounded JSON schema with `additionalProperties:false` at every object | replace | 1.2 |
| `ToolDef` with `default_tier: RiskLevel` (single tier) | Total per-operation risk resolvers (`risk.fixed.*`, `risk.path_scope.*`, `risk.conditional.*`, etc.) | replace | 1.2 |
| `ToolDef` without contract metadata | Single `ToolContractMetadata` (output/target/resume/resource/provider/risk/verification/rollback/redaction/trace/oracle) | replace (add) | 1.2 |
| Parallel `ToolResumeCapability` enum + external resume map | Per-operation `ResumePolicy` inside the contract metadata | replace | 1.2 |
| Handlers over `LocalEnvironment` / direct `EnvironmentProvider` | OS handlers receive injected `Arc<OsControlRuntime>`; raw `HostOsControl` stays private | replace | 1.2 + domain tasks |
| `ToolRegistry::register` overwrites on duplicate | Typed error on duplicate definition/handler/alias or inconsistent metadata | replace | 1.2 |
| No manifest snapshot / trace linkage | Exact §§10.1–10.4 manifest snapshot with reverse-orphan and oracle checks | replace (add) | 1.2 |

`system_config.rs` retains environment-variable behavior (`get/set/list_environment_variable`)
as a **separate, out-of-scope** concern per OSC-035.4; its OS state controls migrate
to providers. This is recorded, not changed here.

## 3. Direct-execution and raw-shell paths (delete)

Superseded direct OS execution paths, removed after provider parity (deletion owned
by Task 2.6; power specifics by OSC-035.5):

| Live path | Reason | Class | Owning task |
|---|---|---|---|
| `power.rs` `sh -c` for `sleep`/`hibernate`/`shutdown_system`/`reboot_system`/`lock_screen` | No Linux `sh -c` or direct shutdown/reboot shell after cutover (OSC-035.5) | delete | 2.4 impl, 2.6 delete |
| Direct `Command`/`ExecWrapper`/`sh -c` in OS providers/handlers | Replaced by one host-bound argv executor borrowing `AdmittedMutationContext` | replace→delete | 1.4, 2.6 |
| Raw command/output audit fields | Deleted; audit is redacted structured metadata + digests only | delete | 1.4, 1.8 |

## 4. Non-canonical OS-adjacent names (replace / delete / out-of-scope)

| Live name | Disposition | Class | Note / owning task |
|---|---|---|---|
| `get_network_status` | → `get_network_state` | replace (rename) | 2.3 connectivity |
| `check_package_updates` | → `check_system_updates` | replace (rename) | 3.x packages |
| `check_package_installed` | folded into `get_package_info` / `list_installed_packages` | delete | 3.x packages |
| `close_application` | → `graceful_close_application` | replace (rename) | 2.x apps |
| `open_application_with_file` | → `open_with_application` | replace (rename) | 2.x apps |
| `list_files` | → `list_directory` (canonical) | delete (alias) | file domain |
| `delete_directory` | superseded by `delete_file` (trash) / `delete_permanently` | delete | file domain |
| `clean_temp_files` | composite direct action; superseded by file provider ops | delete | file domain |
| `execute_bash` | generic shell; not a native OS-control tool (§10.4). Gated to Expert Mode / removed from OS-admin routing | delete (from OS routing) | 0.2 / 0.3 |
| `execute_powershell`, `execute_python`, `execute_fleet_command` | generic/remote execution; outside native host OS-control scope | delete (from OS routing) | 0.2 / 0.3 |
| `get_active_connections` | network-connections view; distinct subsystem from canonical connectivity DTOs | observe (out of scope for v1 manifest) | — |
| `get_environment_variable`, `set_environment_variable`, `list_environment_variable` | retained separately in `system_config.rs` (OSC-035.4) | out of scope | — |

## 5. Alias and drift check

- **No canonical alias exists.** No live alias maps an OS-administration request to
  Bash or to another canonical name in a way that conceals drift. Alias removal that
  routes OS administration to shell is owned by Task 0.2.
- **No BLACK operation is present** in the manifest (verified by the freeze test):
  partitioning, formatting, GRUB/kernel, user administration, raw firewall rule
  editing, firmware flashing, fan control, PKI, SELinux/AppArmor, and systemd-unit
  creation are absent, not aliased.
- **No Tauri/WebSocket command or event name is changed** by this task.

## 6. Naming corrections requiring amendment

None. Every canonical tool name in §§10.1–10.3 is internally consistent and matches
its §10.4 projection in `operation-contracts.json`; the freeze test confirms the
exact 149-name set agrees in both directions. No spelling correction (which would
require a spec amendment before 0.1 closes) was necessary.

## 7. Summary

- 45 canonical names already exist live (reuse candidates); 104 are new.
- Live schema/risk/resume/router/provider constructs are uniformly **replace** items
  owned by Task 1.2 and the domain tasks.
- Direct `sh -c` / raw-shell / direct-`Command` OS paths and non-canonical aliases
  are **delete** items owned by Tasks 1.4/2.4/2.6 (and 0.2/0.3 for shell/routing
  containment).
- No drift is concealed, no BLACK operation is present, and no Tauri/WebSocket
  contract name changes here. Strict runtime registry enforcement remains owned by
  Task 1.2.
