# Linux Desktop Validation — Task 12.5

Requirements: 18.1, 18.2, 18.4, 18.5, 8.3. Date: 2026-07-03.

## Evidence posture

This repository was validated on one developer environment only. `XDG_CURRENT_DESKTOP=ubuntu:GNOME` and `XDG_SESSION_TYPE=wayland` were observed. No KDE, X11, second monitor, alternate-DPI monitor, tray host, compositor pinning, or GPU behavior was physically exercised. Automated checks prove fallback contracts and scaling math; they do not replace WebKitGTK/compositor/manual validation.

| Desktop | Session | Environment observed | Automated contracts | WebKitGTK/manual result |
|---|---|---:|---:|---|
| GNOME | Wayland | yes | covered | **PENDING-HW** |
| GNOME | X11 | no | covered | **PENDING-HW** |
| KDE | Wayland | no | covered | **PENDING-HW** |
| KDE | X11 | no | covered | **PENDING-HW** |

## Automated evidence

- Tray creation failure is non-fatal (`crates/kria-desktop/src/main.rs`); in-app Core, Approval Center, palette, and Mini remain authoritative fallbacks.
- Global shortcut registration is try/degrade (`summon.rs`); `summon.test.ts` proves optional focus failure still opens in-app palette and Ctrl/Cmd+K works.
- Always-on-top remains best-effort (`windows.rs`); companion stays decorated/focusable when compositor ignores pinning. UI makes no pin-success claim.
- Navigation remains in AppShell/Dock/palette; no global-menu dependency.
- `fonts.css` self-hosts Space Grotesk, IBM Plex Sans, and JetBrains Mono. Legacy root aliases now resolve only to generated font tokens, not GTK/Qt/system UI families.
- `linuxDesktopValidation.test.ts` checks explicit matrix coverage, font ownership, dynamic viewport sizing, and fractional DPR backing-store invariants.
- G2 canvas uses `canvasBackingStoreSize`; CSS size stays stable while backing pixels follow DPR 1/1.25/1.5/1.75/2. Invalid DPR fails safe and excessive allocation is capped.
- `RemoteDesktopCanvas.test.tsx` proves unknown, pending-consent, active, and inconsistent capture states are explicit; unavailable controls are not rendered.

## Manual runbook per matrix cell

1. Launch packaged Tauri/WebKitGTK build. Record DE/session, GPU, WebKitGTK version, display scale, monitor arrangement.
2. Disable tray host or extension: app must launch; summon via in-app palette/Mini; approval/Core remain visible.
3. deny/conflict global shortcut: Ctrl/Cmd+K and palette button still work. Verify no focus seizure while typing.
4. Open Mini/Now; if pinning is rejected, verify normal decorated window remains focusable and no UI claims it is pinned.
5. Compare dark/light screenshots at 100/125/150/175/200%; move window between mixed-DPI monitors. Check text, 1px borders, SVG icons, canvas, hit targets, and mode transitions for stable proportions/crispness.
6. Exercise portal capture: denied, pending, granted, revoked/backend-stopped. Labels must match runtime evidence; controls must never silently no-op.

Record each row PASS/FALLBACK only after those steps. Until then status stays **PENDING-HW**.

## Architecture self-check

Changes are presentation/capability signaling only. No tool execution, runtime/substrate authority, recursive event loop, approval/cancellation/verification bypass, or backend command/event contract was added. Runtime status remains source of truth for capture/input and approvals.
