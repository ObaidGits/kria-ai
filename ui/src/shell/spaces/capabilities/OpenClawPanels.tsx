import { createEffect, createSignal, Show } from "solid-js";
import { Badge, Button, Card, Input } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { capabilityStore, type OpenClawSettings } from "../../../stores";
import { updateOpenClawSettings } from "../../../bridge/capabilityActions";
import { closeModal, openModal } from "../../modalHost";

type NumberKey =
  | "warmPerClass"
  | "maxConcurrentInvocations"
  | "defaultTimeoutSecs"
  | "maxWarmAgeSecs"
  | "maxRestartAttempts";

function Toggle(props: {
  id: string;
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label class="kria-capsettings__toggle" for={props.id}>
      <span>{props.label}</span>
      <input
        id={props.id}
        class="kria-capsettings__checkbox kit-focusable"
        type="checkbox"
        checked={props.checked}
        disabled={props.disabled}
        onChange={(event) => props.onChange(event.currentTarget.checked)}
      />
    </label>
  );
}

export function OpenClawRuntimePanel() {
  const [draft, setDraft] = createSignal<OpenClawSettings | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [message, setMessage] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  createEffect(() => {
    const source = capabilityStore.openClawSettings();
    if (source) setDraft({ ...source });
  });
  function patch<K extends keyof OpenClawSettings>(key: K, value: OpenClawSettings[K]) {
    setDraft((current) => current ? { ...current, [key]: value } : current);
  }

  function patchNumber(key: NumberKey, value: string) {
    const number = Number(value);
    if (Number.isFinite(number)) patch(key, number);
  }

  async function save() {
    const settings = draft();
    if (!settings || saving()) return;
    setSaving(true);
    setError(null);
    setMessage(null);
    try {
      const result = await updateOpenClawSettings(settings);
      if (!result.ok) {
        setError(result.message);
        return;
      }
      setMessage(result.data.restartRequired
        ? "Saved. Restart KRIA to apply substrate enable/image changes."
        : "OpenClaw settings saved and active policy refreshed.");
    } finally {
      setSaving(false);
    }
  }

  return (
    <section class="kria-capsettings" aria-label="OpenClaw runtime settings">
      <div class="kria-governance__section-head">
        <div>
          <h3 class="kria-descriptor__section-title">OpenClaw runtime</h3>
          <p class="kria-capcard__desc">Sandbox pool, skill lifecycle, and registry source.</p>
        </div>
        <Show when={draft()}>
          {(settings) => (
            <span class="kria-capcard__meta">
              <Badge tone={settings().runtimeActive ? "success" : settings().enabled ? "warning" : "neutral"}>
                {settings().runtimeActive ? "Runtime running" : settings().enabled ? "Restart to start" : "Disabled"}
              </Badge>
            </span>
          )}
        </Show>
      </div>

      <Show when={error()}>{(value) => <p class="kria-capsettings__error" role="alert">{value()}</p>}</Show>
      <Show when={message()}>{(value) => <p class="kria-capsettings__success" role="status">{value()}</p>}</Show>

      <Show when={draft()} fallback={<p class="kria-capabilities__status">OpenClaw settings unavailable.</p>}>
        {(settings) => (
          <Card class="kria-capsettings__card">
            <div class="kria-capsettings__grid">
              <Toggle id="openclaw-enabled" label="Enable OpenClaw substrate" checked={settings().enabled} onChange={(value) => patch("enabled", value)} />
              <Toggle id="openclaw-rewrite" label="Rewrite skill descriptions with local LLM" checked={settings().rewriteDescriptions} onChange={(value) => patch("rewriteDescriptions", value)} />
              <Toggle id="openclaw-updates" label="Check registry for skill updates" checked={settings().checkUpdates} onChange={(value) => patch("checkUpdates", value)} />
              <Input label="Container image" value={settings().image} onChange={(value) => patch("image", value)} />
              <Input label="Registry index URL" value={settings().registryIndexUrl} onChange={(value) => patch("registryIndexUrl", value)} />
              <Input label="Warm containers per class" type="number" value={String(settings().warmPerClass)} inputProps={{ min: 0, max: 16 }} onChange={(value) => patchNumber("warmPerClass", value)} />
              <Input label="Max concurrent invocations" type="number" value={String(settings().maxConcurrentInvocations)} inputProps={{ min: 1, max: 64 }} onChange={(value) => patchNumber("maxConcurrentInvocations", value)} />
              <Input label="Invocation timeout (seconds)" type="number" value={String(settings().defaultTimeoutSecs)} inputProps={{ min: 1, max: 3600 }} onChange={(value) => patchNumber("defaultTimeoutSecs", value)} />
              <Input label="Idle recycle age (seconds)" type="number" value={String(settings().maxWarmAgeSecs)} inputProps={{ min: 30 }} onChange={(value) => patchNumber("maxWarmAgeSecs", value)} />
              <Input label="Boot retry attempts" type="number" value={String(settings().maxRestartAttempts)} inputProps={{ min: 1, max: 10 }} onChange={(value) => patchNumber("maxRestartAttempts", value)} />
            </div>
            <div class="kria-capcard__actions">
              <Button variant="primary" size="sm" disabled={saving()} onClick={() => void save()}>
                <Icon name="save" size={14} aria-hidden /> {saving() ? "Saving…" : "Save OpenClaw settings"}
              </Button>
              <Show when={settings().enabled !== capabilityStore.openClawSettings()?.enabled || settings().image !== capabilityStore.openClawSettings()?.image}>
                <span class="kria-caprow__desc">Enable/image changes require KRIA restart.</span>
              </Show>
            </div>
          </Card>
        )}
      </Show>
    </section>
  );
}
export function OpenClawTrustPanel() {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  function confirmPolicy(
    key: "communityAllowsNetwork" | "verifiedSkipsHitl",
    next: boolean,
  ) {
    const current = capabilityStore.openClawSettings();
    if (!current || busy()) return;
    const isNetwork = key === "communityAllowsNetwork";
    const modalId = `openclaw-policy-${key}-${next}`;
    openModal({
      id: modalId,
      title: isNetwork
        ? `${next ? "Allow" : "Block"} community skill network access?`
        : `${next ? "Allow" : "Stop"} verified skills skipping approval?`,
      description: isNetwork
        ? "This changes whether community-authored skills may reach the network. Runtime trust policy remains authoritative."
        : "This changes whether verified skills may execute without human-in-the-loop approval. Other runtime policy checks still apply.",
      render: () => (
        <div class="kria-governance__confirm" role="note">
          <Icon name="shield-alert" size={16} aria-hidden />
          <span>{next ? "This relaxes OpenClaw trust policy." : "This tightens OpenClaw trust policy."}</span>
        </div>
      ),
      footer: (
        <>
          <Button variant="ghost" onClick={() => closeModal(modalId)}>Cancel</Button>
          <Button
            variant={next ? "danger" : "primary"}
            onClick={() => {
              closeModal(modalId);
              setBusy(true);
              setError(null);
              void updateOpenClawSettings({ ...current, [key]: next })
                .then((result) => {
                  if (!result.ok) setError(result.message);
                })
                .finally(() => setBusy(false));
            }}
          >
            Confirm policy change
          </Button>
        </>
      ),
    });
  }

  return (
    <section class="kria-governance__section" aria-label="OpenClaw trust policy">
      <h3 class="kria-descriptor__section-title">OpenClaw trust policy</h3>
      <p class="kria-caprow__desc">Security-sensitive trust controls require explicit confirmation.</p>
      <Show when={error()}>{(value) => <p class="kria-capsettings__error" role="alert">{value()}</p>}</Show>
      <Show when={capabilityStore.openClawSettings()} fallback={<p class="kria-capabilities__status">OpenClaw trust policy unavailable.</p>}>
        {(settings) => (
          <div class="kria-capsettings__policy-grid">
            <Card class="kria-capsettings__policy">
              <div class="kria-capcard__head">
                <span class="kria-capcard__name">Community network</span>
                <Badge tone={settings().communityAllowsNetwork ? "warning" : "success"}>
                  {settings().communityAllowsNetwork ? "Allowed" : "Blocked"}
                </Badge>
              </div>
              <p class="kria-capcard__desc">Controls network access for community-tier skills.</p>
              <Button size="sm" variant={settings().communityAllowsNetwork ? "secondary" : "danger"} disabled={busy()} onClick={() => confirmPolicy("communityAllowsNetwork", !settings().communityAllowsNetwork)}>
                {settings().communityAllowsNetwork ? "Block network" : "Review allow"}
              </Button>
            </Card>
            <Card class="kria-capsettings__policy">
              <div class="kria-capcard__head">
                <span class="kria-capcard__name">Verified skill HITL</span>
                <Badge tone={settings().verifiedSkipsHitl ? "warning" : "success"}>
                  {settings().verifiedSkipsHitl ? "May skip" : "Required"}
                </Badge>
              </div>
              <p class="kria-capcard__desc">Controls human approval bypass for verified-tier skills.</p>
              <Button size="sm" variant={settings().verifiedSkipsHitl ? "secondary" : "danger"} disabled={busy()} onClick={() => confirmPolicy("verifiedSkipsHitl", !settings().verifiedSkipsHitl)}>
                {settings().verifiedSkipsHitl ? "Require approval" : "Review skip"}
              </Button>
            </Card>
          </div>
        )}
      </Show>
    </section>
  );
}
