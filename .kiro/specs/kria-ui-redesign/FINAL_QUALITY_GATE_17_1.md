# Task 17.1 — Final Quality Gate

Date: 2026-07-17
Requirements: 16, 17, 18, 20
Outcome: **FAILED / BLOCKED — GNOME Wayland native tranche exercised; required Linux matrix remains incomplete**

## Scope and method

This report combines the previously green automated gate with a real Tauri/WebKitGTK run in the active GNOME Wayland session. It does **not** claim GNOME X11, KDE Wayland, KDE X11, a second display, mixed DPI, fractional scale, screenshot review, focus-ring visual review, or manual visual parity. `npx codescout-cli pack ... --json` again timed out after 120 s; existing scoped CodeScout context and `ui/.codescout/graph.json` were used. Stale high-CPU CodeScout processes were terminated before profiling.

## Build and targeted validation

| Gate / command | Result |
|---|---|
| `cargo check -p kria-desktop` | **PASS** before native run; post-startup-fix check also **PASS**. No later Rust change required another run. |
| Native debug build | **PASS**, including rebuild after startup fix. |
| `npm run test:run -- src/stores/machineStore.test.ts` | **PASS**, 3 tests for poll snapshot equality. |
| `npm run test:run -- src/components/CorePresence.test.tsx src/stores/machineStore.test.ts` | **PASS**, 2 files / 13 tests. Existing Solid disposal warnings only. |
| `npm run check` | **PASS** after final frontend fixes. |
| `npm run build` | **PASS**, 396 modules / 4.21 s. Existing malformed CSS-comment and 687.03 kB vendor-chunk warnings remain. |
| Diagnostics on changed frontend files | **PASS**, no issues. |

Previous full automated evidence remains valid: unit suite 153 files / 1277 tests; combined Chromium + WebKit E2E coverage passed all 46 cases after selector correction and Vite warmup; axe WCAG A/AA passed all seven Spaces in both engines. Those browser/unit gates were not repeated for this native tranche except where the new native performance fixes required targeted tests.

## Native environment

- Ubuntu GNOME Shell 46.0, `XDG_SESSION_TYPE=wayland`, `XDG_CURRENT_DESKTOP=ubuntu:GNOME`, `WAYLAND_DISPLAY=wayland-0`, `DISPLAY=:0`.
- WebKitGTK 2.52.3; native inspector identified title `K.R.I.A.` and dev URL `http://localhost:1420/`.
- Rust/Cargo 1.95.0; KRIA/Tauri app version 0.1.0.
- NVIDIA GeForce RTX 4050 Laptop GPU, driver 580.159.03; Intel UHD Graphics 770 also present. `intel_gpu_top` unavailable.
- NVIDIA Wayland safe baseline active: DMABUF disabled while accelerated compositing remained enabled.
- Mutter renderer: `native`.

## GNOME X11 continuation attempt — blocked by verified session facts

The continuation request labeled the current desktop GNOME X11. The active environment and login manager contradict that label, so no GNOME X11 result is claimed and no X11-only capture/input behavior was inferred from XWayland.

| Exact command | Relevant result |
|---|---|
| `env` | `XDG_SESSION_TYPE=wayland`; `GDK_BACKEND=wayland`; `WAYLAND_DISPLAY=wayland-0`; `DISPLAY=:0`; `XDG_CURRENT_DESKTOP=ubuntu:GNOME`; `XAUTHORITY=/run/user/1000/.mutter-Xwaylandauth.TGXJS3` |
| `loginctl list-sessions` | One active local session: ID `2`, user `obaid`, seat `seat0`, TTY `tty2`. |
| `loginctl show-session 2` | `Type=wayland`, `Active=yes`, `State=active`, `Remote=no`, `Service=gdm-password`, `VTNr=2`. |

`DISPLAY=:0` is the Mutter XWayland compatibility endpoint in this environment; it does not make the host session X11. Launching with `GDK_BACKEND=x11` here would exercise an XWayland client inside GNOME Wayland, not the required GNOME-on-Xorg matrix cell. Therefore build/launch/reactivation, seven Spaces, palette keyboard behavior, approvals/Stop, modes, detach/Mini, tray/hotkey/AOT, X11 capture/input, service and lens degradation, accessibility, resource profiling, bounded load, cancellation, display/scaling, and screenshots were **not rerun or relabeled as GNOME X11** in this attempt. Prior GNOME Wayland evidence above remains unchanged.

