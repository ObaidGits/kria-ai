/**
 * Capabilities Space — segments + descriptor Inspector (task 8.1, Req 7.1/7.2).
 *
 * Provides the six Capabilities segments as a real tablist (kit `Tabs`,
 * Kobalte-backed → correct tablist/tab/tabpanel roles + arrow-key nav,
 * Req 17.1/17.2), driven by the typed router `space=capabilities,
 * segment=<id>` (Req 1.3/1.5):
 *   • Tools ......... native/federated capability catalog (CapabilityRow →
 *                    descriptor Inspector, Req 7.2).
 *   • Skills ........ ClawHub/OpenClaw skills (SkillCard, trust surfaced).
 *   • Models ........ LLM providers + their models (ProviderCard).
 *   • Integrations .. MCP servers + Google/Colab/Telegram (IntegrationCard).
 *   • Governance .... permission grants + actionable evolution proposals.
 *   • Generate ...... generation-capability legibility (GeneratePanel).
 *   • Constellation .. on-demand 3D lens + mandatory 2D catalog fallback
 *                    (ConstellationLens, task 8.3, Req 7.5 / 16.3 / 17.5).
 *
 * Selecting a Tools capability opens its descriptor (descriptor / effects /
 * trust tier / schema, Req 7.2) in the ONE shared Inspector via the
 * `type: "capability"` renderer registered on mount (Req 1.6).
 *
 * ── ARCHITECTURE INVARIANT (KRIA is the authoritative orchestrator) ──────────
 * Pure presentation / read-model: reads `capabilityStore` only, which dispatches
 * to EXISTING backend commands. Tools/skills/models/integrations are execution
 * substrates surfaced here for LEGIBILITY ONLY — this Space wires NO
 * prompt→tool execution shortcut and NO substrate self-authority. The
 * run→permission-gate flow is task 8.2. All capability text is UNTRUSTED and
 * rendered as escaped text (Solid), never as HTML.
 *
 * Requirements: 7.1, 7.2
 */
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import {
  capabilityStore,
  shellStore,
  type CapabilitySegment,
  type IntegrationView,
  type RemoteSkillView,
} from "../../stores";
import { currentRoute, navigate } from "../router";
import { Badge, Button, EmptyState, Search, Tabs } from "../../kit";
import { Icon } from "../../components/Icon";
import {
  CapabilityRow,
  SkillCard,
  IntegrationCard,
  GeneratePanel,
  GovernancePanel,
  ModelsRuntimePanel,
  OpenClawRuntimePanel,
  TrustReviewDialog,
  IntegrationConnectDialog,
  registerDescriptorInspector,
} from "./capabilities";
import ConstellationLens from "./capabilities/constellation/ConstellationLens";
import {
  installSkill,
  connectGoogleWorkspace,
  connectColabTier,
  approveQuarantinedTool,
  rejectQuarantinedTool,
  revokeGrant,
  revokeCppGrant,
  applyEvolutionProposal,
  undoEvolutionProposal,
  setCapabilityAutonomy,
  scanCapabilityDiscovery,
  releaseProviderQuarantine,
} from "../../bridge/capabilityActions";
import "./capabilities/capabilities.css";

// ─── Segment model ───────────────────────────────────────────────────────────

interface SegmentDef {
  value: CapabilitySegment;
  label: string;
}

/**
 * The six Capabilities segments (Req 7.1) + the Constellation lens (Req 7.5,
 * task 8.3). Tools is first → the default; Constellation is last (a lens, not a
 * data segment).
 */
const SEGMENTS: readonly SegmentDef[] = [
  { value: "tools", label: "Tools" },
  { value: "skills", label: "Skills" },
  { value: "models", label: "Models" },
  { value: "integrations", label: "Integrations" },
  { value: "governance", label: "Governance" },
  { value: "generate", label: "Generate" },
  { value: "constellation", label: "Constellation" },
] as const;

function isCapabilitySegment(value: string | undefined): value is CapabilitySegment {
  return !!value && SEGMENTS.some((s) => s.value === value);
}

/** Resolve the routed segment, defaulting to Tools (Req 1.5 / 7.1). */
function routedSegment(): CapabilitySegment {
  const seg = currentRoute().segment;
  return isCapabilitySegment(seg) ? seg : "tools";
}

