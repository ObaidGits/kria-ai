import { Component, createSignal, onCleanup, onMount, Show } from "solid-js";
import { mobileStore } from "./mobileStore";
import {
  confirmSession,
  presetToOpt,
  type QualityPreset,
  remoteStatus,
  requestSession,
  stopSession,
} from "./remoteDesktopApi";
import { attachRdpInput, SCANCODE, type RdpInputHandle, type TouchMode } from "./rdpInput";
import { createRdSession, type RdSession } from "./rdSession";
import { stateAction, stateLabel, type RdState } from "./rdState";
import {
  applyPan,
  applyPinch,
  clampTransform,
  doubleTapToggle,
  fitTransform,
  type Bounds,
  type ViewTransform,
} from "./viewTransform";
import { extractHealth, formatHealth, reportToArray, type HealthSample } from "./rdStats";
import RdToolbar from "./components/RdToolbar";
import RdKeyboardBar from "./components/RdKeyboardBar";

/**
 * In-app remote desktop (Phase 4.6 v3 + UX polish) — capture via
 * xdg-desktop-portal ScreenCast + PipeWire, streamed over WebRTC (server is the
 * offerer / browser answers). This view orchestrates the HITL request→confirm
 * control plane, the {@link RdSession} transport + reconnect, the zoom/pan view
 * transform, touch-mode input, the toolbar/keyboard, and a stats overlay.
 */

/** Pre-connection (control-plane) phase, distinct from the live session state. */
type ViewPhase = "idle" | "requesting" | "awaiting_approval" | "live";

const QUALITY_ORDER: QualityPreset[] = ["auto", "high", "balanced", "low"];