CodeScout graph-first command `npx codescout-cli pack "Continue task 17.1 GNOME X11 native Tauri WebKitGTK quality tranche and update FINAL_QUALITY_GATE_17_1.md" --json` timed out after 120 seconds. Existing scoped graph context was retained; stale CodeScout processes (`209046`, `218156`, `218174`, `218175`) were terminated. No KRIA app, Orca, stress worker, `pidstat`, or tranche-specific GPU sampler was launched by this attempt. The existing `/usr/bin/nvidia-smi -q -x -lms 2000` process belongs to the active IDE session (`PPID 4172`) and was not treated as a QA helper.

Desktop restoration checks after cleanup:

| Exact command | Result |
|---|---|
| `gsettings get org.gnome.desktop.a11y.applications screen-reader-enabled` | `false` |
| `gsettings get org.gnome.desktop.interface enable-animations` | `true` |
| `gsettings get org.gnome.desktop.a11y.interface high-contrast` | `false` |
| `gsettings get org.gnome.desktop.interface text-scaling-factor` | `1.0` |
| `gsettings get org.gnome.desktop.interface gtk-theme` | `'Yaru-dark'` |

GNOME + X11 status remains **BLOCKED / pending**. Required precondition: log into an actual GNOME Xorg session and verify `XDG_SESSION_TYPE=x11` through both process environment and `loginctl show-session`; only then run and measure the native tranche.

## Display and scale facts

One active built-in display only: `eDP-2`, AUO panel, 1920×1200 at 165.002 Hz, scale 1.0. Advertised scales were 1.0 and 2.0. No second monitor was available. GNOME screenshot API denied access, so no screenshot or manual visual assertion is made.
## Native startup defect and fix

A GTK desktop reactivation reproduced this panic:

`Failed to setup app: a webview with label 'main' already exists`

Cause: declarative `main` window creation was replayed during GTK application reactivation. Fix: remove declarative window from `crates/kria-desktop/tauri.conf.json`; create `main` once in Rust setup with `WebviewWindowBuilder`, guarded by `get_webview_window("main").is_none()`. GTK reactivation then passed without duplicate-window panic.

## Native interaction evidence

- Real AT-SPI/WebKitGTK actions reached all seven Spaces: Converse, Memory, Automations, Capabilities, Machines, Observatory, Settings.
- Compact mode passed; Standard restoration passed; Immersive exposed Exit Immersive, Global Stop, and approvals; Exit Immersive passed.
- Approval Center showed an honest empty state and created a detached approval surface.
- KRIA Mini opened with Send/Stop fallbacks.
- Memory Graph exposed its 2D fallback and honest `No graph yet` state. Capability Constellation 2D control was invoked.
- Machines exposed honest degradation: `Remote canvas unavailable` and `Docker evals need an active fleet lease`.
- Stop with no active job returned `false`; no safety or cancellation path was bypassed.
- No real approval request or cancellable job existed, so approve/deny and active cancellation remain unverified.
- No populated native graph existed, so native 3D/lens FPS remains unmeasured.

## Tray, summon, hotkey, and detached surfaces

- GNOME AppIndicator extension was enabled and a StatusNotifier item registered.
- KRIA Mini opened; detached Approval Center was created.
- `wtype` failed honestly: `Compositor does not support the virtual keyboard protocol`.
- AT-SPI/uinput key synthesis reported success, but active focus could not be proven. Native palette content exposed Go/Do/Ask/Change; arrow/Enter/Escape behavior remains blocked.
- Always-on-top compositor acceptance was not observable.

## Accessibility evidence

- Orca 46.1 ran with speech/braille disabled. `orca --list-apps` included native `kria-desktop`; its debug log remained active through final evidence capture. Orca was then stopped and GNOME `screen-reader-enabled` restored to `false`.
- Native AT-SPI exposed landmarks, headings, tabs, controls, dialogs, status bars, search fields, Core state, modes, and seven-Space navigation.
- Final native snapshot after performance fixes: one frame, 407 nodes, still interactive.
- `enable-animations=false`: native tree remained interactive.
- High contrast enabled: three frames / 455-node tree remained interactive; restored to `false`.
- Text scale 1.25: three frames / 455-node tree remained interactive; restored to 1.0.
- Animations restored to `true`.
- Focus-ring visual inspection and complete physical keyboard flow remain unverified.
## Native performance measurements