// ─── Space ─────────────────────────────────────────────────────────────────────

export default function CapabilitiesSpace() {
  const isMini = createMemo(() => shellStore.windowMode() === "mini");

  // Register the descriptor Inspector body (type "capability") so selecting a
  // CapabilityRow opens it in the ONE shared Inspector (Req 1.6 / 7.2). The
  // disposer unregisters on unmount / hot-reload.
  onCleanup(registerDescriptorInspector());

  // Mirror the routed segment into the store and load that segment's data
  // (honest loading state; graceful when a service is absent, Req 20.4).
  createEffect(() => {
    const seg = routedSegment();
    capabilityStore.setActiveSegment(seg);
    void capabilityStore.loadSegment(seg);
  });

  // Consume capability/provider entity routes once their authoritative segment
  // data arrives. Capabilities open the shared descriptor Inspector; provider
  // routes reveal/focus the provider card without inventing a provider action.
  let handledEntityRoute: string | null = null;
  createEffect(() => {
    const route = currentRoute();
    const capabilities = capabilityStore.capabilities();
    const providers = capabilityStore.providers();
    if (route.space !== "capabilities" || !route.entityId) return;
    const routeKey = `${route.space}/${route.segment ?? "tools"}/${route.entityId}`;
    if (handledEntityRoute === routeKey) return;

    if (route.segment === "models") {
      const provider = providers.find((item) => item.id === route.entityId);
      if (!provider) return;
      queueMicrotask(() => {
        if (currentRoute().entityId !== provider.id) return;
        const card = Array.from(
          document.querySelectorAll<HTMLElement>("[data-provider-id]"),
        ).find((element) => element.dataset.providerId === provider.id);
        if (!card) return;
        card.scrollIntoView?.({ block: "center" });
        card.focus({ preventScroll: true });
        handledEntityRoute = routeKey;
      });
      return;
    }

    const capability = capabilities.find((item) => item.id === route.entityId);
    if (!capability) return;
    // Programmatic (route/deep-link) open: hand the stable Capabilities region
    // as the Focus_Return_Owner (§20.3/§20.4).
    shellStore.openInspector(
      "capability",
      capability.id,
      {
        providerId: capability.providerId ?? "",
        capabilityId: capability.capabilityId ?? "",
        name: capability.name,
      },
      { regionSelector: '[data-space="capabilities"]' },
    );
    handledEntityRoute = routeKey;
  });

  function selectSegment(value: string) {
    if (value === "tools") navigate("capabilities");
    else navigate("capabilities", value);
  }

  const items = SEGMENTS.map((seg) => ({
    value: seg.value,
    label: seg.label,
    content: () => <SegmentRegion segment={seg.value} label={seg.label} />,
  }));

  return (
    <section class="kria-capabilities" data-space="capabilities" aria-label="Capabilities">
      <header class="kria-capabilities__header">
        <h1 class="kria-capabilities__title">Capabilities</h1>
        <p class="kria-capabilities__subtitle">
          What KRIA can do — and how each ability is granted, trusted, and evolved.
        </p>
      </header>

      <Show
        when={isMini()}
        fallback={
          <Tabs
            class="kria-capabilities__segments"
            items={items}
            value={routedSegment()}
            onChange={selectSegment}
          />
        }
      >
        <div class="kria-capabilities__compact" data-curated-primary="capability-lookup">
          <ToolsRegion />
        </div>
      </Show>
    </section>
  );
}

// ─── Regions ─────────────────────────────────────────────────────────────────

