/**
 * Settings Space state. Reads only KRIA's config authority and sends natural-
 * language changes through `config_prompt` (Intent → Policy → ConfigService).
 * No setting is mutated optimistically or routed to a tool from this store.
 * Requirements: 10.1, 10.2.
 */
import { createSignal } from "solid-js";
import { bridgeInvoke } from "../bridge/invoke";
import { applyAccessibilityPreferences } from "../platform/accessibilityPreferences";
import { eventBus, type Unsubscribe } from "./eventBus";
import { setLocale } from "./i18n";
import { shellStore } from "./shellStore";

export type SettingsGroup =
  | "you"
  | "voice"
  | "intelligence"
  | "memory-privacy"
  | "safety-approvals"
  | "connections"
  | "system"
  | "developer";

export interface SettingsGroupDefinition {
  id: SettingsGroup;
  label: string;
  description: string;
}

export const SETTINGS_GROUPS: readonly SettingsGroupDefinition[] = [
  { id: "you", label: "You", description: "Appearance and personal preferences" },
  { id: "voice", label: "Voice", description: "Listening, speech, and audio behavior" },
  { id: "intelligence", label: "Intelligence", description: "Models, reasoning, and orchestration" },
  { id: "memory-privacy", label: "Memory & Privacy", description: "Recall, retention, and local data" },
  { id: "safety-approvals", label: "Safety & Approvals", description: "Autonomy, policy, and consent" },
  { id: "connections", label: "Connections", description: "External services and network links" },
  { id: "system", label: "System", description: "Hardware, runtime, and desktop behavior" },
  { id: "developer", label: "Developer", description: "Advanced runtime configuration" },
] as const;

export const FEATURE_WORKSPACE_OWNERS = {
  n8n: "automations",
  skills: "capabilities",
  openclaw: "capabilities",
  clawhub: "capabilities",
  skill_compiler: "capabilities",
  providers: "capabilities",
  mcp: "capabilities",
  mobile: "machines",
  mobile_gateway: "machines",
} as const;

export type FeatureWorkspaceSection = keyof typeof FEATURE_WORKSPACE_OWNERS;

/**
 * Feature substrates are managed by their owning Space, never by Settings.
 * This is a presentation boundary only: execution still follows KRIA's
 * Intent → Capability → Policy → Substrate → Tool → Verification pipeline.
 */
export function isFeatureWorkspaceSection(section: string): section is FeatureWorkspaceSection {
  return Object.prototype.hasOwnProperty.call(FEATURE_WORKSPACE_OWNERS, section);
}

const SECTION_GROUP: Readonly<Record<string, SettingsGroup>> = {
  ui: "you", voice: "voice",
  llm: "intelligence", classifier: "intelligence", agent: "intelligence",
  routing: "intelligence", orchestrator: "intelligence", executive: "intelligence",
  planner: "intelligence", uncertainty: "intelligence", curiosity: "intelligence",
  memory: "memory-privacy", search: "memory-privacy",
  safety: "safety-approvals", capability: "safety-approvals",
  server: "connections", telegram: "connections", colab: "connections", ntfy: "connections",
  hardware: "system", image_generation: "system", remote_desktop: "system",
  browser_agent: "developer",
};

interface ConfigFieldSchema {
  risk?: string;
  restart_required?: boolean;
  env_locked?: boolean;
  env_lock_var?: string | null;
  secret?: boolean;
  non_functional?: boolean;
  valid_values?: string[] | null;
}

type ConfigSchema = Record<string, Record<string, ConfigFieldSchema>>;

export interface SettingMeta {
  key: string;
  section: string;
  field: string;
  label: string;
  description?: string;
  group: SettingsGroup;
  type: "string" | "number" | "boolean" | "select" | "json";
  risk: "none" | "low" | "medium" | "high";
  requiresRestart: boolean;
  envLocked: boolean;
  envLockVar?: string;
  secret: boolean;
  options?: string[];
}

export interface SettingsChangeRecord {
  key: string;
  previousValue: unknown;
  newValue: unknown;
  changedAt: string | number;
  source: "user" | "nl" | "system";
}

export interface NaturalLanguageResult {
  status: string;
  message?: string;
  question?: string;
  reason?: string;
  section?: string;
  field?: string;
}

interface PatchConfigResult {
  status: "applied";
  section: string;
  field: string;
  version: number;
}