All percentages below are `pidstat -u -r`; NVIDIA data is simultaneous `nvidia-smi dmon -s pucm`.

### Initial normal motion, three native surfaces, 60 s

| Process | Average CPU | Average RSS |
|---|---:|---:|
| `kria-desktop` | 19.58% | 337,655 KiB |
| Main WebKit | 32.33% | 172,904 KiB |
| Detached WebKit | 21.97% | 96,090 KiB |
| Mini WebKit | 21.40% | 162,034 KiB |
| Network | 0% | 29,936 KiB |
| **Family total** | **~95.28%** | **~798.6 MiB** |

NVIDIA SM and memory utilization remained 0%, power ~2 W, temperature 39–43°C. Framebuffer was 2163 MiB, mostly existing `llama-server` allocation. This failed the idle-CPU expectation.

### Reduced motion before polling fix, three surfaces, 60 s

Family CPU: **~11.63%**. Family RSS: **~701.8 MiB**. NVIDIA remained 0% and ~2 W. Primary WebKit produced ~20% CPU bursts every four seconds.

### Machines polling defect and fix

`MobileDevicesPanel` polled every 4 s and `RemoteDesktopCanvas` every 3 s. `machineStore` replaced unchanged remote/gateway objects and the 28-device array every response, invalidating the native Machines table. `sameRemoteDesktopStatus`, `sameMobileGatewayStatus`, and `sameMobileDevices` now preserve signal identity when snapshots are unchanged.

Post-fix reduced-motion live sample, three surfaces, 20 s:

- `kria-desktop`: 0.75%.
- Main WebKit: 0.85%.
- Detached WebKit: 0.05%.
- Mini WebKit: 0.10%.
- **Family total: 1.75%**, down from 11.63%; four-second burst disappeared.

### Core idle animation optimization, normal motion, one surface, 60 s

WebKitGTK repainted two filtered Core opacity/transform animations at display refresh. Idle-only timing is now quantized with `steps(20, end)` (four visual updates/s for the 5 s idle breath); active states remain smooth.

| Process | Average CPU | Average RSS |
|---|---:|---:|
| `kria-desktop` | 2.02% | 315,452 KiB |
| Network | 0% | 28,244 KiB |
| Main WebKit | 2.87% | 146,562 KiB |
| **Family total** | **4.89%** | **490,258 KiB / 478.8 MiB** |

NVIDIA SM stayed 0%; memory activity was 0–3%; power ~2 W; temperature 44–45°C; framebuffer 2307 MiB, primarily model allocation. Core remained exposed and the 407-node native tree remained interactive. Optimization is retained. Surface counts differ from the initial three-window sample, so the magnitude is not treated as a strict like-for-like benchmark. Normal-motion idle is greatly improved but still not accepted as conclusively “near zero”; Requirement 16.1 remains open at overall-gate level.
## Bounded-load responsiveness

Startup model load showed `whisper-cli` at ~542% CPU and `llama-server` at ~76% CPU. A separate controlled load used 12 CPU workers (50% of 24 logical CPUs) for exactly 10 s. Native AT-SPI `do_action` timings were:

- Converse: 20.83 ms.
- Memory: 398.36 ms.
- Machines: 36.63 ms.

All returned `True`. Memory exceeded the desired Space-switch target. These are action-call durations, not full paint-completion timings.

## Optional-service degradation and authority

Vision sidecar lacked `fastapi`; GitHub MCP lacked a token; Colab interpreter path was absent; wake-word models were absent. Runtime stayed alive and navigable, reported degraded health, and used bounded retries. Authority remained:

Intent → Capability → Policy → Substrate → Tool → Verification

No prompt-to-tool shortcut, orchestration leak, recursive autonomous loop, uncontrolled retry, substrate self-authority, or safety/confirmation/verification/cancellation bypass was introduced.

## Remaining blockers

| Matrix cell | Status |
|---|---|
| GNOME + Wayland | **PARTIAL / exercised**: native build, launch, reactivation, AT-SPI, Orca, settings toggles, tray/detach fallbacks, bounded load, CPU/RSS/GPU profiling completed; blockers above remain. |
| GNOME + X11 | **BLOCKED / pending**: no X11 session exercised. |
| KDE + Wayland | **BLOCKED / pending**: KDE session unavailable. |
| KDE + X11 | **BLOCKED / pending**: KDE/X11 session unavailable. |

