# Linux Graphics Guidance & Safe-Mode Boot

KRIA is a Tauri app; on Linux it renders through **WebKitGTK** (the system
webview), not Chromium. WebKitGTK's accelerated paths behave differently across
GPU/driver/desktop combinations. On some setups — most commonly **NVIDIA under
Wayland** — the window can come up **blank/white** or the app can crash on
launch, even though the backend started fine.

This page explains what KRIA does automatically, and how to launch **safe mode**
if you still hit a blank screen or crash.

> Scope: this is a rendering/boot-resilience concern only. It changes how the
> webview composites; it does **not** change KRIA's orchestration, tools, or
> memory behavior.

## What KRIA does automatically

At startup (before the webview initializes) KRIA establishes a Linux rendering
baseline:

- **DMABUF renderer disabled by default.** KRIA sets
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` unless you have already set it yourself.
  This is the single most effective fix for the NVIDIA/Wayland blank-window
  issue and keeps accelerated compositing otherwise intact.
- **Problematic-environment detection.** If KRIA detects **Wayland + NVIDIA**,
  it logs a hint to stderr pointing you here.
- **Graceful recovery.** If the *first* boot fails to build the webview, KRIA
  automatically **relaunches itself in safe mode** (see below) instead of dying
  as a blank window. If the UI throws while rendering, a recoverable in-app
  boot-error screen (Retry / Reload) replaces any white screen.

KRIA never overrides an environment flag you set explicitly — your value always
wins.

## Safe mode

Safe mode disables the WebKitGTK paths most likely to cause blank/crash at the
cost of GPU acceleration (rendering falls back to a reliable software/GL path).
Use it if the normal launch shows a blank window or crashes.

Launch safe mode either way:

```bash
# CLI flag
kria-desktop --safe-mode

# or environment variable
KRIA_SAFE_MODE=1 kria-desktop
```

In safe mode KRIA sets (unless you've set them yourself):

| Flag | Effect |
|---|---|
| `WEBKIT_DISABLE_DMABUF_RENDERER=1` | Use a reliable render path (fixes most blank windows) |
| `WEBKIT_DISABLE_COMPOSITING_MODE=1` | Disable accelerated compositing (heaviest fallback; most compatible) |

## Manual environment flags

If you want to tune the behavior yourself, set any of these **before** launching
KRIA. Because KRIA only sets flags that are unset, your explicit values take
precedence.

| Variable | When to use |
|---|---|
| `WEBKIT_DISABLE_DMABUF_RENDERER=1` | Blank white window on NVIDIA/Wayland (KRIA sets this by default) |
| `WEBKIT_DISABLE_COMPOSITING_MODE=1` | Still blank/crashing after the above; disables accel compositing |
| `WEBKIT_DISABLE_DMABUF_RENDERER=0` | Force-enable DMABUF (opt back in on a GPU/driver where it works) |
| `__NV_PRIME_RENDER_OFFLOAD=1` + `__GLX_VENDOR_LIBRARY_NAME=nvidia` | Hybrid-graphics laptops: run KRIA on the NVIDIA GPU via PRIME offload |
| `WEBKIT_FORCE_COMPOSITING_MODE=1` | Rarely needed; force compositing on if your setup benefits |

Example — force the most compatible path manually:

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 kria-desktop
```

## Rendering posture (why 2D is the default)

Because WebKitGTK has **no fast WebGL compositing path** on Linux, KRIA is
**2D-first**: the Memory graph and the Capability constellation render in 2D by
default. The 3D representation is an *opt-in enhancement* that turns on only when
the device passes both runtime capability detection **and** an on-device
performance probe. Reduced-motion always forces the static 2D representation.

Aura-glass **blur** is likewise treated as an enhancement: floating surfaces use
`backdrop-filter` blur only when the device supports it and reduced-motion is
off; otherwise they degrade to a solid translucent surface. The visual language
is designed to survive without blur.

## Troubleshooting checklist

1. Blank/white window on launch → run `kria-desktop --safe-mode`.
2. Still blank → set `WEBKIT_DISABLE_COMPOSITING_MODE=1` explicitly and relaunch.
3. Crash on launch under Wayland → try launching under X11, or use safe mode.
4. UI renders but is sluggish → this is expected to stay 2D on WebKitGTK; 3D
   lenses remain off on hardware that doesn't pass the on-device probe.
5. Capture what happened → run from a terminal to see the `[KRIA]` stderr hints.