function SegmentRegion(props: { segment: CapabilitySegment; label: string }) {
  return (
    <div
      class="kria-capabilities__region"
      data-segment={props.segment}
      aria-label={props.label}
    >
      <Show when={props.segment === "tools"}>
        <ToolsRegion />
      </Show>
      <Show when={props.segment === "skills"}>
        <SkillsRegion />
      </Show>
      <Show when={props.segment === "models"}>
        <ModelsRegion />
      </Show>
      <Show when={props.segment === "integrations"}>
        <IntegrationsRegion />
      </Show>
      <Show when={props.segment === "governance"}>
        <GovernancePanel
          grants={capabilityStore.grants()}
          proposals={capabilityStore.proposals()}
          capabilityHealth={capabilityStore.capabilityHealth()}
          capabilityAutonomy={capabilityStore.capabilityAutonomy()}
          providerQuarantine={capabilityStore.providerQuarantine()}
          discoveryStatus={capabilityStore.discoveryStatus()}
          capabilityTimeline={capabilityStore.capabilityTimeline()}
          quarantinedTools={capabilityStore.quarantinedTools()}
          scopedGrants={capabilityStore.scopedGrants()}
          scopedGrantsStatus={capabilityStore.scopedGrantsStatus()}
          activityLog={capabilityStore.activityLog()}
          activityNote={capabilityStore.activityNote()}
          loading={capabilityStore.loading()}
          onApproveQuarantine={async (id) => {
            await approveQuarantinedTool(id);
          }}
          onRejectQuarantine={async (id) => {
            await rejectQuarantinedTool(id);
          }}
          onRevokeCppGrant={async (id) => {
            await revokeCppGrant(id);
          }}
          onRevokeGrant={async (id) => {
            await revokeGrant(id);
          }}
          onApplyProposal={async (id) => {
            await applyEvolutionProposal(id);
          }}
          onUndoProposal={async (id) => {
            await undoEvolutionProposal(id);
          }}
          onSetAutonomy={async (level) => {
            await setCapabilityAutonomy(level);
          }}
          onScanDiscovery={async () => {
            await scanCapabilityDiscovery();
          }}
          onReleaseProviderQuarantine={async (providerId, capabilityId) => {
            await releaseProviderQuarantine(providerId, capabilityId);
          }}
          onReloadQuarantine={async () => {
            await capabilityStore.loadQuarantine();
          }}
        />
      </Show>
      <Show when={props.segment === "generate"}>
        <GeneratePanel status={capabilityStore.generateStatus()} loading={capabilityStore.loading()} />
      </Show>
      <Show when={props.segment === "constellation"}>
        <ConstellationLens />
      </Show>
    </div>
  );
}

/** Loading placeholder shared by data-backed regions (honest states, Req 7.1). */
function LoadingRow(props: { label: string }) {
  return (
    <div class="kria-capabilities__status" role="status" aria-live="polite">
      {props.label}
    </div>
  );
}

// Tools search query (local to the Space).
const [toolsQuery, setToolsQuery] = createSignal("");
const [toolsResultMode, setToolsResultMode] = createSignal<"catalog" | "discover" | "recommend">("catalog");
const [toolsError, setToolsError] = createSignal<string | null>(null);

