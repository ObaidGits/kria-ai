import { createSignal } from "solid-js";
import { bridgeInvoke } from "../bridge/invoke";
import {
  normalizeFeatureControls,
  type FeatureControl,
  type FeatureControlState,
  type NormalizedFeatureControlsCollection,
} from "./featureControlsContract";

export type {
  FeatureControl,
  FeatureControlState,
} from "./featureControlsContract";

const TRANSITION_POLL_INTERVAL_MS = 1_000;
const MAX_TRANSITION_POLLS = 15;
const DIAGNOSTIC_LOG_LIMIT = 10;

const [collection, setCollection] = createSignal<NormalizedFeatureControlsCollection>(
  normalizeFeatureControls([]),
);
const controls = () => collection().controls;
const status = () => collection().status;
const diagnostics = () => collection().diagnostics;
const rejectedCount = () => collection().rejectedCount;
const [loading, setLoading] = createSignal(false);
const [error, setError] = createSignal<string | null>(null);
const [mutatingIds, setMutatingIds] = createSignal<readonly string[]>([]);

let active = false;
let pollTimer: ReturnType<typeof setTimeout> | null = null;
let pollAttempts = 0;
let requestGeneration = 0;

export function isFeatureTransitioning(control: FeatureControl): boolean {
  return control.state === "starting" || control.state === "stopping";
}

function clearPollTimer(): void {
  if (pollTimer !== null) clearTimeout(pollTimer);
  pollTimer = null;
}

function hasTransitioningControl(): boolean {
  return controls().some(isFeatureTransitioning);
}
function scheduleTransitionPoll(): void {
  if (!active || pollTimer !== null || !hasTransitioningControl()) return;
  if (pollAttempts >= MAX_TRANSITION_POLLS) return;

  pollTimer = setTimeout(() => {
    pollTimer = null;
    pollAttempts += 1;
    void loadControls(false);
  }, TRANSITION_POLL_INTERVAL_MS);
}

function normalizePayload(payload: unknown): NormalizedFeatureControlsCollection {
  const normalized = normalizeFeatureControls(payload);
  if (normalized.diagnostics.length > 0) {
    console.warn("[featureControlsStore] Malformed feature-control payload normalized.", {
      status: normalized.status,
      rejectedCount: normalized.rejectedCount,
      diagnostics: normalized.diagnostics.slice(0, DIAGNOSTIC_LOG_LIMIT),
    });
  }
  return normalized;
}

function setControls(payload: unknown): void {
  setCollection(normalizePayload(payload));
}

function replaceControl(payload: unknown): void {
  const normalized = normalizePayload([payload]);
  const next = normalized.controls[0];
  if (!next) return;

  const current = controls();
  const index = current.findIndex((item) => item.id === next.id);
  if (index < 0) {
    setControls([...current, next]);
    return;
  }
  const updated = [...current];
  updated[index] = next;
  setControls(updated);
}

async function loadControls(showLoading: boolean): Promise<boolean> {
  const generation = ++requestGeneration;
  if (showLoading) setLoading(true);
  const result = await bridgeInvoke<unknown>("list_feature_controls");
  if (generation !== requestGeneration) return false;

  if (result.ok) {
    setControls(result.data);
    setError(null);
  } else {
    setError(result.message);
  }
  setLoading(false);

  if (!hasTransitioningControl()) {
    clearPollTimer();
    pollAttempts = 0;
  } else {
    scheduleTransitionPoll();
  }
  return result.ok;
}

async function refresh(): Promise<void> {
  pollAttempts = 0;
  clearPollTimer();
  await loadControls(controls().length === 0);
}

async function initialize(): Promise<void> {
  active = true;
  await refresh();
}

function dispose(): void {
  active = false;
  clearPollTimer();
  pollAttempts = 0;
  requestGeneration += 1;
  setLoading(false);
  setMutatingIds([]);
}
function isMutating(id: string): boolean {
  return mutatingIds().includes(id);
}

async function setEnabled(featureId: string, enabled: boolean): Promise<boolean> {
  const current = controls().find((item) => item.id === featureId);
  if (!current || isMutating(featureId) || isFeatureTransitioning(current)) return false;

  setMutatingIds((ids) => [...ids, featureId]);
  setError(null);
  const result = await bridgeInvoke<unknown>(
    "set_feature_enabled",
    { featureId, enabled },
    { timeoutMs: 35_000 },
  );
  if (result.ok) replaceControl(result.data);

  pollAttempts = 0;
  clearPollTimer();
  await loadControls(false);
  if (!result.ok) setError(result.message);
  setMutatingIds((ids) => ids.filter((id) => id !== featureId));
  return result.ok;
}

export const featureControlsStore = {
  collection,
  controls,
  status,
  diagnostics,
  rejectedCount,
  loading,
  error,
  mutatingIds,
  setControls,
  initialize,
  refresh,
  dispose,
  isMutating,
  setEnabled,
} as const;
