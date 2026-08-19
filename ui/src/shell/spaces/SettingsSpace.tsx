/**
 * Searchable Settings Space. Natural-language changes are submitted only to
 * KRIA's config authority; this surface never dispatches tools or mutates a
 * frontend-only preference. Requirements: 10.1, 10.2.
 */
import { ErrorBoundary, For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, untrack } from "solid-js";
import { Badge, Button, Card, Chip, Input, Search, Select, Textarea } from "../../kit";
import { Icon } from "../../components/Icon";
import {
  SETTINGS_GROUPS,
  settingsStore,
  settingIsDefault,
  settingIsRelevant,
  settingMatches,
  settingValue,
  type NaturalLanguageResult,
  type SettingMeta,
  type SettingsGroup,
} from "../../stores/settingsStore";
import { currentRoute, navigate } from "../router";
import { FeatureControlsSection } from "./settings/FeatureControlsSection";
import { AwarenessPanel } from "./settings/AwarenessPanel";
import { PresencePreferencesPanel } from "./settings/PresencePreferencesPanel";
// The LLM provider editor lives in the capabilities space because that is where it
// was first built. It is rendered HERE as well because Settings is where a user goes
// to add a provider — before this, the "AI & Models" group offered a single legacy
// routing dropdown with three choices while the real editor (all seven provider
// types, API key, endpoint, model) sat one space away and was never found.
//
// The same component, not a copy: all of its state lives in `capabilityStore`, so
// there is one editor and one source of truth regardless of which space mounts it.
import { ModelsRuntimePanel } from "./capabilities";
import { capabilityStore } from "../../stores/capabilityStore";
import "./SettingsSpace.css";

type SpecialSettingsArea = {
  id: "presence" | "awareness" | "features";
  group: SettingsGroup;
  label: string;
  description: string;
  keywords: string;
};

const SPECIAL_SETTINGS_AREAS: readonly SpecialSettingsArea[] = [
  {
    id: "presence",
    group: "you",
    label: "Presence & Companion",
    description: "Control KRIA's floating Companion presence.",
    keywords: "companion ember floating window presence appearance",
  },
  {
    id: "awareness",
    group: "memory-privacy",
    label: "Desktop awareness",
    description: "Choose what KRIA may sense and what can be remembered.",
    keywords: "sense sensing desktop active window accessibility privacy remember ephemeral",
  },
  {
    id: "features",
    group: "system",
    label: "Features & Services",
    description: "Manage local feature services and inspect their actual runtime state.",
    keywords: "feature controls services runtime enable disable system",
  },
] as const;

function groupDefinition(group: SettingsGroup) {
  return SETTINGS_GROUPS.find((candidate) => candidate.id === group) ?? SETTINGS_GROUPS[0];
}

function specialAreaMatches(area: SpecialSettingsArea, query: string): boolean {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return false;
  return [area.label, area.description, area.keywords, groupDefinition(area.group).label]
    .some((value) => value.toLocaleLowerCase().includes(needle));
}

function resultMessage(result: NaturalLanguageResult): string {
  if (result.question) return result.question;
  if (result.message) return result.message;
  if (result.reason) return result.reason;
  if (result.status === "applied") return `Changed ${result.section}.${result.field}.`;
  if (result.status === "undone") return `Restored ${result.section}.${result.field}.`;
  if (result.status === "not_a_change") return "That does not look like a settings change.";
  if (result.status === "nothing_to_undo") return "No settings change is available to undo.";
  if (result.status === "needs_approval") return "Approval is required in the Approval Center.";
  return `Config authority returned: ${result.status.replace(/_/g, " ")}.`;
}

function formatHistoryValue(value: unknown): string {
  if (value === undefined) return "unset";
  if (typeof value === "string") return value || "empty";
  try { return JSON.stringify(value); } catch { return String(value); }
}

function formatTime(value: string | number): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "Recorded change" : date.toLocaleString();
}

function riskTone(risk: "none" | "low" | "medium" | "high") {
  if (risk === "high") return "danger" as const;
  if (risk === "medium") return "warning" as const;
  if (risk === "low") return "success" as const;
  return "neutral" as const;
}

function riskLabel(risk: "none" | "low" | "medium" | "high"): string {
  return risk === "none" ? "None" : `${risk[0].toUpperCase()}${risk.slice(1)}`;
}

/**
 * One sentence explaining what a risk tier MEANS for the user, or `null` when there
 * is nothing worth saying.
 *
 * `none` returns null on purpose. It used to read "No additional risk classification
 * is available for this raw field." — a sentence whose entire content is that there is
 * no content. The Risk badge beside it already says "None".
 */