const RemoteDesktopView: Component = () => {
  const [viewPhase, setViewPhase] = createSignal<ViewPhase>("idle");
  const [sessionState, setSessionState] = createSignal<RdState>({ tag: "idle" });
  const [error, setError] = createSignal("");
  const [description, setDescription] = createSignal("");
  const [pendingId, setPendingId] = createSignal("");
  const [resumeId, setResumeId] = createSignal("");

  const [transform, setTransform] = createSignal<ViewTransform>({ scale: 1, tx: 0, ty: 0 });
  const [bounds, setBounds] = createSignal<Bounds>({ vw: 1, vh: 1, sw: 1, sh: 1 });
  const [touchMode, setTouchMode] = createSignal<TouchMode>("direct");
  const [quality, setQuality] = createSignal<QualityPreset>("auto");
  const [keyboardOpen, setKeyboardOpen] = createSignal(false);
  const [toolbarCollapsed, setToolbarCollapsed] = createSignal(false);
  const [showStats, setShowStats] = createSignal(false);
  const [statsLine, setStatsLine] = createSignal("");
  const [inputHandle, setInputHandle] = createSignal<RdpInputHandle | null>(null);

  let container: HTMLDivElement | undefined;
  let video: HTMLVideoElement | undefined;
  let hiddenInput: HTMLInputElement | undefined;
  let session: RdSession | null = null;
  let input: RdpInputHandle | null = null;
  let resizeObs: ResizeObserver | null = null;
  let statsTimer: ReturnType<typeof setInterval> | null = null;
  let statsSample: HealthSample | undefined;

  const server = () => mobileStore.serverUrl();
  const token = () => mobileStore.token();

  // ── View transform helpers ──────────────────────────────────────────────
  const applyTransformToVideo = () => {
    const t = transform();
    const b = bounds();
    if (video) {
      video.style.width = `${b.sw}px`;
      video.style.height = `${b.sh}px`;
      video.style.transformOrigin = "0 0";
      video.style.transform = `translate(${t.tx}px, ${t.ty}px) scale(${t.scale})`;
    }
    input?.setViewTransform(t, b);
  };

  const recomputeBounds = (refit: boolean) => {
    if (!container || !video) return;
    const vw = container.clientWidth || 1;
    const vh = container.clientHeight || 1;
    const sw = video.videoWidth || vw;
    const sh = video.videoHeight || vh;
    const b: Bounds = { vw, vh, sw, sh };
    setBounds(b);
    setTransform((t) => (refit ? fitTransform(b) : clampTransform(t, b)));
    applyTransformToVideo();
  };

  const fitReset = () => {
    setTransform(fitTransform(bounds()));
    applyTransformToVideo();
  };

  // ── Session wiring ────────────────────────────────────────────────────────
  const makeSession = (): RdSession => {
    const s = createRdSession({ server: server(), token: token() });
    s.onState((st) => {
      setSessionState(st);
      if (st.tag === "connected") onConnected();
      if (st.tag === "idle") onSessionIdle();
    });
    s.onTrack((stream) => {
      if (video) {
        video.srcObject = stream;
        video.play().catch(() => {});
      }
    });
    return s;
  };

  const onConnected = () => {
    if (!container || input) return;
    input = attachRdpInput(container, (e) => session?.sendInput(e), {
      onPinch: (fx, fy, delta) => {
        const r = container!.getBoundingClientRect();
        setTransform((t) => applyPinch(t, fx - r.left, fy - r.top, delta, bounds()));
        applyTransformToVideo();
      },
      onPan: (dx, dy) => {
        setTransform((t) => applyPan(t, dx, dy, bounds()));
        applyTransformToVideo();
      },
      onDoubleTap: (x, y) => {
        const r = container!.getBoundingClientRect();
        setTransform((t) => doubleTapToggle(t, x - r.left, y - r.top, bounds()));
        applyTransformToVideo();
      },
    });
    input.setMode(touchMode());
    setInputHandle(input);
    // Bounds may already be known (metadata) — recompute + fit.
    recomputeBounds(true);
    startStats();
  };

  const onSessionIdle = () => {
    teardownInput();
    setViewPhase("idle");
  };

  const teardownInput = () => {
    stopStats();
    try {
      input?.destroy();
    } catch {
      /* ignore */
    }
    input = null;
    setInputHandle(null);
    if (video) video.srcObject = null;
  };

  // ── Stats overlay ──────────────────────────────────────────────────────────
  const startStats = () => {
    stopStats();
    statsSample = undefined;
    statsTimer = setInterval(async () => {
      const report = await session?.getStats();
      if (!report) return;
      const { snapshot, sample } = extractHealth(reportToArray(report), statsSample);
      statsSample = sample;
      setStatsLine(formatHealth(snapshot));
    }, 1000);
  };
  const stopStats = () => {
    if (statsTimer) clearInterval(statsTimer);
    statsTimer = null;
    setStatsLine("");
  };

  // ── Control plane (HITL) ────────────────────────────────────────────────────
  const start = async () => {
    setError("");
    setViewPhase("requesting");
    try {
      const req = await requestSession(server(), token());
      setDescription(req.description);
      setPendingId(req.session_id);
      setViewPhase("awaiting_approval");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setViewPhase("idle");
    }
  };

  const confirm = async () => {
    setViewPhase("live");
    try {
      const act = await confirmSession(server(), token(), pendingId());
      session = makeSession();
      session.setQuality(presetToOpt(quality()));
      session.start(act.session_id, presetToOpt(quality()));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setViewPhase("idle");
    }
  };

  const resume = (id: string) => {
    setResumeId("");
    setViewPhase("live");
    session = makeSession();
    session.setQuality(presetToOpt(quality()));
    session.start(id, presetToOpt(quality()));
  };

  const stop = async () => {
    session?.stop();
    session = null;
    teardownInput();
    setViewPhase("idle");
    try {
      await stopSession(server(), token());
    } catch {
      /* ignore */
    }
  };

  const reconnect = () => session?.manualReconnect();

  const retry = () => {
    setError("");
    setViewPhase("idle");
  };

  // ── Toolbar actions ──────────────────────────────────────────────────────
  const toggleKeyboard = () => {
    const open = !keyboardOpen();
    setKeyboardOpen(open);
    if (open) setTimeout(() => hiddenInput?.focus(), 0);
    else hiddenInput?.blur();
  };
  const toggleTouchMode = () => {
    const next = touchMode() === "direct" ? "trackpad" : "direct";
    setTouchMode(next);
    input?.setMode(next);
  };
  const cycleQuality = () => {
    const idx = QUALITY_ORDER.indexOf(quality());
    const next = QUALITY_ORDER[(idx + 1) % QUALITY_ORDER.length];
    setQuality(next);
    if (session) {
      session.setQuality(presetToOpt(next));
      if (sessionState().tag === "connected") session.manualReconnect();
    }
  };
  const toggleFullscreen = () => {
    const el = container;
    if (!el) return;
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => {});
    else void el.requestFullscreen?.().catch(() => {});
  };

  // ── Soft keyboard (hidden input) ────────────────────────────────────────────
  const onHiddenInput = (e: InputEvent) => {
    if (e.data && input) for (const ch of e.data) input.typeChar(ch);
    if (hiddenInput) hiddenInput.value = "";
  };
  const onHiddenKeydown = (e: KeyboardEvent) => {
    if (!input) return;
    if (e.key === "Enter") {
      e.preventDefault();
      input.tapKey(SCANCODE.Enter);
    } else if (e.key === "Backspace") {
      e.preventDefault();
      input.tapKey(SCANCODE.Backspace);
    }
  };

  // ── Orientation / resize ────────────────────────────────────────────────────
  const onOrientation = () => {
    const landscape = window.innerWidth > window.innerHeight;
    setToolbarCollapsed(landscape);
    recomputeBounds(true);
  };

  onMount(async () => {
    if (container && "ResizeObserver" in window) {
      resizeObs = new ResizeObserver(() => recomputeBounds(false));
      resizeObs.observe(container);
    }
    window.addEventListener("orientationchange", onOrientation);
    window.visualViewport?.addEventListener("resize", () => recomputeBounds(false));
    // Resume an already-active server session after a refresh.
    try {
      const st = await remoteStatus(server(), token());
      if (st.state === "active" && st.session_id) setResumeId(st.session_id);
    } catch {
      /* ignore */
    }
  });

  onCleanup(() => {
    window.removeEventListener("orientationchange", onOrientation);
    resizeObs?.disconnect();
    stopStats();
    const wasLive = viewPhase() === "live";
    teardownInput();
    session?.stop();
    if (wasLive) void stopSession(server(), token()).catch(() => {});
  });

  // ── Derived display ──────────────────────────────────────────────────────
  const isConnected = () => viewPhase() === "live" && sessionState().tag === "connected";
  const liveStatusLabel = () => stateLabel(sessionState());
  const liveAction = () => stateAction(sessionState());

  return (
    <div class="mobile-desktop">
      <Show when={isConnected()}>
        <div class="mobile-desktop-banner">🔴 Remote desktop ACTIVE</div>
        <RdToolbar
          collapsed={toolbarCollapsed}
          onExpand={() => setToolbarCollapsed(false)}
          onToggleKeyboard={toggleKeyboard}
          onFitReset={fitReset}
          onDisconnect={stop}
          onFullscreen={toggleFullscreen}
          onReconnect={reconnect}
          touchMode={touchMode}
          onToggleTouchMode={toggleTouchMode}
          quality={quality}
          onCycleQuality={cycleQuality}
          showStats={showStats}
          onToggleStats={() => setShowStats(!showStats())}
        />
        <Show when={showStats() && statsLine()}>
          <div class="mobile-desktop-stats">{statsLine()}</div>
        </Show>
      </Show>

      <Show when={viewPhase() === "idle"}>
        <div class="mobile-desktop-start">
          <p class="mobile-hint">
            View &amp; control this machine's live screen (same session, X11 or Wayland), right
            here in the app. Starting is a high-risk action and is logged.
          </p>
          <Show when={error()}>
            <div class="mobile-error">{error()}</div>
          </Show>
          <Show when={resumeId()}>
            <div class="mobile-desktop-resume">
              <p>A remote desktop session is already active on this machine.</p>
              <div class="mobile-approval-actions">
                <button onClick={() => resume(resumeId())}>Resume</button>
                <button class="danger" onClick={stop}>
                  Stop it
                </button>
              </div>
            </div>
          </Show>
          <button onClick={start}>Start remote desktop</button>
        </div>
      </Show>

      <Show when={viewPhase() === "requesting"}>
        <div class="mobile-desktop-start">
          <p>Requesting session…</p>
        </div>
      </Show>

      <Show when={viewPhase() === "awaiting_approval"}>
        <div class="mobile-desktop-confirm">
          <p>{description()}</p>
          <div class="mobile-approval-actions">
            <button onClick={confirm}>Confirm &amp; connect</button>
            <button class="danger" onClick={() => setViewPhase("idle")}>
              Cancel
            </button>
          </div>
        </div>
      </Show>

      {/* Live: connecting/negotiating/establishing/reconnecting/disconnected/error overlay */}
      <Show when={viewPhase() === "live" && !isConnected()}>
        <div class="mobile-desktop-start">
          <p>{liveStatusLabel()}</p>
          <div class="mobile-approval-actions">
            <Show when={liveAction() === "reconnect"}>
              <button onClick={reconnect}>Reconnect</button>
            </Show>
            <Show when={liveAction() === "retry"}>
              <button onClick={retry}>Try again</button>
            </Show>
            <button class="danger" onClick={stop}>
              Cancel
            </button>
          </div>
        </div>
      </Show>

      <div
        ref={container}
        class="mobile-desktop-screen"
        style={{ display: isConnected() ? "block" : "none" }}
      >
        <video ref={video} class="mobile-desktop-canvas" autoplay playsinline muted tabindex="0" onLoadedMetadata={() => recomputeBounds(true)} />
      </div>

      <Show when={isConnected() && keyboardOpen()}>
        <RdKeyboardBar input={inputHandle} />
      </Show>

      <input
        ref={hiddenInput}
        class="mobile-desktop-hidden-input"
        style={{ display: keyboardOpen() ? "block" : "none" }}
        autocomplete="off"
        autocapitalize="off"
        autocorrect="off"
        spellcheck={false}
        onInput={(e) => onHiddenInput(e as unknown as InputEvent)}
        onKeyDown={onHiddenKeydown}
      />
    </div>
  );
};

export default RemoteDesktopView;
