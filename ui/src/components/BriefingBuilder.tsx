import { Component, createSignal, For, onMount, Show } from "solid-js";
import { appStore, type BriefingConfig, type BriefingSection } from "../stores/app";

/**
 * BriefingBuilder — Settings panel to customise the morning briefing.
 * Reads/writes BriefingConfig via the app store (Tauri get/set_briefing_config).
 */
const BriefingBuilder: Component = () => {
  const [local, setLocal] = createSignal<BriefingConfig | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [message, setMessage] = createSignal("");

  onMount(async () => {
    const cfg = await appStore.loadBriefingConfig();
    setLocal(cfg ?? { sections: [], schedule: { auto: false, time: "08:00", delivery: ["notification"] } });
  });

  const clone = (): BriefingConfig => JSON.parse(JSON.stringify(local()));

  const updateSection = (idx: number, patch: Partial<BriefingSection>) => {
    const next = clone();
    next.sections[idx] = { ...next.sections[idx], ...patch };
    setLocal(next);
  };

  const toggleDelivery = (channel: string) => {
    const next = clone();
    const set = new Set(next.schedule.delivery);
    if (set.has(channel)) set.delete(channel);
    else set.add(channel);
    next.schedule.delivery = Array.from(set);
    setLocal(next);
  };

  const save = async () => {
    const cfg = local();
    if (!cfg) return;
    setSaving(true);
    setMessage("");
    try {
      await appStore.saveBriefingConfig(cfg);
      setMessage("Briefing saved.");
    } catch (e) {
      setMessage(`Save failed: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <section class="settings-section">
      <h3>Briefing Builder</h3>
      <p class="field-hint">
        Choose exactly what your morning briefing includes. The agent renders these sections
        when you ask for a briefing (tool <code>gw_morning_briefing</code>).
      </p>

      <Show when={local()} fallback={<p class="field-hint">Loading…</p>}>
        {(cfg) => (
          <>
            <For each={cfg().sections}>
              {(section, idx) => (
                <div
                  class="settings-field"
                  style="border:1px solid var(--border,#333);border-radius:8px;padding:0.75rem;margin-top:0.6rem"
                >
                  <label style="display:flex;align-items:center;gap:0.5rem;text-transform:capitalize">
                    <input
                      type="checkbox"
                      checked={section.enabled}
                      onChange={(e) => updateSection(idx(), { enabled: e.currentTarget.checked })}
                    />
                    <strong>{section.source}</strong>
                  </label>

                  <Show when={section.source === "gmail"}>
                    <div style="display:flex;gap:0.5rem;flex-wrap:wrap;margin-top:0.5rem">
                      <input
                        type="text"
                        placeholder="Gmail query (e.g. is:unread, subject:urgent)"
                        value={section.query ?? ""}
                        onInput={(e) => updateSection(idx(), { query: e.currentTarget.value })}
                        style="flex:1;min-width:220px"
                      />
                      <input
                        type="number"
                        min="1"
                        placeholder="max"
                        value={section.max ?? 10}
                        onInput={(e) =>
                          updateSection(idx(), { max: Number(e.currentTarget.value) || 10 })
                        }
                        style="width:80px"
                      />
                    </div>
                    <span class="field-hint">Uses Gmail search syntax (from:, subject:, is:unread…).</span>
                  </Show>

                  <Show when={section.source === "calendar"}>
                    <div style="display:flex;gap:0.75rem;align-items:center;margin-top:0.5rem">
                      <select
                        value={section.window ?? "today"}
                        onChange={(e) => updateSection(idx(), { window: e.currentTarget.value })}
                      >
                        <option value="today">Today</option>
                        <option value="next24h">Next 24h</option>
                      </select>
                      <label style="display:flex;align-items:center;gap:0.4rem">
                        <input
                          type="checkbox"
                          checked={section.include_conflicts ?? true}
                          onChange={(e) =>
                            updateSection(idx(), { include_conflicts: e.currentTarget.checked })
                          }
                        />
                        Detect conflicts
                      </label>
                    </div>
                  </Show>

                  <Show when={section.source === "github"}>
                    <input
                      type="text"
                      placeholder="GitHub MCP tool (default list_notifications)"
                      value={section.tool ?? "list_notifications"}
                      onInput={(e) => updateSection(idx(), { tool: e.currentTarget.value })}
                      style="margin-top:0.5rem;width:100%"
                    />
                  </Show>

                  <Show when={section.source === "tasks"}>
                    <select
                      value={section.filter ?? "urgent_and_overdue"}
                      onChange={(e) => updateSection(idx(), { filter: e.currentTarget.value })}
                      style="margin-top:0.5rem"
                    >
                      <option value="urgent_and_overdue">Urgent &amp; overdue</option>
                      <option value="active">All active</option>
                      <option value="all">Everything</option>
                    </select>
                  </Show>
                </div>
              )}
            </For>

            <div
              class="settings-field"
              style="border:1px solid var(--border,#333);border-radius:8px;padding:0.75rem;margin-top:0.8rem"
            >
              <label style="display:flex;align-items:center;gap:0.5rem">
                <input
                  type="checkbox"
                  checked={cfg().schedule.auto}
                  onChange={(e) => {
                    const next = clone();
                    next.schedule.auto = e.currentTarget.checked;
                    setLocal(next);
                  }}
                />
                <strong>Auto-deliver daily</strong>
              </label>
              <div style="display:flex;gap:0.75rem;align-items:center;margin-top:0.5rem">
                <label>Time</label>
                <input
                  type="time"
                  value={cfg().schedule.time}
                  onInput={(e) => {
                    const next = clone();
                    next.schedule.time = e.currentTarget.value;
                    setLocal(next);
                  }}
                />
                <For each={["notification", "chat", "tts"]}>
                  {(ch) => (
                    <label style="display:flex;align-items:center;gap:0.3rem">
                      <input
                        type="checkbox"
                        checked={cfg().schedule.delivery.includes(ch)}
                        onChange={() => toggleDelivery(ch)}
                      />
                      {ch}
                    </label>
                  )}
                </For>
              </div>
              <span class="field-hint">
                Auto-delivery scheduling is honoured by the background scheduler (Phase 2).
              </span>
            </div>

            <div style="display:flex;align-items:center;gap:0.75rem;margin-top:0.9rem">
              <button class="btn-primary" disabled={saving()} onClick={save}>
                {saving() ? "Saving…" : "Save briefing"}
              </button>
              <Show when={message()}>
                <span class="field-hint">{message()}</span>
              </Show>
            </div>
          </>
        )}
      </Show>
    </section>
  );
};

export default BriefingBuilder;
