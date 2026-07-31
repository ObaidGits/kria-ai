/**
 * Tests for Inspector component (task 4.4.1).
 *
 * Validates:
 * - Empty state when itemId=null
 * - Identity section: kind, displayName, authorityClass, policyLabel, validTime, transactionTime
 * - Truth section: truthState with data-truth-state, truthReason, contradictionCount, provenanceLabel
 * - Evidence section: evidence items with polarity
 * - Relationships section: relations with direction and registry label
 * - Use section: why-stored, why-recalled, how-used, used-in-trace-count
 * - History section: history events with event type and description
 * - Actions section: actions with enable/disable state
 * - Loading state per section with role="status"
 * - Error state per section with retry button and role="alert"
 * - Retry button calls onRetrySection with correct sectionId
 * - Section isolation: error in one section does not prevent others from rendering
 * - Correlation ID shown per section when non-null
 * - Graph revision shown per section when non-null
 * - Action click calls onAction with actionId
 * - Dangerous action has data-dangerous=true
 *
 * Requirements: F4.4 (task 4.4.1) — Inspector
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { Inspector } from "./Inspector";
import type {
  InspectorState,
  InspectorProps,
  IdentitySection,
  TruthSection,
  EvidenceSection,
  RelationshipsSection,
  UseSection,
  HistorySection,
  ActionsSection,
  InspectorSectionState,
  InspectorSectionMeta,
} from "./Inspector";

afterEach(() => cleanup());

// ─── Default section factories ────────────────────────────────────────────────

function sectionMeta(state: InspectorSectionState = 'idle', overrides: object = {}) {
  return {
    state,
    correlationId: null,
    graphRevision: null,
    lastLoadedAt: null,
    errorMessage: null,
    isRetrying: false,
    ...overrides,
  };
}

function makeIdentity(overrides: Partial<IdentitySection> = {}): IdentitySection {
  return {
    sectionId: 'identity',
    itemId: null,
    kind: null,
    displayName: null,
    aliases: [],
    authorityClass: null,
    policyLabel: null,
    validTimeStart: null,
    validTimeEnd: null,
    transactionTime: null,
    ...sectionMeta('idle'),
    ...overrides,
  };
}

function makeTruth(overrides: Partial<TruthSection> = {}): TruthSection {
  return {
    sectionId: 'truth',
    truthState: null,
    truthReason: null,
    contradictionCount: null,
    lastVerified: null,
    provenanceLabel: null,
    ...sectionMeta('idle'),
    ...overrides,
  };
}

function makeEvidence(overrides: Partial<EvidenceSection> = {}): EvidenceSection {
  return {
    sectionId: 'evidence',
    evidenceItems: [],
    totalCount: null,
    hasMore: false,
    ...sectionMeta('idle'),
    ...overrides,
  };
}

function makeRelationships(overrides: Partial<RelationshipsSection> = {}): RelationshipsSection {
  return {
    sectionId: 'relationships',
    relations: [],
    totalCount: null,
    hasMore: false,
    ...sectionMeta('idle'),
    ...overrides,
  };
}

function makeUse(overrides: Partial<UseSection> = {}): UseSection {
  return {
    sectionId: 'use',
    whyStored: null,
    whyRecalled: null,
    howUsed: null,
    usedInTraceCount: null,
    ...sectionMeta('idle'),
    ...overrides,
  };
}

function makeHistory(overrides: Partial<HistorySection> = {}): HistorySection {
  return {
    sectionId: 'history',
    events: [],
    totalCount: null,
    hasMore: false,
    ...sectionMeta('idle'),
    ...overrides,
  };
}

function makeActions(overrides: Partial<ActionsSection> = {}): ActionsSection {
  return {
    sectionId: 'actions',
    availableActions: [],
    ...sectionMeta('idle'),
    ...overrides,
  };
}

function makeState(overrides: Partial<InspectorState> = {}): InspectorState {
  return {
    itemId: 'item-1',
    identity: makeIdentity(),
    truth: makeTruth(),
    evidence: makeEvidence(),
    relationships: makeRelationships(),
    use: makeUse(),
    history: makeHistory(),
    actions: makeActions(),
    ...overrides,
  };
}

// Helper to create a default section by sectionId (used in loading/error loops)
function makeSection(id: string): InspectorSectionMeta & { sectionId: string } {
  const meta = sectionMeta('idle');
  switch (id) {
    case 'identity': return { ...makeIdentity(), ...meta };
    case 'truth': return { ...makeTruth(), ...meta };
    case 'evidence': return { ...makeEvidence(), ...meta };
    case 'relationships': return { ...makeRelationships(), ...meta };
    case 'use': return { ...makeUse(), ...meta };
    case 'history': return { ...makeHistory(), ...meta };
    case 'actions': return { ...makeActions(), ...meta };
    default: throw new Error(`Unknown sectionId: ${id}`);
  }
}

function renderInspector(stateOverrides: Partial<InspectorState> = {}, propOverrides: Partial<InspectorProps> = {}) {
  const onRetrySection = vi.fn();
  const onAction = vi.fn();
  const onNavigate = vi.fn();
  render(() => (
    <Inspector
      state={makeState(stateOverrides)}
      onRetrySection={onRetrySection}
      onAction={onAction}
      onNavigate={onNavigate}
      {...propOverrides}
    />
  ));
  return { onRetrySection, onAction, onNavigate };
}

// ─── Root / shell ─────────────────────────────────────────────────────────────

describe("inspector root", () => {
  it("renders inspector-root with aria-label=Inspector", () => {
    renderInspector();
    const root = screen.getByTestId("inspector-root");
    expect(root).toBeInTheDocument();
    expect(root).toHaveAttribute("aria-label", "Inspector");
  });

  it("shows inspector-empty with 'No item selected' when itemId=null", () => {
    renderInspector({ itemId: null });
    expect(screen.getByTestId("inspector-empty")).toHaveTextContent("No item selected");
  });

  it("does not show inspector-empty when itemId is set", () => {
    renderInspector({ itemId: "item-42" });
    expect(screen.queryByTestId("inspector-empty")).not.toBeInTheDocument();
  });

  it("renders all seven section elements", () => {
    renderInspector();
    for (const id of ["identity","truth","evidence","relationships","use","history","actions"]) {
      expect(screen.getByTestId(`inspector-section-${id}`)).toBeInTheDocument();
    }
  });

  it("each section has its aria-label matching sectionId", () => {
    renderInspector();
    for (const id of ["identity","truth","evidence","relationships","use","history","actions"]) {
      expect(screen.getByTestId(`inspector-section-${id}`)).toHaveAttribute("aria-label", id);
    }
  });
});

// ─── Section state attribute ──────────────────────────────────────────────────

describe("section data-section-state", () => {
  const states: InspectorSectionState[] = ["idle","loading","ready","empty","partial","stale","offline","error"];
  for (const state of states) {
    it(`identity section reflects data-section-state=${state}`, () => {
      renderInspector({ identity: makeIdentity({ state }) });
      expect(screen.getByTestId("inspector-section-identity")).toHaveAttribute("data-section-state", state);
    });
  }
});

// ─── Loading state per section ────────────────────────────────────────────────

describe("loading state", () => {
  const sectionIds = ["identity","truth","evidence","relationships","use","history","actions"] as const;

  for (const id of sectionIds) {
    it(`shows section-loading-${id} with role=status when state=loading`, () => {
      const overrides: Partial<InspectorState> = { [id]: { ...makeSection(id), state: 'loading' } };
      renderInspector(overrides);
      const el = screen.getByTestId(`section-loading-${id}`);
      expect(el).toBeInTheDocument();
      expect(el).toHaveAttribute("role", "status");
    });

    it(`does not show section-loading-${id} when state=ready`, () => {
      const overrides: Partial<InspectorState> = { [id]: { ...makeSection(id), state: 'ready' } };
      renderInspector(overrides);
      expect(screen.queryByTestId(`section-loading-${id}`)).not.toBeInTheDocument();
    });
  }
});

// ─── Error state per section ──────────────────────────────────────────────────

describe("error state", () => {
  const sectionIds = ["identity","truth","evidence","relationships","use","history","actions"] as const;

  for (const id of sectionIds) {
    it(`shows section-error-${id} with role=alert and error message when state=error`, () => {
      const overrides: Partial<InspectorState> = {
        [id]: { ...makeSection(id), state: 'error', errorMessage: `Error in ${id}` },
      };
      renderInspector(overrides);
      const el = screen.getByTestId(`section-error-${id}`);
      expect(el).toBeInTheDocument();
      expect(el).toHaveAttribute("role", "alert");
      expect(el).toHaveTextContent(`Error in ${id}`);
    });

    it(`shows retry button section-retry-${id} in error state`, () => {
      const overrides: Partial<InspectorState> = {
        [id]: { ...makeSection(id), state: 'error', errorMessage: 'oops' },
      };
      renderInspector(overrides);
      expect(screen.getByTestId(`section-retry-${id}`)).toBeInTheDocument();
    });

    it(`does not show section-error-${id} when state=ready`, () => {
      const overrides: Partial<InspectorState> = { [id]: { ...makeSection(id), state: 'ready' } };
      renderInspector(overrides);
      expect(screen.queryByTestId(`section-error-${id}`)).not.toBeInTheDocument();
    });
  }
});

// ─── Retry button calls onRetrySection ────────────────────────────────────────

describe("retry button", () => {
  const sectionIds = ["identity","truth","evidence","relationships","use","history","actions"] as const;

  for (const id of sectionIds) {
    it(`retry button for ${id} calls onRetrySection('${id}')`, () => {
      const overrides: Partial<InspectorState> = {
        [id]: { ...makeSection(id), state: 'error', errorMessage: 'err' },
      };
      const { onRetrySection } = renderInspector(overrides);
      fireEvent.click(screen.getByTestId(`section-retry-${id}`));
      expect(onRetrySection).toHaveBeenCalledWith(id);
    });
  }
});

// ─── Section isolation ────────────────────────────────────────────────────────

describe("section isolation", () => {
  it("error in identity does not prevent truth, evidence, and other sections from rendering", () => {
    renderInspector({
      identity: makeIdentity({ state: 'error', errorMessage: 'identity failed' }),
      truth: makeTruth({ state: 'ready', truthState: 'Current', truthReason: 'verified' }),
    });
    // identity is in error
    expect(screen.getByTestId("section-error-identity")).toBeInTheDocument();
    // truth still renders its section
    const truthSection = screen.getByTestId("inspector-section-truth");
    expect(truthSection).toBeInTheDocument();
    expect(truthSection.querySelector("[data-field='truth-state']")).toHaveTextContent("Current");
  });

  it("all seven sections render even if every section is in error state", () => {
    renderInspector({
      identity: makeIdentity({ state: 'error', errorMessage: 'e1' }),
      truth: makeTruth({ state: 'error', errorMessage: 'e2' }),
      evidence: makeEvidence({ state: 'error', errorMessage: 'e3' }),
      relationships: makeRelationships({ state: 'error', errorMessage: 'e4' }),
      use: makeUse({ state: 'error', errorMessage: 'e5' }),
      history: makeHistory({ state: 'error', errorMessage: 'e6' }),
      actions: makeActions({ state: 'error', errorMessage: 'e7' }),
    });
    for (const id of ["identity","truth","evidence","relationships","use","history","actions"]) {
      expect(screen.getByTestId(`inspector-section-${id}`)).toBeInTheDocument();
      expect(screen.getByTestId(`section-error-${id}`)).toBeInTheDocument();
    }
  });
});

// ─── Correlation ID ───────────────────────────────────────────────────────────

describe("correlation ID", () => {
  it("shows section-correlation-identity when correlationId is set", () => {
    renderInspector({ identity: makeIdentity({ correlationId: 'corr-abc' }) });
    expect(screen.getByTestId("section-correlation-identity")).toHaveTextContent("corr-abc");
  });

  it("hides section-correlation-identity when correlationId is null", () => {
    renderInspector({ identity: makeIdentity({ correlationId: null }) });
    expect(screen.queryByTestId("section-correlation-identity")).not.toBeInTheDocument();
  });

  it("shows section-correlation-truth when correlationId is set", () => {
    renderInspector({ truth: makeTruth({ correlationId: 'corr-xyz' }) });
    expect(screen.getByTestId("section-correlation-truth")).toHaveTextContent("corr-xyz");
  });
});

// ─── Graph revision ───────────────────────────────────────────────────────────

describe("graph revision", () => {
  it("shows section-revision-identity when graphRevision is set", () => {
    renderInspector({ identity: makeIdentity({ graphRevision: 55 }) });
    expect(screen.getByTestId("section-revision-identity")).toHaveTextContent("55");
  });

  it("hides section-revision-identity when graphRevision is null", () => {
    renderInspector({ identity: makeIdentity({ graphRevision: null }) });
    expect(screen.queryByTestId("section-revision-identity")).not.toBeInTheDocument();
  });

  it("shows section-revision-evidence when graphRevision is set", () => {
    renderInspector({ evidence: makeEvidence({ graphRevision: 12 }) });
    expect(screen.getByTestId("section-revision-evidence")).toHaveTextContent("12");
  });
});

// ─── Identity section data ────────────────────────────────────────────────────

describe("identity section data", () => {
  function renderIdentity(overrides: Partial<IdentitySection> = {}) {
    renderInspector({ identity: makeIdentity({ state: 'ready', ...overrides }) });
    return screen.getByTestId("inspector-section-identity");
  }

  it("renders kind", () => {
    const sec = renderIdentity({ kind: "entity" });
    expect(sec.querySelector("[data-field='kind']")).toHaveTextContent("entity");
  });

  it("renders displayName", () => {
    const sec = renderIdentity({ displayName: "Alice" });
    expect(sec.querySelector("[data-field='display-name']")).toHaveTextContent("Alice");
  });

  it("renders authorityClass", () => {
    const sec = renderIdentity({ authorityClass: "personal" });
    expect(sec.querySelector("[data-field='authority-class']")).toHaveTextContent("personal");
  });

  it("renders policyLabel (exact from backend)", () => {
    const sec = renderIdentity({ policyLabel: "policy-A" });
    expect(sec.querySelector("[data-field='policy-label']")).toHaveTextContent("policy-A");
  });

  it("renders validTime span when validTimeStart is set", () => {
    const sec = renderIdentity({ validTimeStart: "2024-01-01T00:00:00Z", validTimeEnd: "2025-01-01T00:00:00Z" });
    const el = sec.querySelector("[data-field='valid-time']");
    expect(el).not.toBeNull();
    expect(el).toHaveTextContent("2024-01-01T00:00:00Z");
    expect(el).toHaveTextContent("2025-01-01T00:00:00Z");
  });

  it("renders transactionTime", () => {
    const sec = renderIdentity({ transactionTime: "2024-06-01T12:00:00Z" });
    expect(sec.querySelector("[data-field='transaction-time']")).toHaveTextContent("2024-06-01T12:00:00Z");
  });

  it("hides kind when null", () => {
    const sec = renderIdentity({ kind: null });
    expect(sec.querySelector("[data-field='kind']")).toBeNull();
  });

  it("hides displayName when null", () => {
    const sec = renderIdentity({ displayName: null });
    expect(sec.querySelector("[data-field='display-name']")).toBeNull();
  });
});

// ─── Truth section data ───────────────────────────────────────────────────────

describe("truth section data", () => {
  function renderTruth(overrides: Partial<TruthSection> = {}) {
    renderInspector({ truth: makeTruth({ state: 'ready', ...overrides }) });
    return screen.getByTestId("inspector-section-truth");
  }

  it("renders truthState with data-truth-state attribute", () => {
    const sec = renderTruth({ truthState: "Current" });
    const el = sec.querySelector("[data-field='truth-state']");
    expect(el).toHaveTextContent("Current");
    expect(el).toHaveAttribute("data-truth-state", "Current");
  });

  it("renders truthReason", () => {
    const sec = renderTruth({ truthReason: "Verified by policy A" });
    expect(sec.querySelector("[data-field='truth-reason']")).toHaveTextContent("Verified by policy A");
  });

  it("renders contradictionCount", () => {
    const sec = renderTruth({ contradictionCount: 3 });
    expect(sec.querySelector("[data-field='contradiction-count']")).toHaveTextContent("3");
  });

  it("renders provenanceLabel (exact from backend)", () => {
    const sec = renderTruth({ provenanceLabel: "source-A/v1.2" });
    expect(sec.querySelector("[data-field='provenance-label']")).toHaveTextContent("source-A/v1.2");
  });

  it("hides truthState when null", () => {
    const sec = renderTruth({ truthState: null });
    expect(sec.querySelector("[data-field='truth-state']")).toBeNull();
  });

  it("hides contradictionCount when null", () => {
    const sec = renderTruth({ contradictionCount: null });
    expect(sec.querySelector("[data-field='contradiction-count']")).toBeNull();
  });
});

// ─── Evidence section data ────────────────────────────────────────────────────

describe("evidence section data", () => {
  it("renders evidence-list with items", () => {
    renderInspector({
      evidence: makeEvidence({
        state: 'ready',
        evidenceItems: [
          { id: "ev-1", source: "web", locator: null, method: "scrape", version: "1.0", polarity: "support", score: 0.9, semanticsLabel: "fact", policyLabel: null },
          { id: "ev-2", source: "doc", locator: "p.3", method: "extract", version: "2.1", polarity: "contradict", score: null, semanticsLabel: "claim", policyLabel: "policy-B" },
        ],
      }),
    });
    expect(screen.getByTestId("evidence-list")).toBeInTheDocument();
    expect(screen.getByTestId("evidence-item-ev-1")).toBeInTheDocument();
    expect(screen.getByTestId("evidence-item-ev-2")).toBeInTheDocument();
  });

  it("evidence item has data-field=polarity attribute", () => {
    renderInspector({
      evidence: makeEvidence({
        state: 'ready',
        evidenceItems: [
          { id: "ev-3", source: "x", locator: null, method: "m", version: "v", polarity: "support", score: null, semanticsLabel: "s", policyLabel: null },
        ],
      }),
    });
    const item = screen.getByTestId("evidence-item-ev-3");
    expect(item).toHaveAttribute("data-field", "polarity");
    expect(item).toHaveAttribute("data-polarity", "support");
  });

  it("contradict polarity item has data-polarity=contradict", () => {
    renderInspector({
      evidence: makeEvidence({
        state: 'ready',
        evidenceItems: [
          { id: "ev-4", source: "x", locator: null, method: "m", version: "v", polarity: "contradict", score: null, semanticsLabel: "s", policyLabel: null },
        ],
      }),
    });
    expect(screen.getByTestId("evidence-item-ev-4")).toHaveAttribute("data-polarity", "contradict");
  });

  it("does not render evidence-list when state=idle", () => {
    renderInspector({ evidence: makeEvidence({ state: 'idle' }) });
    expect(screen.queryByTestId("evidence-list")).not.toBeInTheDocument();
  });
});

// ─── Relationships section data ───────────────────────────────────────────────

describe("relationships section data", () => {
  it("renders relations-list with items", () => {
    renderInspector({
      relationships: makeRelationships({
        state: 'ready',
        relations: [
          { id: "rel-1", direction: "outgoing", registryLabel: "knows", sourceLabel: "Alice", targetLabel: "Bob", evidenceCount: 2, validity: "active" },
          { id: "rel-2", direction: "incoming", registryLabel: "member-of", sourceLabel: "Bob", targetLabel: "Team", evidenceCount: 1, validity: "active" },
        ],
      }),
    });
    expect(screen.getByTestId("relations-list")).toBeInTheDocument();
    expect(screen.getByTestId("relation-item-rel-1")).toBeInTheDocument();
    expect(screen.getByTestId("relation-item-rel-2")).toBeInTheDocument();
  });

  it("relation item has data-field=direction and data-direction attribute", () => {
    renderInspector({
      relationships: makeRelationships({
        state: 'ready',
        relations: [
          { id: "rel-3", direction: "outgoing", registryLabel: "knows", sourceLabel: "X", targetLabel: "Y", evidenceCount: 1, validity: "active" },
        ],
      }),
    });
    const item = screen.getByTestId("relation-item-rel-3");
    expect(item).toHaveAttribute("data-field", "direction");
    expect(item).toHaveAttribute("data-direction", "outgoing");
  });

  it("relation item renders registry-label", () => {
    renderInspector({
      relationships: makeRelationships({
        state: 'ready',
        relations: [
          { id: "rel-4", direction: "symmetric", registryLabel: "peer-of", sourceLabel: "A", targetLabel: "B", evidenceCount: 0, validity: "active" },
        ],
      }),
    });
    const item = screen.getByTestId("relation-item-rel-4");
    expect(item.querySelector("[data-field='registry-label']")).toHaveTextContent("peer-of");
  });

  it("does not render relations-list when state=idle", () => {
    renderInspector({ relationships: makeRelationships({ state: 'idle' }) });
    expect(screen.queryByTestId("relations-list")).not.toBeInTheDocument();
  });
});

// ─── Use section data ─────────────────────────────────────────────────────────

describe("use section data", () => {
  function renderUse(overrides: Partial<UseSection> = {}) {
    renderInspector({ use: makeUse({ state: 'ready', ...overrides }) });
    return screen.getByTestId("inspector-section-use");
  }

  it("renders why-stored", () => {
    const sec = renderUse({ whyStored: "Important context" });
    expect(sec.querySelector("[data-field='why-stored']")).toHaveTextContent("Important context");
  });

  it("renders why-recalled", () => {
    const sec = renderUse({ whyRecalled: "Matched query intent" });
    expect(sec.querySelector("[data-field='why-recalled']")).toHaveTextContent("Matched query intent");
  });

  it("renders how-used", () => {
    const sec = renderUse({ howUsed: "Referenced in answer" });
    expect(sec.querySelector("[data-field='how-used']")).toHaveTextContent("Referenced in answer");
  });

  it("renders used-in-trace-count", () => {
    const sec = renderUse({ usedInTraceCount: 7 });
    expect(sec.querySelector("[data-field='used-in-trace-count']")).toHaveTextContent("7");
  });

  it("hides why-stored when null", () => {
    const sec = renderUse({ whyStored: null });
    expect(sec.querySelector("[data-field='why-stored']")).toBeNull();
  });

  it("hides used-in-trace-count when null", () => {
    const sec = renderUse({ usedInTraceCount: null });
    expect(sec.querySelector("[data-field='used-in-trace-count']")).toBeNull();
  });
});

// ─── History section data ─────────────────────────────────────────────────────

describe("history section data", () => {
  it("renders history-list with events", () => {
    renderInspector({
      history: makeHistory({
        state: 'ready',
        events: [
          { id: "ev-h1", eventType: "correction", timestamp: "2024-01-01T00:00:00Z", actor: "user", description: "Corrected name" },
          { id: "ev-h2", eventType: "creation", timestamp: "2023-06-01T00:00:00Z", actor: null, description: "Item created" },
        ],
      }),
    });
    expect(screen.getByTestId("history-list")).toBeInTheDocument();
    expect(screen.getByTestId("history-event-ev-h1")).toBeInTheDocument();
    expect(screen.getByTestId("history-event-ev-h2")).toBeInTheDocument();
  });

  it("history event has data-field=event-type with data-event-type attribute", () => {
    renderInspector({
      history: makeHistory({
        state: 'ready',
        events: [{ id: "ev-h3", eventType: "supersession", timestamp: "2024-02-01T00:00:00Z", actor: null, description: "Superseded by newer entry" }],
      }),
    });
    const item = screen.getByTestId("history-event-ev-h3");
    expect(item).toHaveAttribute("data-field", "event-type");
    expect(item).toHaveAttribute("data-event-type", "supersession");
  });

  it("history event renders description", () => {
    renderInspector({
      history: makeHistory({
        state: 'ready',
        events: [{ id: "ev-h4", eventType: "deletion", timestamp: "2024-03-01T00:00:00Z", actor: "admin", description: "Deleted by admin" }],
      }),
    });
    const item = screen.getByTestId("history-event-ev-h4");
    expect(item.querySelector("[data-field='description']")).toHaveTextContent("Deleted by admin");
  });

  it("does not render history-list when state=idle", () => {
    renderInspector({ history: makeHistory({ state: 'idle' }) });
    expect(screen.queryByTestId("history-list")).not.toBeInTheDocument();
  });
});

// ─── Actions section data ─────────────────────────────────────────────────────

describe("actions section data", () => {
  function renderActionsSection(overrides: Partial<ActionsSection> = {}) {
    renderInspector({ actions: makeActions({ state: 'ready', ...overrides }) });
    return screen.getByTestId("inspector-section-actions");
  }

  it("renders actions-list with action buttons", () => {
    renderActionsSection({
      availableActions: [
        { id: "act-correct", label: "Correct", isEnabled: true, isDangerous: false, requiresPreview: false },
        { id: "act-forget", label: "Forget", isEnabled: false, isDangerous: true, requiresPreview: true },
      ],
    });
    expect(screen.getByTestId("actions-list")).toBeInTheDocument();
    expect(screen.getByTestId("inspector-action-act-correct")).toBeInTheDocument();
    expect(screen.getByTestId("inspector-action-act-forget")).toBeInTheDocument();
  });

  it("action button has aria-label from label field", () => {
    renderActionsSection({
      availableActions: [{ id: "act-a", label: "Inspect Item", isEnabled: true, isDangerous: false, requiresPreview: false }],
    });
    expect(screen.getByTestId("inspector-action-act-a")).toHaveAttribute("aria-label", "Inspect Item");
  });

  it("enabled action is not disabled", () => {
    renderActionsSection({
      availableActions: [{ id: "act-b", label: "Correct", isEnabled: true, isDangerous: false, requiresPreview: false }],
    });
    expect(screen.getByTestId("inspector-action-act-b")).not.toBeDisabled();
  });

  it("disabled action has disabled attribute", () => {
    renderActionsSection({
      availableActions: [{ id: "act-c", label: "Delete", isEnabled: false, isDangerous: true, requiresPreview: false }],
    });
    expect(screen.getByTestId("inspector-action-act-c")).toBeDisabled();
  });

  it("disabled action has aria-disabled=true", () => {
    renderActionsSection({
      availableActions: [{ id: "act-d", label: "Delete", isEnabled: false, isDangerous: true, requiresPreview: false }],
    });
    expect(screen.getByTestId("inspector-action-act-d")).toHaveAttribute("aria-disabled", "true");
  });

  it("enabled action does not have aria-disabled", () => {
    renderActionsSection({
      availableActions: [{ id: "act-e", label: "Save", isEnabled: true, isDangerous: false, requiresPreview: false }],
    });
    expect(screen.getByTestId("inspector-action-act-e")).not.toHaveAttribute("aria-disabled");
  });

  it("dangerous action has data-dangerous=true", () => {
    renderActionsSection({
      availableActions: [{ id: "act-f", label: "Purge", isEnabled: true, isDangerous: true, requiresPreview: false }],
    });
    expect(screen.getByTestId("inspector-action-act-f")).toHaveAttribute("data-dangerous", "true");
  });

  it("non-dangerous action has data-dangerous=false", () => {
    renderActionsSection({
      availableActions: [{ id: "act-g", label: "Inspect", isEnabled: true, isDangerous: false, requiresPreview: false }],
    });
    expect(screen.getByTestId("inspector-action-act-g")).toHaveAttribute("data-dangerous", "false");
  });

  it("clicking enabled action calls onAction with actionId", () => {
    const onAction = vi.fn();
    render(() => (
      <Inspector
        state={makeState({ actions: makeActions({ state: 'ready', availableActions: [
          { id: "act-h", label: "Correct", isEnabled: true, isDangerous: false, requiresPreview: false },
        ]}) })}
        onRetrySection={vi.fn()}
        onAction={onAction}
        onNavigate={vi.fn()}
      />
    ));
    fireEvent.click(screen.getByTestId("inspector-action-act-h"));
    expect(onAction).toHaveBeenCalledWith("act-h");
  });

  it("clicking disabled action does not call onAction", () => {
    const onAction = vi.fn();
    render(() => (
      <Inspector
        state={makeState({ actions: makeActions({ state: 'ready', availableActions: [
          { id: "act-i", label: "Delete", isEnabled: false, isDangerous: true, requiresPreview: false },
        ]}) })}
        onRetrySection={vi.fn()}
        onAction={onAction}
        onNavigate={vi.fn()}
      />
    ));
    fireEvent.click(screen.getByTestId("inspector-action-act-i"));
    expect(onAction).not.toHaveBeenCalled();
  });

  it("does not render actions-list when state=idle", () => {
    renderInspector({ actions: makeActions({ state: 'idle' }) });
    expect(screen.queryByTestId("actions-list")).not.toBeInTheDocument();
  });
});
