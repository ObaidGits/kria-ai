/**
 * context — the homepage Context Engine (Phase 6).
 *
 * A lightweight layer that determines WHAT the user is doing and exposes it as a
 * single reactive `activeContext`. Everything adaptive on the homepage (Orbit
 * capability set, Presence Line, Composer placeholder) is *derived* from this —
 * no component hardcodes context logic; they consume `currentContext()`.
 *
 * Detection is deliberately pluggable. Today the only source is a demo signal
 * (`setActiveContext`, cycled from the top bar / ⌥⇧C). Tomorrow real sources —
 * active application, open document, current project, calendar, clipboard, AI
 * reasoning — can call `setActiveContext` (or a future scored resolver) WITHOUT
 * touching the Orbit, Composer, or Presence Line. That is the whole point of the
 * separation: context flows one way, UI only reads.
 */
import { createSignal } from "solid-js";
import { CAPABILITIES, type Capability } from "./capabilities";

export type HomeContext =
  | "general"
  | "coding"
  | "writing"
  | "meetings"
  | "automation"
  | "research"
  | "documents";

/** Operational status vocabulary for the Command Deck (Phase 7). */
export type OpStatus = "running" | "waiting" | "done" | "attention";

export interface DeckOperation {
  label: string;
  icon: string;
  status: OpStatus;
}

export interface ContextDef {
  id: HomeContext;
  label: string;
  /** Ordered capability ids (into CAPABILITIES) the Orbit surfaces here. */
  orbit: string[];
  /** One calm Presence Line reflecting this context (lead text). */
  presence: string;
  /** Optional trailing highlighted word/name (e.g. "Obaid."). */
  accent?: string;
  /** Optional second, quieter presence line. */
  sub?: string;
  /** Composer placeholder that matches the current activity. */
  placeholder: string;
  /** Command Deck: the current operational objective (one glance). */
  objective: string;
  /** Command Deck: heading for the primary "current activity" zone. */
  deckFocus: string;
  /** Command Deck: contextual quick operational actions. */
  operations: DeckOperation[];
}

export const CONTEXTS: Record<HomeContext, ContextDef> = {
  general: {
    id: "general",
    label: "General",
    orbit: ["memory", "documents", "automation", "calendar", "agents", "terminal"],
    presence: "Good evening,",
    accent: " Obaid.",
    sub: "You're free for the next hour.",
    placeholder: "Talk to KRIA, or type a command…",
    objective: "Standing by for your direction.",
    deckFocus: "Operations",
    operations: [
      { label: "Open Logs", icon: "brief", status: "running" },
      { label: "Review Output", icon: "check", status: "waiting" },
      { label: "Pause Agent", icon: "focus", status: "running" },
    ],
  },
  coding: {
    id: "coding",
    label: "Coding",
    orbit: ["terminal", "git", "projectMemory", "debugger", "logs", "agents"],
    presence: "I'm watching your project. Two files changed.",
    placeholder: "Explain this function, or ask about the code…",
    objective: "Ship the Command Center Mission Control.",
    deckFocus: "Project Operations",
    operations: [
      { label: "Run Build", icon: "play", status: "running" },
      { label: "Review Diff", icon: "git", status: "attention" },
      { label: "Open Logs", icon: "brief", status: "running" },
    ],
  },
  writing: {
    id: "writing",
    label: "Writing",
    orbit: ["research", "drafts", "documents", "summarize", "rewrite", "memory"],
    presence: "Your draft is ready when you are.",
    placeholder: "Help me rewrite this, or draft something new…",
    objective: "Finish the local-first AI post.",
    deckFocus: "Document Operations",
    operations: [
      { label: "Approve Draft", icon: "check", status: "waiting" },
      { label: "Summarize", icon: "book", status: "waiting" },
      { label: "Rewrite Intro", icon: "chat", status: "waiting" },
    ],
  },
  meetings: {
    id: "meetings",
    label: "Meetings",
    orbit: ["calendar", "recorder", "notes", "tasks", "summary", "memory"],
    presence: "Your next meeting starts in 15 minutes.",
    placeholder: "Prepare meeting notes, or summarise the agenda…",
    objective: "Prepare for the design review.",
    deckFocus: "Meeting Operations",
    operations: [
      { label: "Start Recording", icon: "mic", status: "waiting" },
      { label: "Take Notes", icon: "book", status: "waiting" },
      { label: "Share Summary", icon: "send", status: "done" },
    ],
  },
  automation: {
    id: "automation",
    label: "Automation",
    orbit: ["runningTasks", "queue", "history", "logs", "agents", "triggers"],
    presence: "Two automations are running smoothly.",
    placeholder: "Create a workflow, or check what's running…",
    objective: "Keep automations healthy.",
    deckFocus: "Automation Queue",
    operations: [
      { label: "Resume Automation", icon: "play", status: "running" },
      { label: "Pause Agent", icon: "focus", status: "running" },
      { label: "Open Logs", icon: "brief", status: "attention" },
    ],
  },
  research: {
    id: "research",
    label: "Research",
    orbit: ["research", "documents", "summarize", "memory", "notes", "agents"],
    presence: "Ready to dig into whatever you're exploring.",
    placeholder: "Research a topic, or ask me to compare sources…",
    objective: "Compare Tauri IPC approaches.",
    deckFocus: "Research Operations",
    operations: [
      { label: "New Search", icon: "search", status: "waiting" },
      { label: "Save Source", icon: "check", status: "done" },
      { label: "Summarize", icon: "book", status: "waiting" },
    ],
  },
  documents: {
    id: "documents",
    label: "Documents",
    orbit: ["documents", "research", "summarize", "drafts", "memory", "agents"],
    presence: "128 documents indexed and ready to search.",
    placeholder: "Find a document, or ask about its contents…",
    objective: "Organise the knowledge base.",
    deckFocus: "Document Operations",
    operations: [
      { label: "Reindex", icon: "flow", status: "running" },
      { label: "Search Docs", icon: "search", status: "waiting" },
      { label: "Summarize", icon: "book", status: "waiting" },
    ],
  },
};