Still unverified: physical fractional scaling; mixed-DPI/multi-monitor behavior; screenshots/manual visual parity; visible focus rings; native arrow/Enter/Escape palette flow; always-on-top acceptance; real approve/deny; real active-job Stop/cancellation; populated graph FPS; Intel GPU telemetry.

## Verdict

GNOME Wayland native evidence is now substantial and two native performance defects were fixed, but task 17.1 cannot pass until remaining Linux matrix cells and physical/manual blockers are exercised. Overall outcome remains **FAILED / BLOCKED**. `tasks.md` status was not changed.

## Environment-unblocking setup audit (installation blocked by sudo)

A host-only setup pass was authorized to expose GNOME Xorg, Plasma Wayland, Plasma X11, Orca, and native accessibility/profiling tools. Inspection was completed before mutation. No session was relabeled, no matrix result was added, no logout/reboot was attempted, and KRIA runtime authority was untouched.

### Pre-change host state

- Active display manager: `gdm.service` / GDM, active and running; package `gdm3` version `46.2-1ubuntu1~24.04.9`.
- Active user session remains GNOME Wayland: `loginctl show-session 2` reported `Type=wayland`, `Service=gdm-password`, `Active=yes`, `State=active`, `VTNr=2`.
- `/etc/gdm3/custom.conf` is the stock configuration. `WaylandEnable=false` remains commented; automatic/timed login remain disabled. No GDM setting was edited, so no config backup was required.
- Existing GNOME session entries:
  - X11: `/usr/share/xsessions/ubuntu-xorg.desktop`, `/usr/share/xsessions/ubuntu.desktop`.
  - Wayland: `/usr/share/wayland-sessions/ubuntu-wayland.desktop`, `/usr/share/wayland-sessions/ubuntu.desktop`.
- Existing GNOME/Xorg/accessibility packages: `ubuntu-session 46.0-1ubuntu4`, `gnome-session-bin 46.0-1ubuntu4`, `xorg 1:7.7+23ubuntu3`, `xserver-xorg 1:7.7+23ubuntu3`, `orca 46.1-1ubuntu1`, `at-spi2-core 2.52.0-1build1`, `libatk-adaptor:amd64 2.52.0-1build1`.
- Existing profiling/graphics tools: `sysstat 12.6.1-2`, `linux-tools-common 6.8.0-136.136`, running-kernel tools `linux-tools-7.0.0-28-generic 7.0.0-28.28~24.04.1`, and `mesa-utils 9.0.0-2`.
- Plasma, SDDM, Accerciser, Sysprof, and Intel GPU Tools were not installed. No KDE `.desktop` session entries existed.

### Minimal package plan

A no-change APT simulation used `--no-install-recommends` with explicit session/tool packages. Proposed explicit packages:

- `plasma-desktop` candidate `4:5.27.12-0ubuntu0.1` — minimal Plasma desktop/session integration.
- `plasma-workspace-wayland` candidate `4:5.27.12-0ubuntu0.1` — Plasma Wayland session plus `kwin-wayland 4:5.27.11-0ubuntu3`.
- `kwin-x11` candidate `4:5.27.11-0ubuntu3` — explicitly required because it is only recommended, not installed by `--no-install-recommends`; enables a functional Plasma X11 compositor/session.
- `accerciser` candidate `3.42.0-1ubuntu0.1` — native AT-SPI inspection.
- `sysprof` candidate `46.0-1build1` — native GNOME/system profiling.
- `intel-gpu-tools` candidate `1.28-1ubuntu2` — Intel GPU telemetry missing from the prior tranche.

`orca`, `at-spi2-core`, `sysstat`, exact running-kernel Linux tools, and `mesa-utils` are already installed and do not need reinstalling. `linux-tools-generic` was deliberately omitted: it targets Ubuntu's 6.8 generic kernel while this host runs `7.0.0-28-generic`, whose exact tools package is already installed. The simulation proposed no `sddm` installation, no GDM removal, no desktop-package removal, and no NVIDIA changes. Full Kubuntu metapackages were deliberately avoided.

### Authentication blocker and exact user action

No package mutation occurred. Noninteractive privilege check failed exactly:

```text
sudo: a password is required
```

Authenticate locally, then run the following commands in a terminal:

```bash
printf '%s\n' 'gdm3 shared/default-x-display-manager select gdm3' | sudo debconf-set-selections
sudo env DEBIAN_FRONTEND=noninteractive apt-get install --no-install-recommends plasma-desktop plasma-workspace-wayland kwin-x11 accerciser sysprof intel-gpu-tools
```

