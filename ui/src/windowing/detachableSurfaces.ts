import { createSignal } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { bridgeInvokeOptional, coerceApprovalEnvelope } from "../bridge";
import { approvalStore } from "../stores/approvalStore";
import { isTauriAvailable } from "../bridge/types";

export const DETACHABLE_SURFACES = [
  "thread",
  "approval-center",
  "lens",
  "remote-desktop",
  "observatory-now",
] as const;

export const COMPANION_SURFACES = ["kria-mini", "now-mini"] as const;

export type DetachableSurface = (typeof DETACHABLE_SURFACES)[number];
export type CompanionSurface = (typeof COMPANION_SURFACES)[number];
export type WindowSurface = DetachableSurface | CompanionSurface;

const SURFACE_CONTEXT_EVENT = "kria://surface-context";
const APPROVAL_RESOLVED_EVENT = "approval://presentation-resolved";

interface SurfaceContextPayload {
  surface?: string;
  context?: string | null;
}

interface ApprovalResolvedPayload {
  id?: string;
  status?: string;
}

export function isDetachableSurface(value: string | null): value is DetachableSurface {
  return value !== null && (DETACHABLE_SURFACES as readonly string[]).includes(value);
}

export function isCompanionSurface(value: string | null): value is CompanionSurface {
  return value !== null && (COMPANION_SURFACES as readonly string[]).includes(value);
}

export function isWindowSurface(value: string | null): value is WindowSurface {
  return isDetachableSurface(value) || isCompanionSurface(value);
}

export function surfaceFromLocation(search: string): {
  surface: WindowSurface | null;
  context: string | null;
} {
  const params = new URLSearchParams(search);
  const surface = params.get("surface");
  return {
    surface: isWindowSurface(surface) ? surface : null,
    context: params.get("context"),
  };
}

const initial = typeof window === "undefined"
  ? { surface: null, context: null }
  : surfaceFromLocation(window.location.search);
const [surface, setSurface] = createSignal<WindowSurface | null>(initial.surface);
const [context, setContext] = createSignal<string | null>(initial.context);
const [active, setActive] = createSignal(initial.surface === null);
const [inlineCompanionSurface, setInlineCompanionSurface] = createSignal<CompanionSurface | null>(null);
let unlisteners: UnlistenFn[] = [];
let initialized = false;

export async function openDetachedSurface(
  nextSurface: DetachableSurface,
  nextContext?: string | null,
): Promise<boolean> {
  const label = await bridgeInvokeOptional<string>("open_detached_surface", {
    surface: nextSurface,
    context: nextContext ?? null,
  });
  return label !== null;
}

/**
 * Open one of two optional Mini companions. Tauri windows are an enhancement;
 * browsers or Linux environments without usable multi-window support receive a
 * single bounded in-shell fallback instead.
 */
export async function openCompanion(nextSurface: CompanionSurface): Promise<boolean> {
  const label = await bridgeInvokeOptional<string>("open_companion", {
    companion: nextSurface,
  });
  if (label !== null) {
    setInlineCompanionSurface(null);
    return true;
  }
  setInlineCompanionSurface(nextSurface);
  return false;
}

export function closeInlineCompanion(): void {
  setInlineCompanionSurface(null);
}

/**
 * Track current Tauri window focus, hydrate late-joining approval mirrors, and
 * receive presentation-only cross-window updates. No execution command lives
 * here; decisions continue through bridge/approval.ts.
 */
export async function initWindowPresentation(): Promise<void> {
  if (initialized) return;
  initialized = true;
  if (!isTauriAvailable()) {
    setActive(true);
    return;
  }

  try {
    const current = getCurrentWindow();
    setActive(await current.isFocused());
    unlisteners.push(await current.onFocusChanged((event) => setActive(event.payload)));
    unlisteners.push(await listen<SurfaceContextPayload>(SURFACE_CONTEXT_EVENT, (event) => {
      const nextSurface = event.payload.surface ?? null;
      if (!isDetachableSurface(nextSurface)) return;
      setSurface(nextSurface);
      setContext(event.payload.context ?? null);
    }));
    unlisteners.push(await listen<ApprovalResolvedPayload>(APPROVAL_RESOLVED_EVENT, (event) => {
      if (event.payload.id) approvalStore.dismiss(event.payload.id);
    }));

    const pending = await bridgeInvokeOptional<unknown[]>("get_pending_approval_presentations");
    for (const raw of pending ?? []) {
      const envelope = coerceApprovalEnvelope(raw);
      if (envelope) approvalStore.addFromEnvelope(envelope);
    }
  } catch {
    // Multi-window/focus APIs are Linux/DE enhancements. Main in-app surface
    // remains active and usable when unavailable.
    setActive(true);
  }
}

export function disposeWindowPresentation(): void {
  for (const unlisten of unlisteners) {
    try { unlisten(); } catch { /* already disposed */ }
  }
  unlisteners = [];
  initialized = false;
}

/** Test seam for deterministic active-window behavior. */
export function setWindowPresentationActive(value: boolean): void {
  setActive(value);
}

export const windowPresentation = {
  surface,
  context,
  isActive: active,
  isDetached: () => isDetachableSurface(surface()),
  isCompanion: () => isCompanionSurface(surface()),
} as const;

export const inlineCompanion = {
  surface: inlineCompanionSurface,
  close: closeInlineCompanion,
} as const;