function riskExplanation(risk: "none" | "low" | "medium" | "high"): string | null {
  // Shortened from "High-impact change: explicit approval is required and runtime
  // safety or exposure may change." Both halves said the same thing twice; what the
  // user needs to know is that it is gated and that it can change exposure.
  if (risk === "high") return "Needs approval; can change safety or exposure.";
  if (risk === "medium") return "KRIA asks for approval before applying it.";
  if (risk === "low") return "KRIA can apply this directly.";
  return null;
}

/**
 * Longest description shown in full. Beyond this it is clamped to one line behind a
 * "More" toggle.
 *
 * 70 of the 76 settings carry a description and 25 of those run past 60 characters, so
 * every screenful was mostly explanatory prose with the controls pushed apart. Short
 * one-liners are genuinely useful and cost nothing, so they stay visible; only the
 * paragraphs are folded away. Nothing is deleted — the user can still read all of it.
 */
const DESCRIPTION_INLINE_LIMIT = 60;

/** A setting's description: inline when short, clamped with a toggle when long. */
function SettingDescription(props: { text: string }) {
  const [expanded, setExpanded] = createSignal(false);
  const isLong = () => props.text.length > DESCRIPTION_INLINE_LIMIT;

  return (
    <Show when={isLong()} fallback={<p class="kria-settings__row-description">{props.text}</p>}>
      <p
        class="kria-settings__row-description"
        classList={{ "kria-settings__row-description--clamped": !expanded() }}
      >
        {props.text}
      </p>
      <button
        type="button"
        class="kria-settings__description-toggle kit-focusable"
        aria-expanded={expanded()}
        onClick={() => setExpanded((open) => !open)}
      >
        {expanded() ? "Less" : "More"}
      </button>
    </Show>
  );
}

function editorText(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

function parseEditorValue(meta: SettingMeta, raw: string, current: unknown): unknown {
  if (typeof current === "boolean" || meta.type === "boolean") return raw === "true";
  if (typeof current === "number" || meta.type === "number") {
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) throw new Error("Enter a valid number.");
    return parsed;
  }
  if (typeof current === "object" || meta.type === "json") {
    try {
      return JSON.parse(raw);
    } catch {
      throw new Error("Enter valid JSON.");
    }
  }
  return raw;
}

function optionItems(meta: SettingMeta) {
  return (meta.options ?? []).map((option) => typeof option === "string"
    ? { value: option, label: option, description: undefined }
    : option);
}

function sentinelFor(meta: SettingMeta, value: unknown) {
  const serialized = editorText(value);
  return (meta.sentinels ?? []).find((sentinel) => sentinel.value === serialized);
}

function rangeOutput(meta: SettingMeta, raw: string): string {
  const value = Number(raw);
  if (!Number.isFinite(value)) return raw;
  if (meta.unit === "%") return `${Math.round(value * 100)}%`;
  return `${raw}${meta.unit ? ` ${meta.unit}` : ""}`;
}