/** Tools — the native/federated capability catalog; each row → descriptor. */
function ToolsRegion() {
  const filtered = createMemo(() => {
    const query = toolsQuery().trim().toLowerCase();
    const list = capabilityStore.capabilities();
    if (!query || toolsResultMode() !== "catalog") return list;
    return list.filter(
      (c) =>
        c.name.toLowerCase().includes(query) ||
        c.description.toLowerCase().includes(query) ||
        (c.tags ?? []).some((t) => t.toLowerCase().includes(query)),
    );
  });

  async function queryRuntime(mode: "discover" | "recommend") {
    const query = toolsQuery().trim();
    if (!query) return;
    setToolsError(null);
    const outcome = mode === "discover"
      ? await capabilityStore.discoverTools(query)
      : await capabilityStore.recommendTools(query);
    if (outcome.ok) setToolsResultMode(mode);
    else setToolsError(outcome.message);
  }

  function updateToolsQuery(value: string) {
    if (toolsResultMode() !== "catalog") {
      setToolsResultMode("catalog");
      void capabilityStore.loadTools();
    }
    setToolsQuery(value);
    setToolsError(null);
  }

  return (
    <>
      <h2 class="kria-capabilities__region-title">Tools</h2>
      <Show when={capabilityStore.capabilityPlatformStatus() || capabilityStore.capabilityProviders().length > 0}>
        <section class="kria-capabilities__provider-summary" aria-label="Capability provider runtime">
          <div class="kria-capcard__head">
            <h3 class="kria-descriptor__section-title">Capability providers</h3>
            <Show when={capabilityStore.capabilityPlatformStatus()}>
              {(status) => (
                <span class="kria-capcard__meta">
                  <Badge tone={status().enabled ? "success" : "neutral"}>
                    {status().enabled ? "CPP enabled" : "CPP disabled"}
                  </Badge>
                  <Badge tone="neutral">{status().healthyProviders}/{status().providerCount} healthy</Badge>
                  <Badge tone="neutral">{status().descriptorCount} descriptors</Badge>
                </span>
              )}
            </Show>
          </div>
          <Show when={capabilityStore.capabilityProviders().length > 0}>
            <ul class="kria-capabilities__grid" aria-label="Capability providers">
              <For each={capabilityStore.capabilityProviders()}>
                {(provider) => (
                  <li>
                    <div class="kria-capcard" role="group" aria-label={provider.providerId}>
                      <div class="kria-capcard__head">
                        <span class="kria-capcard__name">{provider.providerId}</span>
                        <Badge tone={provider.health === "healthy" ? "success" : provider.health === "unhealthy" ? "danger" : "warning"}>
                          {provider.health}
                        </Badge>
                      </div>
                      <p class="kria-capcard__desc">
                        {provider.state} · {provider.descriptorCount} descriptors
                        <Show when={provider.version}> · version {provider.version}</Show>
                      </p>
                      <Show when={provider.error}>
                        {(message) => <p class="kria-capabilities__status" role="alert">{message()}</p>}
                      </Show>
                    </div>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>
      </Show>
      <div class="kria-capabilities__browse">
        <Search
          class="kria-capabilities__search"
          label="Search tools"
          placeholder="Search catalog or describe a goal"
          value={toolsQuery()}
          onChange={updateToolsQuery}
        />
        <Button
          variant="secondary"
          size="sm"
          disabled={!toolsQuery().trim() || capabilityStore.loading()}
          onClick={() => void queryRuntime("discover")}
        >
          Semantic search
        </Button>
        <Button
          variant="secondary"
          size="sm"
          disabled={!toolsQuery().trim() || capabilityStore.loading()}
          onClick={() => void queryRuntime("recommend")}
        >
          Recommend for goal
        </Button>
      </div>
      <Show when={toolsResultMode() !== "catalog"}>
        <p class="kria-caprow__desc" role="status">
          Runtime-ranked {toolsResultMode() === "discover" ? "discovery" : "recommendations"} for “{toolsQuery()}”.
        </p>
      </Show>
      <Show when={toolsError()}>{(message) => <p class="kria-capabilities__status" role="alert">{message()}</p>}</Show>

      <Show when={capabilityStore.loading()}>
        <LoadingRow label="Loading tools…" />
      </Show>

      <Show
        when={!capabilityStore.loading() && filtered().length > 0}
        fallback={
          <Show when={!capabilityStore.loading()}>
            <EmptyState
              icon="zap"
              title="No tools"
              description="KRIA's tool catalog is empty or unavailable right now."
            />
          </Show>
        }
      >
        <ul class="kria-capabilities__list">
          <For each={filtered()}>{(cap) => <CapabilityRow capability={cap} />}</For>
        </ul>
      </Show>
    </>
  );
}

// Remote-skill browse state (local to the Space).
const [browseQuery, setBrowseQuery] = createSignal("");
const [reviewSkill, setReviewSkill] = createSignal<RemoteSkillView | null>(null);

/** Skills — installed ClawHub/OpenClaw skills + remote browse with trust review. */
function SkillsRegion() {
  const skills = () => capabilityStore.skills();

  async function browse() {
    await capabilityStore.fetchRemoteSkills(browseQuery().trim());
  }

  return (
    <>
      <h2 class="kria-capabilities__region-title">Skills</h2>
      <OpenClawRuntimePanel />

      {/* Install-with-trust-review flow (Req 7.4): browse remote → review → install. */}
      <div class="kria-capabilities__browse">
        <Search
          class="kria-capabilities__search"
          label="Search ClawHub"
          placeholder="Search ClawHub skills to install"
          value={browseQuery()}
          onChange={(v) => setBrowseQuery(v)}
        />
        <Button variant="secondary" size="sm" onClick={browse} disabled={capabilityStore.remoteSkillsLoading()}>
          <Icon name="search" size={14} aria-hidden />
          {capabilityStore.remoteSkillsLoading() ? "Searching…" : "Search"}
        </Button>
      </div>

      <Show when={capabilityStore.remoteSkills().length > 0}>
        <ul class="kria-capabilities__grid" aria-label="Installable skills">
          <For each={capabilityStore.remoteSkills()}>
            {(rs) => (
              <li>
                <div class="kria-capcard" role="group" aria-label={rs.name}>
                  <div class="kria-capcard__head">
                    <span class="kria-capcard__name">
                      <Icon name="download" size={14} aria-hidden /> {rs.name}
                    </span>
                  </div>
                  <Show when={rs.description}>
                    <p class="kria-capcard__desc">{rs.description}</p>
                  </Show>
                  <div class="kria-capcard__actions">
                    <Button
                      variant="primary"
                      size="sm"
                      disabled={rs.installed}
                      onClick={() => setReviewSkill(rs)}
                    >
                      {rs.installed ? "Installed" : "Review & install"}
                    </Button>
                  </div>
                </div>
              </li>
            )}
          </For>
        </ul>
      </Show>

      <Show when={capabilityStore.loading()}>
        <LoadingRow label="Loading skills…" />
      </Show>
      <Show
        when={!capabilityStore.loading() && skills().length > 0}
        fallback={
          <Show when={!capabilityStore.loading()}>
            <EmptyState
              icon="sparkles"
              title="No skills installed"
              description="Skills installed from ClawHub will appear here with their trust tier."
            />
          </Show>
        }
      >
        <ul class="kria-capabilities__grid">
          <For each={skills()}>{(skill) => <SkillCard skill={skill} />}</For>
        </ul>
      </Show>

      {/* Trust review before install (Req 7.4). */}
      <Show when={reviewSkill()}>
        {(skill) => (
          <TrustReviewDialog
            skill={skill()}
            open={true}
            onOpenChange={(open) => {
              if (!open) setReviewSkill(null);
            }}
            onInstall={async (approvedCapabilities) => {
              await installSkill({
                slug: skill().slug,
                manifestUrl: skill().manifestUrl,
                approvedCapabilities: { capabilities: approvedCapabilities },
              });
            }}
          />
        )}
      </Show>
    </>
  );
}

/** Models — LLM providers + the models they expose. */
function ModelsRegion() {
  return (
    <>
      <h2 class="kria-capabilities__region-title">Models</h2>
      <Show when={capabilityStore.loading()}>
        <LoadingRow label="Loading provider runtime…" />
      </Show>
      <Show when={!capabilityStore.loading()}>
        <ModelsRuntimePanel />
      </Show>
    </>
  );
}

// Integration connect-form state (mcp/telegram need input; local to the Space).
const [connectKind, setConnectKind] = createSignal<"mcp" | "telegram" | null>(null);

/** Integrations — MCP servers + optional external connections (connect, Req 7.4). */
function IntegrationsRegion() {
  const integrations = () => capabilityStore.integrations();

  /**
   * Route a connect request by kind (Req 7.4). Google / Colab need no input and
   * dispatch directly; MCP / Telegram open a small connect form. Every path is
   * a dispatch-only call to an EXISTING backend command.
   */
  async function onConnect(it: IntegrationView) {
    switch (it.kind) {
      case "google":
        await connectGoogleWorkspace();
        break;
      case "colab":
        await connectColabTier();
        break;
      case "mcp":
        setConnectKind("mcp");
        break;
      case "telegram":
        setConnectKind("telegram");
        break;
    }
  }

  return (
    <>
      <h2 class="kria-capabilities__region-title">Integrations</h2>
      <Show when={capabilityStore.loading()}>
        <LoadingRow label="Loading integrations…" />
      </Show>
      <Show
        when={!capabilityStore.loading() && integrations().length > 0}
        fallback={
          <Show when={!capabilityStore.loading()}>
            <EmptyState
              icon="network"
              title="No integrations"
              description="MCP servers and external connections will appear here."
            />
          </Show>
        }
      >
        <ul class="kria-capabilities__grid">
          <For each={integrations()}>
            {(it) => <IntegrationCard integration={it} onConnect={onConnect} />}
          </For>
        </ul>
      </Show>

      {/* Connect form for kinds needing input (Req 7.4). */}
      <Show when={connectKind()}>
        {(kind) => (
          <IntegrationConnectDialog
            kind={kind()}
            open={true}
            onOpenChange={(open) => {
              if (!open) setConnectKind(null);
            }}
          />
        )}
      </Show>
    </>
  );
}