function humanize(value: string): string {
  return value.replace(/_/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function settingType(value: unknown, options?: string[] | null): SettingMeta["type"] {
  if (options?.length) return "select";
  if (typeof value === "boolean") return "boolean";
  if (typeof value === "number") return "number";
  if (typeof value === "string") return "string";
  return "json";
}

function risk(value: string | undefined): SettingMeta["risk"] {
  switch (value?.toLowerCase()) {
    case "green":
    case "low":
      return "low";
    case "yellow":
    case "medium":
      return "medium";
    case "red":
    case "black":
    case "high":
      return "high";
    default:
      return "none";
  }
}

export function normalizeSettingsSchema(
  config: Record<string, unknown>,
  configSchema: ConfigSchema,
): SettingMeta[] {
  const result: SettingMeta[] = [];
  for (const [section, fields] of Object.entries(configSchema)) {
    // Feature workspaces own substrate setup and operation (Req 10.5).
    if (isFeatureWorkspaceSection(section)) continue;
    const values = config[section];
    if (!values || typeof values !== "object" || Array.isArray(values)) continue;
    for (const [field, meta] of Object.entries(fields)) {
      if (meta.non_functional) continue;
      const value = (values as Record<string, unknown>)[field];
      if (value === undefined) continue;
      result.push({
        key: `${section}.${field}`,
        section,
        field,
        label: humanize(field),
        group: SECTION_GROUP[section] ?? "developer",
        type: settingType(value, meta.valid_values),
        risk: risk(meta.risk),
        requiresRestart: Boolean(meta.restart_required),
        envLocked: Boolean(meta.env_locked),
        envLockVar: meta.env_lock_var ?? undefined,
        secret: Boolean(meta.secret),
        options: meta.valid_values ?? undefined,
      });
    }
  }
  return result.sort((a, b) => a.label.localeCompare(b.label));
}

export function normalizeConfigHistory(payload: unknown): SettingsChangeRecord[] {
  const entries = Array.isArray(payload) ? payload : [];
  return entries.slice(0, 50).map((entry) => {
    const item = (entry ?? {}) as Record<string, unknown>;
    const change = (item.change ?? {}) as Record<string, unknown>;
    const rawSource = String(change.source ?? "system").toLowerCase();
    const source = rawSource.includes("prompt") || rawSource === "nl"
      ? "nl" : rawSource.includes("ui") || rawSource === "user" ? "user" : "system";
    const section = String(change.section ?? "unknown");
    const field = String(change.field ?? "unknown");
    return {
      key: `${section}.${field}`,
      previousValue: change.prior,
      newValue: change.new,
      changedAt: String(item.timestamp ?? ""),
      source,
    };
  });
}

export function settingValue(config: Record<string, unknown>, meta: SettingMeta): unknown {
  const section = config[meta.section];
  return section && typeof section === "object" && !Array.isArray(section)
    ? (section as Record<string, unknown>)[meta.field]
    : undefined;
}

export function settingMatches(meta: SettingMeta, value: unknown, query: string): boolean {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;
  return [meta.label, meta.key, meta.group, String(value ?? "")]
    .some((candidate) => candidate.toLocaleLowerCase().includes(needle));
}

const [settings, setSettings] = createSignal<Record<string, unknown>>({});
const [schema, setSchema] = createSignal<SettingMeta[]>([]);
const [history, setHistory] = createSignal<SettingsChangeRecord[]>([]);
const [searchQuery, setSearchQuery] = createSignal("");
const [activeGroup, setActiveGroup] = createSignal<SettingsGroup>("you");
const [loading, setLoading] = createSignal(false);
const [error, setError] = createSignal<string | null>(null);
const [nlDraft, setNlDraft] = createSignal("");
const [nlBusy, setNlBusy] = createSignal(false);
const [nlResult, setNlResult] = createSignal<NaturalLanguageResult | null>(null);
const [savingKeys, setSavingKeys] = createSignal<readonly string[]>([]);
const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});

let configSubscription: Unsubscribe | null = null;
let refreshQueued = false;
let loadGeneration = 0;

function projectRuntimeSettings(config: Record<string, unknown>): void {
  applyAccessibilityPreferences(config);
  const ui = config.ui && typeof config.ui === "object" && !Array.isArray(config.ui)
    ? config.ui as Record<string, unknown>
    : {};
  if (ui.theme === "dark" || ui.theme === "light") shellStore.setTheme(ui.theme);
  if (typeof ui.language === "string") setLocale(ui.language);
}