function SettingEditor(props: { meta: SettingMeta }) {
  const currentValue = createMemo(() => settingValue(settingsStore.settings(), props.meta));
  const [draft, setDraft] = createSignal(editorText(currentValue()));
  const [listEntry, setListEntry] = createSignal("");
  const [parseError, setParseError] = createSignal<string>();
  const [saved, setSaved] = createSignal(false);
  const disabled = createMemo(() => props.meta.envLocked || settingsStore.isSaving(props.meta.key));
  const error = createMemo(() => parseError() ?? settingsStore.fieldError(props.meta.key));
  const dirty = createMemo(() => draft() !== editorText(currentValue()));
  const canReset = createMemo(() => !props.meta.secret && !disabled()
    && Boolean(props.meta.hasDefault) && !settingIsDefault(props.meta, currentValue()));
  const activeSentinel = createMemo(() => sentinelFor(props.meta, currentValue()));
  const controlId = () => `setting-${props.meta.key.replace(/[^a-z0-9_-]/gi, "-")}`;
  const options = createMemo(() => optionItems(props.meta));
  const selectedOption = createMemo(() => options().find((option) => option.value === editorText(currentValue())));
  const listDraft = createMemo<string[]>(() => {
    if (props.meta.type !== "list") return [];
    try {
      const parsed = JSON.parse(draft());
      return Array.isArray(parsed) ? parsed.map(String) : [];
    } catch {
      return [];
    }
  });

  createEffect(() => setDraft(editorText(currentValue())));

  const commitValue = async (value: unknown) => {
    setParseError(undefined);
    setSaved(false);
    setSaved(await settingsStore.updateSetting(props.meta, value));
  };

  const commit = async (raw: string) => {
    try {
      await commitValue(parseEditorValue(props.meta, raw, currentValue()));
    } catch (cause) {
      setParseError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const reset = () => {
    if (canReset()) void commitValue(props.meta.defaultValue);
  };

  if (props.meta.secret) {
    return (
      <div class="kria-settings__editor kria-settings__editor--secret">
        <span class="kria-settings__value">Credential value is never shown</span>
        <small>
          {props.meta.secretAction === "manage"
            ? "Use the secure credential action for this integration."
            // Shortened from "Managed outside Settings. Use the owning integration or
            // launch environment; generic config editing is intentionally blocked." The
            // user cannot act on WHY it is blocked, only on where to go instead.
            : "Set this in the owning integration, not here."}
        </small>
      </div>
    );
  }

  if (props.meta.type === "boolean") {
    return (
      <div class="kria-settings__editor kria-settings__editor--switch">
        <label class="kria-settings__switch" for={controlId()}>
          <span>{settingsStore.isSaving(props.meta.key) ? "Saving…" : currentValue() ? "On" : "Off"}</span>
          <input
            id={controlId()}
            type="checkbox"
            role="switch"
            class="kria-settings__switch-input kit-focusable"
            checked={Boolean(currentValue())}
            disabled={disabled()}
            aria-label={`${props.meta.label}: ${currentValue() ? "On" : "Off"}`}
            onChange={(event) => void commitValue(event.currentTarget.checked)}
          />
        </label>
        <Show when={canReset()}><Button variant="ghost" size="sm" onClick={reset}>Reset to default</Button></Show>
        <Show when={saved()}><span class="kria-settings__saved" role="status">Saved</span></Show>
        <Show when={error()}>{(message) => <span class="kria-settings__field-error" role="alert">{message()}</span>}</Show>
        <Show when={props.meta.envLocked && props.meta.envLockVar}>
          <small class="kria-settings__managed-note">Managed by <code>{props.meta.envLockVar}</code>. Change or remove it in KRIA's launch environment, then restart.</small>
        </Show>
      </div>
    );
  }

  if (props.meta.type === "list") {
    const addItem = () => {
      const value = listEntry().trim();
      if (!value || listDraft().includes(value)) return;
      setDraft(JSON.stringify([...listDraft(), value]));
      setListEntry("");
      setSaved(false);
    };
    return (
      <div class="kria-settings__editor kria-settings__editor--list">
        <div class="kria-settings__list-values">
          <For each={listDraft()} fallback={<span class="kria-settings__empty-value">No entries</span>}>
            {(item) => <Chip onRemove={() => { setDraft(JSON.stringify(listDraft().filter((value) => value !== item))); setSaved(false); }} removeLabel={`Remove ${item}`}>{item}</Chip>}
          </For>
        </div>
        <div class="kria-settings__list-add">
          <Input label={`Add to ${props.meta.label}`} hideLabel value={listEntry()} placeholder="Add an entry" disabled={disabled()} onChange={setListEntry} />
          <Button type="button" variant="secondary" size="sm" disabled={disabled() || !listEntry().trim()} onClick={addItem}>Add</Button>
        </div>
        <div class="kria-settings__editor-actions">
          <Button size="sm" disabled={disabled() || !dirty()} onClick={() => void commit(draft())}>{settingsStore.isSaving(props.meta.key) ? "Saving…" : "Save list"}</Button>
          <Show when={canReset()}><Button variant="ghost" size="sm" onClick={reset}>Reset to default</Button></Show>
        </div>
        <Show when={error()}>{(message) => <span class="kria-settings__field-error" role="alert">{message()}</span>}</Show>
      </div>
    );
  }

  return (
    <div class="kria-settings__editor">
      <Show
        when={options().length > 0}
        fallback={
          <form class="kria-settings__editor-form" onSubmit={(event) => { event.preventDefault(); void commit(draft()); }}>
            <Show when={props.meta.type === "json"} fallback={
              <>
                <Show when={props.meta.type === "range" && props.meta.minimum !== undefined && props.meta.maximum !== undefined}>
                  <div class="kria-settings__range">
                    <input
                      aria-label={`${props.meta.label} slider`}
                      type="range"
                      min={props.meta.minimum}
                      max={props.meta.maximum}
                      step={props.meta.step ?? "any"}
                      value={draft()}
                      disabled={disabled()}
                      onInput={(event) => { setDraft(event.currentTarget.value); setSaved(false); }}
                    />
                    <output>{rangeOutput(props.meta, draft())}</output>
                  </div>
                </Show>
                <div class="kria-settings__input-with-unit">
                  <Input
                    label={`Value for ${props.meta.label}`}
                    hideLabel
                    type={props.meta.type === "number" || props.meta.type === "range" ? "number" : props.meta.editor === "url" ? "url" : "text"}
                    value={draft()}
                    disabled={disabled()}
                    placeholder={activeSentinel()?.label}
                    inputProps={{ min: props.meta.minimum, max: props.meta.maximum, step: props.meta.step }}
                    onChange={(value) => { setDraft(value); setSaved(false); }}
                  />
                  <Show when={props.meta.unit && props.meta.unit !== "%"}><span class="kria-settings__unit">{props.meta.unit}</span></Show>
                </div>
              </>
            }>
              <Textarea label={`JSON value for ${props.meta.label}`} hideLabel rows={5} value={draft()} disabled={disabled()} onChange={(value) => { setDraft(value); setSaved(false); }} />
            </Show>
            <Show when={sentinelFor(props.meta, draft())}>{(sentinel) => <small class="kria-settings__sentinel"><strong>{sentinel().label}:</strong> {sentinel().description}</small>}</Show>
            <div class="kria-settings__editor-actions">
              <Button type="submit" size="sm" disabled={disabled() || !dirty()}>{settingsStore.isSaving(props.meta.key) ? "Saving…" : "Save changes"}</Button>
              <Show when={canReset()}><Button type="button" variant="ghost" size="sm" onClick={reset}>Reset to default</Button></Show>
              <Show when={dirty()}><span class="kria-settings__dirty">Unsaved</span></Show>
            </div>
          </form>
        }
      >
        <Select label={`Value for ${props.meta.label}`} hideLabel options={options()} value={editorText(currentValue())} disabled={disabled()} onChange={(value) => { if (value !== undefined) void commitValue(value); }} />
        <Show when={selectedOption()?.description}><small class="kria-settings__option-help">{selectedOption()?.description}</small></Show>
        <Show when={canReset()}><Button variant="ghost" size="sm" onClick={reset}>Reset to default</Button></Show>
      </Show>
      <Show when={saved()}><span class="kria-settings__saved" role="status">Saved</span></Show>
      <Show when={error()}>{(message) => <span class="kria-settings__field-error" role="alert">{message()}</span>}</Show>
      <Show when={props.meta.envLocked && props.meta.envLockVar}>
        <small class="kria-settings__managed-note">Managed by <code>{props.meta.envLockVar}</code>. Change or remove it in KRIA's launch environment, then restart.</small>
      </Show>
    </div>
  );
}

export default function SettingsSpace() {
  const [developerUnlocked, setDeveloperUnlocked] = createSignal(false);
  const [developerGuardArmed, setDeveloperGuardArmed] = createSignal(false);
  const [assistantOpen, setAssistantOpen] = createSignal(false);
  const [historyOpen, setHistoryOpen] = createSignal(false);
  let focusedSettingRoute: string | null = null;
  let historyTrigger: HTMLButtonElement | undefined;

  // Settings palette/hash deep links select the schema-backed category, reveal
  // the row, then move keyboard focus to it. Advanced routes still stop at the
  // deliberate guard and never auto-unlock dangerous settings.
  createEffect(() => {
    const route = currentRoute();
    const schema = settingsStore.schema();
    if (route.space !== "settings") return;
    const group = SETTINGS_GROUPS.find((item) => item.id === route.segment);
    if (!group) return;

    if (settingsStore.activeGroup() !== group.id) settingsStore.setActiveGroup(group.id);
    if (untrack(settingsStore.searchQuery)) settingsStore.setSearchQuery("");
    if (!route.entityId) return;

    const setting = schema.find((item) => item.key === route.entityId && item.group === group.id);
    if (!setting || (setting.group === "developer" && !developerUnlocked())) return;
    const routeKey = `${route.space}/${route.segment}/${route.entityId}`;
    if (focusedSettingRoute === routeKey) return;

    queueMicrotask(() => {
      if (currentRoute().entityId !== setting.key) return;
      const row = Array.from(
        document.querySelectorAll<HTMLElement>("[data-setting-key]"),
      ).find((element) => element.dataset.settingKey === setting.key);
      if (!row) return;
      row.scrollIntoView?.({ block: "center" });
      row.focus({ preventScroll: true });
      focusedSettingRoute = routeKey;
    });
  });

  onMount(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (historyOpen()) {
        event.preventDefault();
        setHistoryOpen(false);
        queueMicrotask(() => historyTrigger?.focus({ preventScroll: true }));
      } else if (assistantOpen()) {
        event.preventDefault();
        setAssistantOpen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    onCleanup(() => window.removeEventListener("keydown", onKeyDown));
  });

  // The provider panel reads `capabilityStore`, which is populated by whichever space
  // asked for it. The capabilities space loads the "models" segment when its Models
  // tab opens; Settings has no such trigger, so without this the panel would mount
  // against an empty store and honestly report "no providers" for an install that has
  // several. Loading is keyed on the group so it costs nothing until the user actually
  // opens AI & Models.
  //
  // `loadedProviders` guards against re-fetching on every unrelated signal change in
  // this effect's scope — the user switching group away and back is a deliberate
  // refresh, a re-render is not.
  let loadedProviders = false;
  createEffect(() => {
    const isIntelligence = settingsStore.activeGroup() === "intelligence";
    if (!isIntelligence) {
      loadedProviders = false;
      return;
    }
    if (loadedProviders) return;
    loadedProviders = true;
    void capabilityStore.loadSegment("models").catch(() => {
      // A failed load leaves the panel's own honest empty/error state in place; the
      // store owns that messaging, so swallowing here avoids a second toast for one
      // failure.
      loadedProviders = false;
    });
  });

  const searchQuery = createMemo(() => settingsStore.searchQuery().trim());
  const isSearching = createMemo(() => searchQuery().length > 0);
  const matchedSettings = createMemo(() => settingsStore.schema().filter((meta) => {
    const value = settingValue(settingsStore.settings(), meta);
    const groupMatch = isSearching() ? true : meta.group === settingsStore.activeGroup();
    return groupMatch && settingMatches(meta, value, searchQuery());
  }));
  const hiddenDeveloperCount = createMemo(() => developerUnlocked()
    ? 0
    : matchedSettings().filter((meta) => meta.group === "developer").length);
  const visibleSettings = createMemo(() => matchedSettings().filter(
    (meta) => (meta.group !== "developer" || developerUnlocked())
      && (isSearching() || settingIsRelevant(meta, settingsStore.settings())),
  ));
  const visibleSubsections = createMemo(() => {
    const groups = new Map<string, {
      key: string;
      label: string;
      description?: string;
      settings: SettingMeta[];
    }>();
    for (const meta of visibleSettings()) {
      const label = meta.subsection ?? groupDefinition(meta.group).label;
      const key = `${meta.group}:${label}`;
      const existing = groups.get(key);
      if (existing) existing.settings.push(meta);
      else groups.set(key, { key, label, description: meta.subsectionDescription, settings: [meta] });
    }
    return [...groups.values()];
  });
  const specialResults = createMemo(() => SPECIAL_SETTINGS_AREAS.filter(
    (area) => specialAreaMatches(area, searchQuery()),
  ));
  const currentGroup = createMemo(() => groupDefinition(settingsStore.activeGroup()));
  const currentHasSpecialArea = createMemo(() => SPECIAL_SETTINGS_AREAS.some(
    (area) => area.group === settingsStore.activeGroup(),
  ));

  const selectGroup = (group: SettingsGroup) => {
    settingsStore.setActiveGroup(group);
    settingsStore.setSearchQuery("");
    if (group !== "developer") setDeveloperGuardArmed(false);
    navigate("settings", group);
  };
  const lockDeveloperSettings = () => {
    setDeveloperUnlocked(false);
    setDeveloperGuardArmed(false);
  };
  const closeHistory = () => {
    setHistoryOpen(false);
    queueMicrotask(() => historyTrigger?.focus({ preventScroll: true }));
  };
  const submit = (event: SubmitEvent) => {
    event.preventDefault();
    void settingsStore.submitNaturalLanguageChange();
  };

  return (
    <section class="kria-settings" data-space="settings" aria-labelledby="settings-title">
      <header class="kria-settings__header">
        <div class="kria-settings__intro">
          <span class="kria-settings__eyebrow">KRIA control center</span>
          <h1 id="settings-title">Settings</h1>
          <p>Choose a category, search a preference, or ask KRIA to make a governed change.</p>
        </div>
        <div class="kria-settings__header-controls">
          <Search
            label="Search settings"
            value={settingsStore.searchQuery()}
            placeholder="Search settings and categories…"
            onChange={settingsStore.setSearchQuery}
            class="kria-settings__search"
          />
          <div class="kria-settings__header-actions">
            <Button
              variant="ghost"
              size="sm"
              aria-expanded={assistantOpen()}
              aria-controls="settings-assistant"
              onClick={() => setAssistantOpen((open) => !open)}
            >
              <Icon name="sparkles" size={15} aria-hidden={true} />
              Ask KRIA
            </Button>
            <Button
              ref={(element) => { historyTrigger = element; }}
              variant="ghost"
              size="sm"
              aria-expanded={historyOpen()}
              aria-controls="settings-history"
              onClick={() => setHistoryOpen(true)}
            >
              <Icon name="history" size={15} aria-hidden={true} />
              Change history
            </Button>
          </div>
        </div>
      </header>

      <Show when={assistantOpen()}>
        <section id="settings-assistant" class="kria-settings__assistant" aria-labelledby="settings-assistant-title">
          <div class="kria-settings__assistant-head">
            <div>
              <h2 id="settings-assistant-title">Ask KRIA to change a setting</h2>
              <p>Requests pass through config policy, approval, persistence, and verification.</p>
            </div>
            <button
              type="button"
              class="kria-settings__close kit-focusable"
              aria-label="Close Ask KRIA"
              onClick={() => setAssistantOpen(false)}
            >
              <Icon name="x" size={16} aria-hidden={true} />
            </button>
          </div>
          <form class="kria-settings__nl" onSubmit={submit}>
            <Input
              label="Change a setting with KRIA"
              hideLabel
              value={settingsStore.nlDraft()}
              placeholder="For example: reduce interface motion"
              disabled={settingsStore.nlBusy()}
              onChange={settingsStore.setNlDraft}
              class="kria-settings__nl-input"
            />
            <Button type="submit" disabled={settingsStore.nlBusy() || !settingsStore.nlDraft().trim()}>
              {settingsStore.nlBusy() ? "Checking…" : "Send request"}
            </Button>
          </form>
          <Show when={settingsStore.nlResult()}>
            {(result) => (
              <div class="kria-settings__nl-result" role="status" aria-live="polite">
                <Badge tone={result().status === "applied" || result().status === "undone" ? "success" : result().status === "clarify" ? "info" : "warning"}>
                  {result().status.replace(/_/g, " ")}
                </Badge>
                <span>{resultMessage(result())}</span>
              </div>
            )}
          </Show>
        </section>
      </Show>

      <div class="kria-settings__layout">
        <nav class="kria-settings__groups" aria-label="Settings categories">
          <span class="kria-settings__nav-title">Categories</span>
          <ul>
            <For each={SETTINGS_GROUPS}>
              {(group) => (
                <li>
                  <button
                    type="button"
                    class="kria-settings__group-button kit-focusable"
                    classList={{
                      "kria-settings__group-button--active": !isSearching() && group.id === settingsStore.activeGroup(),
                      "kria-settings__group-button--developer": group.id === "developer",
                    }}
                    data-guarded={group.id === "developer" ? !developerUnlocked() : undefined}
                    aria-current={!isSearching() && group.id === settingsStore.activeGroup() ? "page" : undefined}
                    onClick={() => selectGroup(group.id)}
                  >
                    <span class="kria-settings__group-copy">
                      <span class="kria-settings__group-label">
                        <Show when={group.id === "developer"}><Icon name="shield-alert" size={15} aria-hidden={true} /></Show>
                        <strong>{group.label}</strong>
                      </span>
                      <small>{group.description}</small>
                    </span>
                    <Show when={group.id === "developer" && !developerUnlocked()}>
                      <Badge tone="warning">Guarded</Badge>
                    </Show>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </nav>

        <ErrorBoundary fallback={(_error, reset) => (
          <div class="kria-settings__body" role="alert" aria-labelledby="settings-category-error-title">
            <section class="kria-settings__category-error">
              <Icon name="shield-alert" size={24} aria-hidden={true} />
              <div>
                <h2 id="settings-category-error-title">This category could not be shown</h2>
                <p>The rest of Settings is still available. Return to General and try this category again.</p>
                <Button
                  size="sm"
                  onClick={() => {
                    selectGroup("you");
                    queueMicrotask(reset);
                  }}
                >
                  Return to General
                </Button>
              </div>
            </section>
          </div>
        )}>
        <div
          class="kria-settings__body"
          classList={{ "kria-settings__body--developer": developerUnlocked() && currentGroup().id === "developer" && !isSearching() }}
          aria-labelledby="settings-section-title"
        >
          <div class="kria-settings__section-head">
            <div>
              <span class="kria-settings__section-kicker">
                {isSearching() ? "All categories" : "Selected category"}
              </span>
              <h2 id="settings-section-title">{isSearching() ? "Search results" : currentGroup().label}</h2>
              <p>{isSearching() ? `Matches for “${searchQuery()}”` : currentGroup().description}</p>
            </div>
            <div class="kria-settings__section-actions">
              <Badge>{visibleSettings().length + specialResults().length} results</Badge>
              <Show when={isSearching()}>
                <Button variant="ghost" size="sm" onClick={() => settingsStore.setSearchQuery("")}>Clear search</Button>
              </Show>
              <Show when={developerUnlocked() && (currentGroup().id === "developer" || hiddenDeveloperCount() > 0)}>
                <Button variant="ghost" size="sm" onClick={lockDeveloperSettings}>Lock Advanced</Button>
              </Show>
            </div>
          </div>

          <Show when={isSearching() && specialResults().length > 0}>
            <div class="kria-settings__area-results" aria-label="Matching settings categories">
              <For each={specialResults()}>
                {(area) => (
                  <button
                    type="button"
                    class="kria-settings__area-result kit-focusable"
                    onClick={() => selectGroup(area.group)}
                  >
                    <span class="kria-settings__area-result-copy">
                      <small>{groupDefinition(area.group).label}</small>
                      <strong>{area.label}</strong>
                      <span>{area.description}</span>
                    </span>
                    <span class="kria-settings__area-result-action">Open category</span>
                  </button>
                )}
              </For>
            </div>
          </Show>

          <Show when={!isSearching() && settingsStore.activeGroup() === "intelligence"}>
            <ModelsRuntimePanel />
          </Show>
          <Show when={!isSearching() && settingsStore.activeGroup() === "you"}>
            <PresencePreferencesPanel />
          </Show>
          <Show when={!isSearching() && settingsStore.activeGroup() === "memory-privacy"}>
            <AwarenessPanel />
          </Show>
          <Show when={!isSearching() && settingsStore.activeGroup() === "system"}>
            <FeatureControlsSection />
          </Show>

          <Show when={hiddenDeveloperCount() > 0}>
            <section class="kria-settings__developer-guard" aria-labelledby="developer-guard-title">
              <span class="kria-settings__developer-guard-icon" aria-hidden="true">
                <Icon name="shield-alert" size={24} />
              </span>
              <div>
                <h3 id="developer-guard-title">Advanced settings are guarded</h3>
                <p>
                  {hiddenDeveloperCount()} advanced {hiddenDeveloperCount() === 1 ? "setting is" : "settings are"} hidden.
                  These controls can weaken runtime safety or alter orchestration. Revealing them never bypasses KRIA policy or approvals.
                </p>
                <Show
                  when={developerGuardArmed()}
                  fallback={
                    <Button variant="secondary" size="sm" onClick={() => setDeveloperGuardArmed(true)}>
                      Review advanced access
                    </Button>
                  }
                >
                  <div class="kria-settings__developer-confirm" role="group" aria-label="Confirm Advanced settings access">
                    <strong>Confirm that you understand these are potentially dangerous runtime controls.</strong>
                    <div>
                      <Button variant="ghost" size="sm" onClick={() => setDeveloperGuardArmed(false)}>Cancel</Button>
                      <Button
                        variant="danger"
                        size="sm"
                        onClick={() => { setDeveloperUnlocked(true); setDeveloperGuardArmed(false); }}
                      >
                        Reveal Advanced settings
                      </Button>
                    </div>
                  </div>
                </Show>
              </div>
            </section>
          </Show>

          <Show when={settingsStore.loading()}>
            <p class="kria-settings__state" role="status">Loading settings from config authority…</p>
          </Show>
          <Show when={settingsStore.error()}>
            {(message) => <p class="kria-settings__state kria-settings__state--error" role="alert">{message()}</p>}
          </Show>
          <Show when={!settingsStore.loading() && !settingsStore.error()
            && visibleSettings().length === 0 && specialResults().length === 0
            && hiddenDeveloperCount() === 0 && (isSearching() || !currentHasSpecialArea())}>
            <p class="kria-settings__state">{isSearching() ? "No settings or categories match this search." : "No settings are available in this category."}</p>
          </Show>

          <div class="kria-settings__rows kria-settings__subsections" role="list" aria-label={isSearching() ? "Matching settings" : `${currentGroup().label} settings`}>
            <For each={visibleSubsections()}>
              {(subsection) => (
                <section class="kria-settings__subsection" aria-labelledby={`settings-subsection-${subsection.key.replace(/[^a-z0-9_-]/gi, "-")}`}>
                  <header class="kria-settings__subsection-head">
                    <div>
                      <h3 id={`settings-subsection-${subsection.key.replace(/[^a-z0-9_-]/gi, "-")}`}>{subsection.label}</h3>
                      <Show when={subsection.description}><p>{subsection.description}</p></Show>
                    </div>
                    <Badge>{subsection.settings.length}</Badge>
                  </header>
                  <div class="kria-settings__subsection-rows">
                    <For each={subsection.settings}>
                      {(meta) => (
                        <Card
                          class="kria-settings__row"
                          classList={{ "kria-settings__row--developer": meta.group === "developer" }}
                          role="listitem"
                          data-setting-key={meta.key}
                          tabIndex={-1}
                        >
                          <div class="kria-settings__row-copy">
                            <Show when={isSearching()}>
                              <span class="kria-settings__row-category">{groupDefinition(meta.group).label} · {meta.subsection}</span>
                            </Show>
                            <strong>{meta.label}</strong>
                            <Show when={meta.description}>
                              {(description) => <SettingDescription text={description()} />}
                            </Show>
                            <Show when={isSearching() && !settingIsRelevant(meta, settingsStore.settings()) && meta.dependency}>
                              {(dependency) => <p class="kria-settings__dependency-note">{dependency().description}</p>}
                            </Show>
                            <Show when={sentinelFor(meta, settingValue(settingsStore.settings(), meta))}>
                              {(sentinel) => <span class="kria-settings__current-mode">Current: {sentinel().label}</span>}
                            </Show>
                            <Show when={meta.requiresRestart || meta.envLocked || meta.visibility === "raw"}>
                              <div class="kria-settings__badges" aria-label={`Important constraints for ${meta.label}`}>
                                <Show when={meta.requiresRestart}><Badge tone="warning">Restart required</Badge></Show>
                                <Show when={meta.envLocked}><Badge tone="info">Managed by environment</Badge></Show>
                                <Show when={meta.visibility === "raw"}><Badge tone="warning">Raw configuration</Badge></Show>
                              </div>
                            </Show>
                            <details class="kria-settings__technical">
                              <summary>Technical details</summary>
                              <code>{meta.key}</code>
                              <Show when={riskExplanation(meta.risk)}>
                                {(explanation) => (
                                  <span class="kria-settings__risk-explanation">{explanation()}</span>
                                )}
                              </Show>
                              <div class="kria-settings__badges" aria-label={`Technical constraints for ${meta.label}`}>
                                <Badge tone={riskTone(meta.risk)}>Risk: {riskLabel(meta.risk)}</Badge>
                                <Show when={meta.minimum !== undefined || meta.maximum !== undefined}>
                                  <Badge>Range: {meta.minimum ?? "−∞"}–{meta.maximum ?? "∞"}{meta.unit ? ` ${meta.unit}` : ""}</Badge>
                                </Show>
                                <Show when={meta.envLocked && meta.envLockVar}><Badge tone="info">Environment: {meta.envLockVar}</Badge></Show>
                              </div>
                            </details>
                          </div>
                          <SettingEditor meta={meta} />
                        </Card>
                      )}
                    </For>
                  </div>
                </section>
              )}
            </For>
          </div>
        </div>
        </ErrorBoundary>
      </div>

      <Show when={historyOpen()}>
        <button
          type="button"
          class="kria-settings__history-scrim"
          aria-label="Close change history"
          tabIndex={-1}
          onClick={closeHistory}
        />
        <aside id="settings-history" class="kria-settings__history" aria-labelledby="settings-history-title">
          <div class="kria-settings__history-head">
            <div>
              <span class="kria-settings__section-kicker">Verified config audit</span>
              <h2 id="settings-history-title">Change history</h2>
              <p>Recent governed changes from the UI, KRIA, and the system.</p>
            </div>
            <button type="button" class="kria-settings__close kit-focusable" aria-label="Close change history panel" onClick={closeHistory}>
              <Icon name="x" size={16} aria-hidden={true} />
            </button>
          </div>
          <Button variant="ghost" size="sm" onClick={() => void settingsStore.load()}>Refresh history</Button>
          <Show when={settingsStore.history().length > 0} fallback={<p class="kria-settings__state">No recorded changes yet.</p>}>
            <ol>
              <For each={settingsStore.history()}>
                {(entry) => (
                  <li>
                    <div><code>{entry.key}</code><Badge tone={entry.source === "nl" ? "accent" : "neutral"}>{entry.source}</Badge></div>
                    <p><span>{formatHistoryValue(entry.previousValue)}</span><span aria-hidden="true">→</span><strong>{formatHistoryValue(entry.newValue)}</strong></p>
                    <time>{formatTime(entry.changedAt)}</time>
                  </li>
                )}
              </For>
            </ol>
          </Show>
        </aside>
      </Show>
    </section>
  );
}
