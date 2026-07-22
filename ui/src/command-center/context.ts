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