/** Demo ordering for the ⌥⇧C / top-bar cycle. */
export const CONTEXT_ORDER: HomeContext[] = [
  "general",
  "coding",
  "writing",
  "meetings",
  "automation",
  "research",
  "documents",
];

const [activeContext, setActiveContext] = createSignal<HomeContext>("general");
export { activeContext, setActiveContext };

/** The resolved definition for the current context. */
export function currentContext(): ContextDef {
  return CONTEXTS[activeContext()];
}

/** The Orbit capabilities for the current context (resolved objects, deduped). */
export function currentOrbit(): Capability[] {
  return currentContext()
    .orbit.map((id) => CAPABILITIES[id])
    .filter((c): c is Capability => Boolean(c));
}

/** Command Deck: contextual operational actions for the current context. */
export function currentOperations(): DeckOperation[] {
  return currentContext().operations;
}

/** Advance to the next context (demo signal — stands in for real detection). */
export function cycleContext() {
  const order = CONTEXT_ORDER;
  const i = order.indexOf(activeContext());
  setActiveContext(order[(i + 1) % order.length]);
}


// ── Living Core cognition model ─────────────────────────────────────────────
// This is a bounded presentation model. A future runtime resolver can drive the
// same contract without changing the homepage components.
export type CoreState = "idle" | "listening" | "thinking" | "retrieving" | "executing";

export interface CognitionSnapshot {
  state: CoreState;
  stateLabel: string;
  activity: string;
  detail: string;
  goal: string;
  evidence: string;
  memory: string;
  nextAction: string;
  effort: string;
  confidence: number;
}

const COGNITION_BY_CONTEXT: Record<HomeContext, Omit<CognitionSnapshot, "state" | "stateLabel" | "activity" | "detail">> = {
  general: {
    goal: "Choose the most useful next step.",
    evidence: "No urgent action is running; the next hour is open.",
    memory: "Your last session ended while refining KRIA's interface.",
    nextAction: "Continue the interface work",
    effort: "About 20 min",
    confidence: 86,
  },
  coding: {
    goal: "Move the active project forward without losing context.",
    evidence: "Recent code changes and project memory are available locally.",
    memory: "The last coding session focused on Command Center interaction quality.",
    nextAction: "Review the latest project changes",
    effort: "About 10 min",
    confidence: 91,
  },
  writing: {
    goal: "Turn the current draft into a clear finished argument.",
    evidence: "A draft and related research are present in this context.",
    memory: "The opening needed a stronger local-first point of view.",
    nextAction: "Continue the draft",
    effort: "About 25 min",
    confidence: 83,
  },
  meetings: {
    goal: "Enter the next conversation with decisions and context ready.",
    evidence: "The design review is the next relevant calendar context.",
    memory: "The last review left one open interaction-design decision.",
    nextAction: "Prepare a concise meeting brief",
    effort: "About 8 min",
    confidence: 88,
  },
  automation: {
    goal: "Keep active workflows understandable and under control.",
    evidence: "Automation context exposes queue, history, logs, and triggers.",
    memory: "The most recent workflow review prioritised observable failures.",
    nextAction: "Inspect automation outcomes",
    effort: "About 6 min",
    confidence: 89,
  },
  research: {
    goal: "Build a useful answer from relevant local knowledge.",
    evidence: "Research, documents, notes, and memory are available together.",
    memory: "The current comparison concerns Tauri IPC approaches.",
    nextAction: "Continue the source comparison",
    effort: "About 18 min",
    confidence: 81,
  },
  documents: {
    goal: "Find the right source without making the user browse manually.",
    evidence: "The local document index is the active knowledge context.",
    memory: "Recent work centred on organising the knowledge base.",
    nextAction: "Search related documents",
    effort: "About 4 min",
    confidence: 92,
  },
};

export const [coreState, setCoreState] = createSignal<CoreState>("idle");
export const [activeIntent, setActiveIntent] = createSignal("");

function intentSummary(): string {
  const intent = activeIntent().trim();
  if (!intent) return "your request";
  return intent.length > 56 ? `${intent.slice(0, 53)}…` : `“${intent}”`;
}

export function currentCognition(): CognitionSnapshot {
  const baseline = COGNITION_BY_CONTEXT[activeContext()];
  const state = coreState();
  const live: Record<CoreState, Pick<CognitionSnapshot, "stateLabel" | "activity" | "detail">> = {
    idle: {
      stateLabel: "Ready",
      activity: "Everything is ready",
      detail: `${currentContext().label} work is loaded and context is preserved. KRIA will wait for your direction.`,
    },
    listening: {
      stateLabel: "Listening",
      activity: "Listening for your next instruction",
      detail: "Voice activity is represented locally and remains under your control.",
    },
    thinking: {
      stateLabel: "Reasoning",
      activity: `Structuring ${intentSummary()}`,
      detail: "Separating intent, context, constraints, and the safest next step.",
    },
    retrieving: {
      stateLabel: "Retrieving",
      activity: "Connecting relevant project memory",
      detail: "Prioritising context that can explain and support the next action.",
    },
    executing: {
      stateLabel: "Executing",
      activity: "Following the approved plan",
      detail: "Progress and verification remain visible while the action runs.",
    },
  };
  return { state, ...live[state], ...baseline };
}
