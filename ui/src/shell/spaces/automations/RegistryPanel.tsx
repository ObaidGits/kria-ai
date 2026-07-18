/**
 * RegistryPanel — the advanced executable workflow registry, made REACHABLE in
 * the Automations Build segment (kria-ui-redesign task 7.5, Req 20.3).
 *
 * The legacy advanced registry (`N8nWorkflowManagementPanel view="advanced"`)
 * was defined but never mounted (inventory §II — registry / import / danger-
 * zone unreachable). Its value is revived here on the design kit + tokens:
 *   • reachable runtime-profile discovery, review, and KRIA registration
 *   • registered workflows with id/version/status/risk/environment
 *   • danger-zone removal with DELIBERATE confirm (Req 8.4-style)
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Sync / remove are DISPATCH-ONLY through existing n8n commands via `n8nStore`
 * (n8n is the substrate; KRIA orchestrates). No orchestration, no prompt→tool
 * shortcut. Untrusted registry text renders as escaped text.
 *
 * Requirements: 20.3, 8.4
 */
import { createMemo, createSignal, For, Show, onMount } from "solid-js";
import { Badge, Button, Card, Confirm, EmptyState, Input, Search, Select, Textarea } from "../../../kit";
import type { BadgeTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import {
  n8nStore,
  type N8nCredentialSummary,
  type N8nReviewedWorkflowMetadata,
  type N8nRuntimeProfileDraft,
  type N8nWorkflow,
} from "../../../stores/n8n";
import "./build-panels.css";

function riskTone(risk: string | undefined): BadgeTone {
  switch ((risk ?? "").toLowerCase()) {
    case "green":
      return "success";
    case "yellow":
      return "warning";
    case "red":
      return "danger";
    default:
      return "neutral";
  }
}

function externalCredentialRequirements(workflow: N8nWorkflow): string[] {
  return (workflow.credential_requirements ?? []).filter((requirement) => {
    const normalized = requirement.trim().toLowerCase();
    return normalized && normalized !== "none" && !normalized.startsWith("mapped:");
  });
}

function CredentialMapping(props: { workflow: N8nWorkflow }) {
  const requirements = createMemo(() => externalCredentialRequirements(props.workflow));
  const [credentials, setCredentials] = createSignal<N8nCredentialSummary[]>([]);
  const [selections, setSelections] = createSignal<Record<string, string>>({});
  const [message, setMessage] = createSignal("");
  const [loaded, setLoaded] = createSignal(false);
  const busy = createMemo(() => {
    const key = n8nStore.managementBusyKey();
    return key === "credentials:list" || key === `credentials:map:${props.workflow.workflow_id}`;
  });
  const options = createMemo(() => credentials().map((credential) => ({
    value: credential.credential_id,
    label: `${credential.credential_name} · ${credential.credential_type}`,
  })));

  async function loadCredentials() {
    setMessage("");
    try {
      setCredentials(await n8nStore.loadCredentialSummaries());
      setLoaded(true);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function saveMappings() {
    const mappings = requirements().map((requirement) => {
      const credential = credentials().find((item) => item.credential_id === selections()[requirement]);
      return credential
        ? {
            credentialType: requirement,
            credentialId: credential.credential_id,
            credentialName: credential.credential_name,
          }
        : null;
    });
    if (mappings.some((mapping) => mapping === null)) {
      setMessage("Choose one redacted n8n credential for every requirement.");
      return;
    }
    setMessage("");
    try {
      const result = await n8nStore.saveAuthoringCredentialMapping(
        props.workflow.workflow_id,
        mappings.filter((mapping): mapping is NonNullable<typeof mapping> => mapping !== null),
      );
      setMessage(result.message || "Credential references mapped. Run a backend test before approval.");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <details class="kria-registry__credentials">
      <summary>Credential mapping · {requirements().length} required</summary>
      <Show
        when={loaded()}
        fallback={
          <Button variant="secondary" size="sm" disabled={busy()} onClick={() => void loadCredentials()}>
            Load redacted credentials
          </Button>
        }
      >
        <Show
          when={credentials().length > 0}
          fallback={<p class="kria-registry__message">No n8n credential summaries are available.</p>}
        >
          <div class="kria-registry__credential-grid">
            <For each={requirements()}>
              {(requirement) => (
                <Select
                  label={requirement}
                  options={options()}
                  value={selections()[requirement]}
                  placeholder="Choose credential reference…"
                  onChange={(value) => setSelections((current) => ({ ...current, [requirement]: value ?? "" }))}
                />
              )}
            </For>
          </div>
          <Button size="sm" disabled={busy()} onClick={() => void saveMappings()}>
            Map credential references
          </Button>
        </Show>
      </Show>
      <Show when={message()}>
        <p class="kria-registry__message" role="status">{message()}</p>
      </Show>
    </details>
  );
}

function RegistryRow(props: { workflow: N8nWorkflow }) {
  const wf = () => props.workflow;
  const deleting = createMemo(() => n8nStore.isDeletingWorkflow(wf().workflow_id));

  return (
    <li>
      <Card class="kria-registry__row" aria-label={wf().display_name || wf().workflow_id}>
        <div class="kria-registry__main">
          <span class="kria-registry__name">{wf().display_name || wf().workflow_id}</span>
          <span class="kria-registry__meta">
            {wf().workflow_id} · v{wf().workflow_version}
          </span>
        </div>
        <div class="kria-registry__tags">
          <Badge tone="neutral">{wf().status}</Badge>
          <Badge tone={riskTone(wf().risk_tier)}>{wf().risk_tier || "unrated"}</Badge>
          <Show when={wf().environment}>
            <Badge tone="info">{wf().environment}</Badge>
          </Show>
        </div>
        <div class="kria-registry__actions">
          {/* Danger zone: deliberate confirm before removing (Req 8.4-style). */}
          <Confirm
            triggerLabel={deleting() ? "Removing…" : "Remove"}
            triggerIcon="trash-2"
            title={`Remove ${wf().display_name || wf().workflow_id}?`}
            message="This removes the workflow from KRIA's registry. It will no longer be runnable until re-imported."
            confirmLabel="Remove"
            cancelLabel="Keep"
            risk="danger"
            onConfirm={() => void n8nStore.deleteWorkflow(wf().workflow_id)}
          />
        </div>
        <Show
          when={
            wf().adaptation_strategy?.includes("chat_")
            && wf().generated_copy_n8n_verified
            && externalCredentialRequirements(wf()).length > 0
          }
        >
          <CredentialMapping workflow={wf()} />
        </Show>
      </Card>
    </li>
  );
}

function splitLabels(value: string): string[] {
  return Array.from(new Set(value.split(",").map((item) => item.trim()).filter(Boolean)));
}

function reviewedRisk(profile: N8nRuntimeProfileDraft): "Green" | "Yellow" | "Red" {
  switch (profile.risk_estimate.trim().toLowerCase()) {
    case "green":
      return "Green";
    case "red":
      return "Red";
    default:
      return "Yellow";
  }
}

function ProfileReviewCard(props: {
  profile: N8nRuntimeProfileDraft;
  saved: boolean;
  registered: boolean;
}) {
  const profile = () => props.profile;
  const defaultName = profile().display_name?.trim() || profile().n8n_workflow_name?.trim() || profile().workflow_id;
  const [displayName, setDisplayName] = createSignal(defaultName);
  const [description, setDescription] = createSignal(
    profile().enrichment_suggestion?.description?.trim()
      || `Imported n8n workflow "${defaultName}".`,
  );
  const [category, setCategory] = createSignal(
    profile().enrichment_suggestion?.category?.trim() || profile().category?.trim() || "automation",
  );
  const suggestedTags = profile().enrichment_suggestion?.tags ?? [];
  const suggestedDataScope = profile().enrichment_suggestion?.data_scope ?? [];
  const [tags, setTags] = createSignal(
    (suggestedTags.length
      ? suggestedTags
      : ["n8n", profile().trigger_strategy, profile().result_mode]).filter(Boolean).join(", "),
  );
  const [dataScope, setDataScope] = createSignal(
    (suggestedDataScope.length
      ? suggestedDataScope
      : profile().data_scope?.length
        ? profile().data_scope
        : ["user_requested"]).join(", "),
  );
  const [webhookMethod, setWebhookMethod] = createSignal(profile().webhook_method?.toUpperCase() || "");
  const [runnerBackend, setRunnerBackend] = createSignal(profile().runner_backend || "");
  const [runnerTarget, setRunnerTarget] = createSignal(profile().runner_target || "");
  const [runnerContainerName, setRunnerContainerName] = createSignal(profile().runner_container_name || "");
  const [brokerWorkflowId, setBrokerWorkflowId] = createSignal(profile().broker_workflow_id || "");
  const [brokerWebhookMethod, setBrokerWebhookMethod] = createSignal(profile().broker_webhook_method?.toUpperCase() || "POST");
  const [brokerWebhookPath, setBrokerWebhookPath] = createSignal(profile().broker_webhook_path || "");
  const usesManualRunner = createMemo(() => profile().trigger_strategy.toLowerCase().includes("manual"));
  const usesBroker = createMemo(() => profile().trigger_strategy.toLowerCase().includes("broker"));
  const usesDirectWebhook = createMemo(() => {
    const trigger = profile().trigger_strategy.toLowerCase();
    return trigger.includes("webhook") || trigger.includes("form") || trigger.includes("chat");
  });
  const [message, setMessage] = createSignal("");
  const busy = createMemo(() => {
    const key = n8nStore.managementBusyKey();
    return key === `profiles:save:${profile().profile_id}`
      || key === `profiles:refresh:${profile().profile_id}`
      || key === `profile:save_workflow:${profile().profile_id}`;
  });

  async function saveProfileAsDraft() {
    setMessage("");
    try {
      if (!props.saved) await n8nStore.saveRuntimeProfileDraft(profile());
      const request: N8nReviewedWorkflowMetadata = {
        profileId: profile().profile_id,
        webhookMethod: webhookMethod() || undefined,
        runnerBackend: runnerBackend() || undefined,
        runnerTarget: runnerTarget().trim() || undefined,
        runnerContainerName: runnerContainerName().trim() || undefined,
        brokerWorkflowId: brokerWorkflowId().trim() || undefined,
        brokerWebhookMethod: brokerWebhookMethod() || undefined,
        brokerWebhookPath: brokerWebhookPath().trim() || undefined,
        displayName: displayName().trim(),
        description: description().trim(),
        category: category().trim(),
        tags: splitLabels(tags()),
        aliases: [displayName().trim(), profile().workflow_id].filter(Boolean),
        examplePrompts: [`Run ${displayName().trim() || profile().workflow_id}`],
        dataScope: splitLabels(dataScope()),
        credentialRequirements: profile().credential_requirements?.length
          ? profile().credential_requirements
          : ["none"],
        hitlPolicy: profile().hitl_detected ? "required_review" : "none",
        riskTier: reviewedRisk(profile()),
      };
      if (!request.displayName || !request.description || !request.category || request.dataScope.length === 0) {
        setMessage("Name, description, category, and data scope are required.");
        return;
      }
      const result = await n8nStore.saveProfileAsWorkflowDraft(request);
      const blockers = Array.isArray(result?.blockers) ? result.blockers.map(String) : [];
      setMessage(
        result?.message
          || (blockers.length > 0
            ? `Saved for review: ${blockers.join("; ")}`
            : "Workflow setup saved in KRIA."),
      );
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function refreshProfile() {
    setMessage("");
    try {
      await n8nStore.refreshRuntimeProfileDraft(profile().profile_id);
      setMessage("Runtime analysis refreshed from n8n.");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <li>
      <Card class="kria-registry__profile" aria-label={`Workflow setup ${defaultName}`}>
        <div class="kria-registry__profile-head">
          <div class="kria-registry__main">
            <span class="kria-registry__name">{defaultName}</span>
            <span class="kria-registry__meta">
              {profile().trigger_strategy} · {profile().result_mode} · {profile().n8n_workflow_id}
            </span>
          </div>
          <div class="kria-registry__tags">
            <Badge tone={riskTone(profile().risk_estimate)}>{profile().risk_estimate || "needs review"}</Badge>
            <Badge tone={profile().credential_status === "present" ? "success" : "warning"}>
              credentials {profile().credential_status || "unknown"}
            </Badge>
            <Badge tone={props.registered ? "success" : props.saved ? "info" : "neutral"}>
              {props.registered ? "registered" : props.saved ? "analysis saved" : "discovered"}
            </Badge>
          </div>
        </div>

        <Show when={(profile().warnings?.length ?? 0) > 0 || (profile().lifecycle_warnings?.length ?? 0) > 0}>
          <ul class="kria-registry__warnings">
            <For each={[...(profile().warnings ?? []), ...(profile().lifecycle_warnings ?? [])]}>
              {(warning) => <li>{warning}</li>}
            </For>
          </ul>
        </Show>

        <details class="kria-registry__review">
          <summary>Review setup metadata</summary>
          <div class="kria-registry__review-grid">
            <Input label="Display name" value={displayName()} onChange={setDisplayName} />
            <Input label="Category" value={category()} onChange={setCategory} />
            <Textarea label="Description" value={description()} rows={2} onChange={setDescription} />
            <Input label="Tags (comma separated)" value={tags()} onChange={setTags} />
            <Input label="Data scope (comma separated)" value={dataScope()} onChange={setDataScope} />
          </div>
          <Show when={usesDirectWebhook() || usesManualRunner() || usesBroker()}>
            <fieldset class="kria-registry__runtime-fields">
              <legend>Runtime dispatch</legend>
              <Show when={usesDirectWebhook()}>
                <Select
                  label="Webhook method"
                  options={[{ value: "GET", label: "GET" }, { value: "POST", label: "POST" }]}
                  value={webhookMethod() || undefined}
                  placeholder="Detected by KRIA"
                  onChange={(value) => setWebhookMethod(value ?? "")}
                />
              </Show>
              <Show when={usesManualRunner()}>
                <Select
                  label="Runner backend"
                  options={[
                    { value: "local_cli", label: "Local CLI" },
                    { value: "managed_docker", label: "Managed Docker" },
                    { value: "remote_ssh", label: "Remote SSH" },
                    { value: "remote_docker", label: "Remote Docker" },
                  ]}
                  value={runnerBackend() || undefined}
                  placeholder="Use KRIA default"
                  onChange={(value) => setRunnerBackend(value ?? "")}
                />
                <Input label="Runner target" value={runnerTarget()} onChange={setRunnerTarget} />
                <Input label="Runner container" value={runnerContainerName()} onChange={setRunnerContainerName} />
              </Show>
              <Show when={usesBroker()}>
                <Input label="Broker workflow ID" value={brokerWorkflowId()} onChange={setBrokerWorkflowId} />
                <Select
                  label="Broker webhook method"
                  options={[{ value: "GET", label: "GET" }, { value: "POST", label: "POST" }]}
                  value={brokerWebhookMethod()}
                  onChange={(value) => setBrokerWebhookMethod(value ?? "POST")}
                />
                <Input label="Broker webhook path" value={brokerWebhookPath()} onChange={setBrokerWebhookPath} />
              </Show>
            </fieldset>
          </Show>
          <div class="kria-registry__actions">
            <Button variant="secondary" size="sm" disabled={busy()} onClick={() => void refreshProfile()}>
              <Icon name="refresh-cw" size={14} /> Refresh analysis
            </Button>
            <Button size="sm" disabled={busy()} onClick={() => void saveProfileAsDraft()}>
              <Icon name={busy() ? "loader" : "save"} size={14} />
              {props.registered ? "Update KRIA setup" : "Save to KRIA"}
            </Button>
          </div>
        </details>
        <Show when={message()}>
          <p class="kria-registry__message" role="status">{message()}</p>
        </Show>
      </Card>
    </li>
  );
}

export function RegistryPanel() {
  const [query, setQuery] = createSignal("");

  onMount(() => void n8nStore.initialize().catch(() => undefined));

  const workflows = createMemo(() => {
    const q = query().trim().toLowerCase();
    const all = n8nStore.configuredWorkflows();
    if (!q) return all;
    return all.filter(
      (w) =>
        (w.display_name ?? "").toLowerCase().includes(q) ||
        w.workflow_id.toLowerCase().includes(q),
    );
  });

  const profiles = createMemo(() => {
    const byId = new Map<string, N8nRuntimeProfileDraft>();
    for (const profile of n8nStore.savedRuntimeProfiles()) byId.set(profile.profile_id, profile);
    for (const profile of n8nStore.runtimeProfileDrafts()) byId.set(profile.profile_id, profile);
    return [...byId.values()].sort((a, b) =>
      (a.display_name || a.n8n_workflow_name || a.workflow_id)
        .localeCompare(b.display_name || b.n8n_workflow_name || b.workflow_id),
    );
  });
  const savedProfileIds = createMemo(() =>
    new Set(n8nStore.savedRuntimeProfiles().map((profile) => profile.profile_id)),
  );
  const registeredWorkflowIds = createMemo(() => {
    const ids = new Set<string>();
    for (const workflow of n8nStore.configuredWorkflows()) {
      ids.add(workflow.workflow_id);
      if (workflow.n8n_workflow_id) ids.add(workflow.n8n_workflow_id);
    }
    return ids;
  });
  const syncing = createMemo(() => n8nStore.managementBusyKey() === "profiles:sync");

  return (
    <section class="kria-registry" aria-label="Executable workflow registry">
      <div class="kria-registry__head">
        <h2 class="kria-automations__region-title">Executable Workflow Registry</h2>
        <Button
          variant="secondary"
          size="sm"
          disabled={syncing()}
          aria-label="Sync workflows from n8n"
          onClick={() => void n8nStore.syncRuntimeProfileDrafts().catch(() => undefined)}
        >
          <Icon name={syncing() ? "loader" : "refresh-cw"} size={14} />
          {syncing() ? "Syncing…" : "Sync from n8n"}
        </Button>
      </div>

      <Show when={n8nStore.managementError()}>
        <p class="kria-registry__error" role="alert">
          <Icon name="alert-triangle" size={13} aria-hidden /> {n8nStore.managementError()}
        </p>
      </Show>

      <Show when={profiles().length > 0}>
        <div class="kria-registry__profiles">
          <div class="kria-registry__section-head">
            <div>
              <h3>Discovered workflow setup</h3>
              <p>Review runtime analysis, then register safe metadata in KRIA.</p>
            </div>
            <Badge tone="info">{profiles().length}</Badge>
          </div>
          <ul class="kria-registry__list">
            <For each={profiles()}>
              {(profile) => (
                <ProfileReviewCard
                  profile={profile}
                  saved={savedProfileIds().has(profile.profile_id)}
                  registered={
                    registeredWorkflowIds().has(profile.workflow_id)
                    || registeredWorkflowIds().has(profile.n8n_workflow_id)
                  }
                />
              )}
            </For>
          </ul>
        </div>
      </Show>

      <div class="kria-registry__search">
        <Search
          label="Search the registry"
          placeholder="Search workflows…"
          value={query()}
          onChange={setQuery}
        />
      </div>

      <Show
        when={workflows().length > 0}
        fallback={
          <EmptyState
            icon="workflow"
            title="No registered workflows"
            description="Sync from n8n to discover workflows, then import them to make them runnable."
          />
        }
      >
        <p class="kria-registry__count">{workflows().length} registered</p>
        <ul class="kria-registry__list">
          <For each={workflows()}>{(wf) => <RegistryRow workflow={wf} />}</For>
        </ul>
      </Show>
    </section>
  );
}

export default RegistryPanel;