The preseed is defensive; the simulated dependency set did not include SDDM. After the install returns successfully, resume this task so package versions, GDM ownership, and all four session entries can be verified before logout.

### Expected activation and rollback

A logout/login is required to select another desktop/session from GDM. A reboot is not expected for these packages, but may be used later if GDM does not refresh session discovery; this agent will not trigger logout or reboot.

Planned post-install verification:

```bash
dpkg-query -W gdm3 plasma-desktop plasma-workspace plasma-workspace-wayland kwin-x11 kwin-wayland orca accerciser sysprof intel-gpu-tools
systemctl status display-manager --no-pager --full
```

Verify GNOME Xorg and Plasma X11 entries under `/usr/share/xsessions/`, Plasma Wayland under `/usr/share/wayland-sessions/`, and confirm `/etc/X11/default-display-manager` still resolves to `/usr/sbin/gdm3`. Only a real login whose environment and `loginctl` both report the selected DE/session type can fill a matrix cell.

If rollback is later needed, first return to a GNOME session, then remove only the explicit additions:

```bash
sudo apt-get remove plasma-desktop plasma-workspace-wayland kwin-x11 accerciser sysprof intel-gpu-tools
sudo dpkg-reconfigure gdm3
```

Do not run unattended `autoremove`; review `apt-get --simulate autoremove` first to avoid removing shared desktop libraries. `/etc/gdm3/custom.conf` needs no restoration because this pass did not edit it. Current setup outcome: **BLOCKED by sudo authentication; host unchanged**.
## Continuation audit — 2026-07-17 20:46–20:50 IST

Outcome remains **FAILED / BLOCKED**. This continuation re-checked host state before testing, reran every executable automated gate, and exercised only the real GNOME Wayland native cell. It did not use `sudo`, bypass authentication, log out, reboot, or relabel an XWayland compatibility endpoint as an X11 session.

### Graph/spec preflight

- `npx codescout-cli pack "Verify task 17.1 final quality gate current host session packages and executable Linux native E2E a11y performance gates" --json` timed out after 120 seconds. Its stale process, PID `225548`, was terminated.
- Root `.codescout/graph.json` was absent; the required scoped fallback `ui/.codescout/graph.json` exists and was used with direct reads of the linked Playwright configuration and three gate suites.
- The spec directory contains `requirements.md`, `design.md`, and `tasks.md`; no `.config` file exists. This task was therefore not treated as a bugfix exploration task.

### Current host/session/package truth

Environment and login manager independently prove the only active matrix cell is **GNOME + Wayland**:

```text
XDG_CURRENT_DESKTOP=ubuntu:GNOME
XDG_SESSION_TYPE=wayland
GDK_BACKEND=wayland
WAYLAND_DISPLAY=wayland-0
DISPLAY=:0
```

```text
loginctl show-session 2:
Id=2
Name=obaid
VTNr=2
Seat=seat0
TTY=tty2
Remote=no
Service=gdm-password
Type=wayland
Active=yes
State=active
```

GDM remains authoritative: `gdm.service` is active/running; `gdm3` is `46.2-1ubuntu1~24.04.9` with status `ii`; `/etc/X11/default-display-manager` contains `/usr/sbin/gdm3`.

The authorized package command was **not** run. Exact re-check:

- Installed: `orca 46.1-1ubuntu1` (`ii`).
- Not installed: `plasma-desktop`, `plasma-workspace`, `plasma-workspace-wayland`, `kwin-x11`, `kwin-wayland`, `accerciser`, `sysprof` (`un`), and `intel-gpu-tools`.
- Available executable among the requested extra tools: `/usr/bin/orca` only. `accerciser`, `sysprof`, and `intel_gpu_top` are absent.
- X11 descriptors remain GNOME-only: `ubuntu-xorg.desktop`, `ubuntu.desktop`.
- Wayland descriptors remain GNOME-only: `ubuntu-wayland.desktop`, `ubuntu.desktop`.
- No Plasma descriptor exists, so KDE Wayland/X11 cannot be selected or tested.

### Fresh automated gate evidence

`npm run quality:final` in `ui/` completed with exit code 0. Because the script uses an `&&` chain, type checking, all three consistency lints, unit/component tests, production build, and full Playwright E2E all passed:

