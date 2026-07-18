/**
 * Searchable Settings Space. Natural-language changes are submitted only to
 * KRIA's config authority; this surface never dispatches tools or mutates a
 * frontend-only preference. Requirements: 10.1, 10.2.
 */
import { For, Show, createEffect, createMemo, createSignal } from "solid-js";
import { Badge, Button, Card, Input, Search, Select } from "../../kit";
import { Icon } from "../../components/Icon";
import {
  SETTINGS_GROUPS,
  settingsStore,
  settingMatches,
  settingValue,
  type NaturalLanguageResult,
  type SettingMeta,
} from "../../stores/settingsStore";
import { currentRoute } from "../router";
import "./SettingsSpace.css";

function displayValue(value: unknown, secret: boolean): string {
  if (secret) return "Stored securely";
  if (typeof value === "boolean") return value ? "On" : "Off";
  if (value === null || value === undefined || value === "") return "Not set";
  if (typeof value === "object") return "Configured";
  return String(value);
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

function SettingEditor(props: { meta: SettingMeta }) {
  const currentValue = createMemo(() => settingValue(settingsStore.settings(), props.meta));
  const [draft, setDraft] = createSignal(editorText(currentValue()));
  const [parseError, setParseError] = createSignal<string>();
  const disabled = createMemo(() => props.meta.envLocked || settingsStore.isSaving(props.meta.key));
  const error = createMemo(() => parseError() ?? settingsStore.fieldError(props.meta.key));

  createEffect(() => setDraft(editorText(currentValue())));

  const commit = async (raw: string) => {
    setParseError(undefined);
    try {
      const value = parseEditorValue(props.meta, raw, currentValue());
      await settingsStore.updateSetting(props.meta, value);
    } catch (cause) {
      setParseError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  if (props.meta.secret) {
    return <span class="kria-settings__value">{displayValue(currentValue(), true)}</span>;
  }

  const options = createMemo(() => {
    const values = props.meta.options ?? (props.meta.type === "boolean" ? ["true", "false"] : []);
    return values.map((value) => ({
      value,
      label: value === "true" ? "On" : value === "false" ? "Off" : value,
    }));
  });

  return (
    <div class="kria-settings__editor">
      <Show
        when={options().length > 0}
        fallback={
          <form
            class="kria-settings__editor-form"
            onSubmit={(event) => {
              event.preventDefault();
              void commit(draft());
            }}
          >
            <Input
              label={`Value for ${props.meta.label}`}
              hideLabel
              type={props.meta.type === "number" ? "number" : "text"}
              value={draft()}
              disabled={disabled()}
              errorMessage={error()}
              onChange={setDraft}
            />
            <Button type="submit" size="sm" disabled={disabled()}>
              {settingsStore.isSaving(props.meta.key) ? "Saving…" : "Save"}
            </Button>
          </form>
        }
      >
        <Select
          label={`Value for ${props.meta.label}`}
          hideLabel
          options={options()}
          value={editorText(currentValue())}
          disabled={disabled()}
          errorMessage={error()}
          onChange={(value) => { if (value !== undefined) void commit(value); }}
        />
      </Show>
    </div>
  );
}

export default function SettingsSpace() {
  const [developerUnlocked, setDeveloperUnlocked] = createSignal(false);
  const [developerGuardArmed, setDeveloperGuardArmed] = createSignal(false);
  let focusedSettingRoute: string | null = null;

  // Settings palette/hash deep links select the schema-backed group, reveal the
  // row, then move keyboard focus to it. Developer routes still stop at the
  // existing deliberate guard and never auto-unlock dangerous settings.
  createEffect(() => {
    const route = currentRoute();
    const schema = settingsStore.schema();
    if (route.space !== "settings") return;
    const group = SETTINGS_GROUPS.find((item) => item.id === route.segment);
    if (!group) return;

    if (settingsStore.activeGroup() !== group.id) settingsStore.setActiveGroup(group.id);
    if (settingsStore.searchQuery()) settingsStore.setSearchQuery("");
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

  const matchedSettings = createMemo(() => {
    const query = settingsStore.searchQuery();
    return settingsStore.schema().filter((meta) => {
      const value = settingValue(settingsStore.settings(), meta);
      const groupMatch = query.trim() ? true : meta.group === settingsStore.activeGroup();
      return groupMatch && settingMatches(meta, value, query);
    });
  });
  const hiddenDeveloperCount = createMemo(() => developerUnlocked()
    ? 0
    : matchedSettings().filter((meta) => meta.group === "developer").length);
  const visibleSettings = createMemo(() => matchedSettings().filter(
    (meta) => meta.group !== "developer" || developerUnlocked(),
  ));
  const currentGroup = createMemo(() =>
    SETTINGS_GROUPS.find((group) => group.id === settingsStore.activeGroup()) ?? SETTINGS_GROUPS[0]);
  const counts = createMemo(() => {
    const result = new Map<string, number>();
    for (const item of settingsStore.schema()) result.set(item.group, (result.get(item.group) ?? 0) + 1);
    return result;
  });
  const lockDeveloperSettings = () => {
    setDeveloperUnlocked(false);
    setDeveloperGuardArmed(false);
  };

  const submit = (event: SubmitEvent) => {
    event.preventDefault();
    void settingsStore.submitNaturalLanguageChange();
  };

  return (
    <section class="kria-settings" data-space="settings" aria-labelledby="settings-title">
      <header class="kria-settings__header">
        <div>
          <h1 id="settings-title">Settings</h1>
          <p>Find a preference or tell KRIA what you want changed.</p>
        </div>
        <Search
          label="Search settings"
          value={settingsStore.searchQuery()}
          placeholder="Search all settings…"
          onChange={settingsStore.setSearchQuery}
          class="kria-settings__search"
        />
      </header>

      <form class="kria-settings__nl" aria-labelledby="settings-change-title" onSubmit={submit}>
        <span class="kria-settings__nl-icon" aria-hidden="true"><Icon name="sparkles" size={18} /></span>
        <Input
          label="Change a setting with KRIA"
          hideLabel
          value={settingsStore.nlDraft()}
          placeholder="Change X to Y…"
          disabled={settingsStore.nlBusy()}
          onChange={settingsStore.setNlDraft}
          class="kria-settings__nl-input"
        />
        <Button type="submit" disabled={settingsStore.nlBusy() || !settingsStore.nlDraft().trim()}>
          {settingsStore.nlBusy() ? "Checking…" : "Review change"}
        </Button>
      </form>
      <p class="kria-settings__authority-note">
        Requests pass through KRIA config policy, approval, persistence, and verification.
      </p>
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

      <div class="kria-settings__layout">
        <nav class="kria-settings__groups" aria-label="Settings groups">
          <ul>
            <For each={SETTINGS_GROUPS}>
              {(group) => (
                <li>
                  <button
                    type="button"
                    class="kria-settings__group-button kit-focusable"
                    classList={{
                      "kria-settings__group-button--active": group.id === settingsStore.activeGroup(),
                      "kria-settings__group-button--developer": group.id === "developer",
                    }}
                    data-guarded={group.id === "developer" ? !developerUnlocked() : undefined}
                    aria-current={group.id === settingsStore.activeGroup() ? "page" : undefined}
                    onClick={() => {
                      settingsStore.setActiveGroup(group.id);
                      settingsStore.setSearchQuery("");
                      if (group.id !== "developer") setDeveloperGuardArmed(false);
                    }}
                  >
                    <span class="kria-settings__group-label">
                      <Show when={group.id === "developer"}><Icon name="shield-alert" size={15} /></Show>
                      <span>{group.label}</span>
                    </span>
                    <span class="kria-settings__group-meta">
                      <Show when={group.id === "developer" && !developerUnlocked()}>
                        <Badge tone="warning">Guarded</Badge>
                      </Show>
                      <small>{counts().get(group.id) ?? 0}</small>
                    </span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </nav>

        <div
          class="kria-settings__body"
          classList={{ "kria-settings__body--developer": developerUnlocked() && currentGroup().id === "developer" }}
          aria-labelledby="settings-section-title"
        >
          <div class="kria-settings__section-head">
            <div>
              <h2 id="settings-section-title">{settingsStore.searchQuery().trim() ? "Search results" : currentGroup().label}</h2>
              <p>{settingsStore.searchQuery().trim() ? `Matches for “${settingsStore.searchQuery().trim()}”` : currentGroup().description}</p>
            </div>
            <div class="kria-settings__section-actions">
              <Badge>{visibleSettings().length} settings</Badge>
              <Show when={developerUnlocked() && (currentGroup().id === "developer" || hiddenDeveloperCount() > 0)}>
                <Button variant="ghost" size="sm" onClick={lockDeveloperSettings}>Lock Developer</Button>
              </Show>
            </div>
          </div>
          <Show when={hiddenDeveloperCount() > 0}>
            <section class="kria-settings__developer-guard" aria-labelledby="developer-guard-title">
              <span class="kria-settings__developer-guard-icon" aria-hidden="true">
                <Icon name="shield-alert" size={24} />
              </span>
              <div>
                <h3 id="developer-guard-title">Developer settings are quarantined</h3>
                <p>
                  {hiddenDeveloperCount()} advanced {hiddenDeveloperCount() === 1 ? "setting is" : "settings are"} hidden.
                  Changes can weaken runtime safety or alter orchestration. Revealing them never bypasses KRIA config policy,
                  approvals, persistence, or verification.
                </p>
                <Show
                  when={developerGuardArmed()}
                  fallback={
                    <Button variant="secondary" size="sm" onClick={() => setDeveloperGuardArmed(true)}>
                      Review developer access
                    </Button>
                  }
                >
                  <div class="kria-settings__developer-confirm" role="group" aria-label="Confirm Developer settings access">
                    <strong>Confirm that you understand these are advanced, potentially dangerous settings.</strong>
                    <div>
                      <Button variant="ghost" size="sm" onClick={() => setDeveloperGuardArmed(false)}>Cancel</Button>
                      <Button
                        variant="danger"
                        size="sm"
                        onClick={() => { setDeveloperUnlocked(true); setDeveloperGuardArmed(false); }}
                      >
                        Reveal Developer settings
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
          <Show when={!settingsStore.loading() && !settingsStore.error() && visibleSettings().length === 0 && hiddenDeveloperCount() === 0}>
            <p class="kria-settings__state">No settings match this search.</p>
          </Show>
          <div class="kria-settings__rows" role="list">
            <For each={visibleSettings()}>
              {(meta) => (
                <Card
                  class="kria-settings__row"
                  classList={{ "kria-settings__row--developer": meta.group === "developer" }}
                  role="listitem"
                  data-setting-key={meta.key}
                  tabIndex={-1}
                >
                  <div class="kria-settings__row-copy">
                    <strong>{meta.label}</strong>
                    <code>{meta.key}</code>
                    <div class="kria-settings__badges" aria-label={`Setting constraints for ${meta.label}`}>
                      <Badge tone={riskTone(meta.risk)}>Risk: {riskLabel(meta.risk)}</Badge>
                      <Show when={meta.requiresRestart}><Badge tone="warning">Restart required</Badge></Show>
                      <Show when={meta.envLocked}>
                        <Badge tone="info">Environment lock{meta.envLockVar ? `: ${meta.envLockVar}` : ""}</Badge>
                      </Show>
                    </div>
                  </div>
                  <SettingEditor meta={meta} />
                </Card>
              )}
            </For>
          </div>
        </div>

        <aside class="kria-settings__history" aria-labelledby="settings-history-title">
          <div class="kria-settings__section-head">
            <div><h2 id="settings-history-title">Change history</h2><p>Verified config audit trail</p></div>
            <Button variant="ghost" size="sm" onClick={() => void settingsStore.load()}>Refresh</Button>
          </div>
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
      </div>
    </section>
  );
}