async function load(): Promise<void> {
  const generation = ++loadGeneration;
  setLoading(true);
  setError(null);
  const [configResult, schemaResult, historyResult] = await Promise.all([
    bridgeInvoke<Record<string, unknown>>("get_settings"),
    bridgeInvoke<ConfigSchema>("get_config_schema"),
    bridgeInvoke<{ history?: unknown[] }>("get_config_history", { limit: 50 }),
  ]);
  if (generation !== loadGeneration) return;
  if (configResult.ok && schemaResult.ok) {
    setSettings(configResult.data);
    projectRuntimeSettings(configResult.data);
    setSchema(normalizeSettingsSchema(configResult.data, schemaResult.data));
  } else {
    const message = !configResult.ok
      ? configResult.message
      : !schemaResult.ok
        ? schemaResult.message
        : "Settings authority is unavailable.";
    setError(message);
  }
  setHistory(historyResult.ok ? normalizeConfigHistory(historyResult.data.history) : []);
  setLoading(false);
}

function scheduleAuthoritativeRefresh(): void {
  if (refreshQueued) return;
  refreshQueued = true;
  queueMicrotask(() => {
    refreshQueued = false;
    void load();
  });
}

async function initialize(): Promise<void> {
  if (!configSubscription) {
    configSubscription = eventBus.on("config:changed", scheduleAuthoritativeRefresh, "microtask");
  }
  await load();
}

function disposeRuntime(): void {
  configSubscription?.();
  configSubscription = null;
  refreshQueued = false;
  loadGeneration += 1;
}

function isSaving(key: string): boolean {
  return savingKeys().includes(key);
}

function fieldError(key: string): string | undefined {
  return fieldErrors()[key];
}

async function updateSetting(meta: SettingMeta, value: unknown): Promise<boolean> {
  if (meta.secret) {
    setFieldErrors((current) => ({ ...current, [meta.key]: "Use this setting's secure credential flow." }));
    return false;
  }
  if (meta.envLocked) {
    setFieldErrors((current) => ({
      ...current,
      [meta.key]: `Locked by ${meta.envLockVar ?? "an environment variable"}.`,
    }));
    return false;
  }
  if (isSaving(meta.key)) return false;

  const previous = settingValue(settings(), meta);
  setSavingKeys((current) => [...current, meta.key]);
  setFieldErrors((current) => {
    const next = { ...current };
    delete next[meta.key];
    return next;
  });
  const result = await bridgeInvoke<PatchConfigResult>(
    "patch_config",
    { section: meta.section, field: meta.field, value },
    { timeoutMs: 35_000 },
  );
  setSavingKeys((current) => current.filter((key) => key !== meta.key));
  if (!result.ok) {
    setFieldErrors((current) => ({ ...current, [meta.key]: result.message }));
    return false;
  }

  await load();
  eventBus.emit("settings:changed", { key: meta.key, value, previous });
  return true;
}

async function updateSettingByKey(key: string, value: unknown): Promise<boolean> {
  let meta = schema().find((candidate) => candidate.key === key);
  if (!meta) {
    await load();
    meta = schema().find((candidate) => candidate.key === key);
  }
  if (!meta) {
    setError(`Config schema does not expose ${key}.`);
    return false;
  }
  return updateSetting(meta, value);
}

function stageNaturalLanguageChange(text: string): void {
  setNlDraft(text.trim());
  setNlResult(null);
}

async function submitNaturalLanguageChange(): Promise<void> {
  const prompt = nlDraft().trim();
  if (!prompt || nlBusy()) return;
  setNlBusy(true);
  setNlResult(null);
  const result = await bridgeInvoke<NaturalLanguageResult>(
    "config_prompt",
    { prompt },
    { timeoutMs: 35_000 },
  );
  if (result.ok) {
    setNlResult(result.data);
    if (["applied", "undone"].includes(result.data.status)) await load();
  } else {
    setNlResult({ status: result.code, message: result.message });
  }
  setNlBusy(false);
}

export const settingsStore = {
  settings, schema, history, searchQuery, activeGroup, loading, error,
  nlDraft, nlBusy, nlResult, savingKeys, fieldErrors,
  setSettings, setSchema, setHistory, setSearchQuery, setActiveGroup,
  setNlDraft, setNlResult, load, initialize, disposeRuntime,
  isSaving, fieldError, updateSetting, updateSettingByKey, stageNaturalLanguageChange,
  submitNaturalLanguageChange,
} as const;