| Gate | Fresh result |
|---|---|
| TypeScript check | **PASS** |
| Token/component/expansion consistency lints | **PASS** |
| Vitest | **PASS — 154 files / 1,280 tests** |
| Production build | **PASS — 396 modules / 4.03 s** |
| Full Playwright | **PASS — 46/46** across WebKit and Chromium |
| Flow-map E2E | **PASS** in both engines |
| Axe WCAG A/AA + keyboard dialog gate | **PASS** for all seven Spaces in both engines |
| Browser performance/degradation gates | **PASS** in both engines |

Non-failing warnings remain: Solid test disposal/multiple-instance warnings, missing TanStack `data-index` diagnostics in one virtualization test, malformed CSS comment warning during minification, and a `687.03 kB` vendor chunk warning. They did not fail the configured gates but remain cleanup debt.

`cargo check -p kria-desktop` also passed in 1.88 s. Existing dead-code warnings remain for `InjectionScore.confidence`, `InjectionScore.accepted`, and `openclaw_skills_db_path`.

### Fresh native GNOME Wayland evidence

`cargo run -p kria-desktop` launched the real Tauri/WebKitGTK desktop process in the environment/loginctl-proven GNOME Wayland session. Orca's live application list included:

```text
pid: 237868   kria-desktop   target/debug/kria-desktop
```

Native startup preserved safety and degradation behavior:

- Global safety halt engaged while vision/uinput services were warming.
- Runtime explicitly detected the host as Wayland for automation restrictions and warned that xdotool modifier release is unavailable.
- The environment grounder reported `XWayland` because this code's capability taxonomy intentionally means “Wayland session with a `DISPLAY` compatibility endpoint”; it is **not** used as matrix evidence. The matrix remains GNOME Wayland by environment plus `loginctl`.
- Optional services failed honestly and did not crash the UI: vision lacked `fastapi`; Colab referenced a missing Python interpreter; GitHub MCP lacked `GITHUB_PERSONAL_ACCESS_TOKEN`; wake-word model files were absent.
- No prompt-to-tool shortcut, policy bypass, confirmation bypass, verification bypass, cancellation bypass, uncontrolled retry, recursive loop, or substrate self-authority was observed.

The app enabled GTK toolkit accessibility for the native process. After the app was stopped, the prior desktop state was restored and verified:

```text
toolkit-accessibility=false
screen-reader-enabled=false
enable-animations=true
high-contrast=false
text-scaling-factor=1.0
```

No `kria-desktop`, `kria-uinput-daemon`, `llama-server`, or `kria-vision` child remained after cleanup.

### Fresh native performance sample — informative, not an idle pass

A 20-second sample was captured during active model/service startup, not settled idle:

- `kria-desktop`: average CPU **3.10%**, average RSS **1,160,026 KiB**.
- One startup second reached 48% CPU while model/service memory pages were loading.
- WebKit point sample: **0.5% CPU**, **118,124 KiB RSS**.
- The `pidstat -C` expression did not include the truncated `WebKitWebProces` command name, so no full native-family average is claimed from this run.
- NVIDIA sample: SM **0%** throughout; memory engine **0–3%**; power **2–22 W** while model allocations changed. This is not an idle-only UI sample.

Therefore this continuation does **not** upgrade Requirement 16.1. Prior settled native evidence remains the stronger sample; “idle near zero” is still not conclusively accepted for the overall matrix. Intel telemetry remains unavailable because `intel-gpu-tools` is absent.

### Evidence intentionally not claimed

No new claim is made for a physical second monitor, mixed DPI, fractional scaling, screenshots, visual focus rings, long-session stability, always-on-top compositor acceptance, a real approval decision, a real cancellable job, populated graph FPS, or Intel GPU telemetry. The fresh native launch did not create suitable real approval/job/graph data, so those gates remain open rather than mocked or inferred.

### Matrix status after continuation

| Matrix cell | Status |
|---|---|
| GNOME + Wayland | **PARTIAL / exercised again** — build, native launch, Orca discovery, degradation, cleanup, and startup performance sample completed; physical/manual blockers remain. |
| GNOME + X11 | **BLOCKED** — session descriptor exists, but no real Xorg login is active. |
| KDE + Wayland | **BLOCKED** — Plasma packages/session descriptor absent. |
| KDE + X11 | **BLOCKED** — Plasma/KWin packages/session descriptor absent. |

### Exact user action still required

Authenticate locally and run:

```bash
printf '%s\n' 'gdm3 shared/default-x-display-manager select gdm3' | sudo debconf-set-selections
sudo env DEBIAN_FRONTEND=noninteractive apt-get install --no-install-recommends plasma-desktop plasma-workspace-wayland kwin-x11 accerciser sysprof intel-gpu-tools
```

Then verify installation and GDM ownership. Manually log out and use GDM to enter each remaining session. For every matrix cell, resume testing only after both process environment and `loginctl show-session <id>` agree on the real DE/session type. No agent-triggered logout or reboot is authorized. Physical/manual evidence additionally requires connecting a second mixed-DPI monitor and providing real approval, cancellable-job, and populated-graph scenarios.

### Authority consistency and final verdict

KRIA remains authoritative orchestrator. OpenClaw, MCP, n8n, and providers remain substrates. Observed flow remains:

`Intent → Capability → Policy → Substrate → Tool → Verification`

No runtime authority invariant was changed by this audit. Final verdict remains **FAILURE / BLOCKED**, not success. Automated gates are green, but required Linux matrix and physical/manual Definition-of-Done evidence are incomplete. `tasks.md` was not changed.

## Exact staged AppShell cutover gate — 2026-07-18

This gate validates the exact redesign index in the isolated worktree at `/media/obaid/SSD/kria-skills/.staged-validation`; unrelated owner worktree changes were excluded.

| Gate | Result |
|---|---|
| `npm run check` | **PASS** |
| UI consistency lints | **PASS** |
| Full Vitest suite | **PASS — 148 files / 1,247 tests** |
| Final focused AppShell + Converse Vitest | **PASS — 2 files / 21 tests** |
| `npm run build` | **PASS — 420 modules**; existing vendor chunk warning only |
| `cargo check -p kria-desktop` | **PASS** |
| Core workflow-continuation tests | **PASS — 24/24** |
| Phase-8 focused integration tests | **PASS — 3/3** |
| Desktop approval tests | **PASS — 6/6** |
| Playwright WebKit + Chromium | **PASS — 46/46** |

The browser gate initially exposed three real integration defects after first-run provisioning became backend-authoritative: test fixtures did not declare completed provisioning, deterministic Memory seeds raced the initial backend refresh, and floating Converse rail controls obscured the export target. Fixtures now provide explicit backend state, the dev-only harness becomes ready after Memory initialization, and all conversation actions share normal toolbar flow. Workflow authoring E2E now follows the authoritative save → backend-test lifecycle instead of expecting a client-only dry run.

Browser fixtures remain regression evidence, not native Tauri IPC evidence. Advanced `memory_*` AppShell behavior still depends on the separate Memory backend upgrade; that backend is intentionally excluded from this cutover commit. GNOME X11, KDE Wayland/X11, mixed-DPI, and remaining physical/manual cells above remain blocked and are not represented as passed.

## Committed advanced Memory backend — 2026-07-18

Advanced `memory_*` AppShell behavior is now backed by committed runtime code in commit `f7cc8da3c69b8cd8c9608129c5eebe217dda6cb7` (`Land advanced Memory architecture`). This supersedes the earlier cutover note that described the backend as excluded.

Validation used an isolated materialization of the exact staged index at `/media/obaid/SSD/kria-skills/.memory-staged-validation`; unrelated owner worktree changes were absent. `git diff --cached --check` passed before commit.

| Exact staged gate | Result |
|---|---|
| `cargo check -p kria-core -p kria-server` | **PASS** |
| `cargo check -p kria-desktop` | **PASS** |
| `cargo test -p kria-core --lib memory::` | **PASS — 212/212** |
| `cargo test -p kria-core --test memory_invariants` | **PASS — 3/3** |
| `cargo test -p kria-core --test memory_recovery` | **PASS — 2/2** |
| `cargo test -p kria-core --test memory_scale` | **PASS — 1/1**, exercising **500 memories** |
| `cargo test -p kria-server --test integration_api` | **PASS — 13/13**, including unavailable-state gating and live `MemorySystem` HTTP routes |

Warnings were non-failing existing dead/unused-code warnings, including `InjectionScore` fields and test-only unused imports. No 500K-memory claim is made; scale evidence covers 500 memories only. Browser mocks remain regression evidence and do not prove native Rust/Tauri IPC. GNOME X11, KDE Wayland/X11, mixed-DPI, physical/manual scenarios, and native performance acceptance remain blocked exactly as documented above; this Memory commit does not change the overall task 17.1 verdict.

