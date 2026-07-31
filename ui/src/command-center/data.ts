/**
 * Command Center — static demo data (frontend-only).
 *
 * This surface is a pure presentation mock: every value below is hardcoded
 * demo content. It performs NO backend calls, no store reads, and no tool
 * execution — it exists to reproduce the "command center" homepage visual.
 *
 * NOTE: this module lives OUTSIDE the token-lint / component-audit /
 * expansion-governance scan roots (src/{design-system,kit,shell,palette,
 * prototypes}) by design, so the HUD can carry its own bespoke theme without
 * touching the presence-homepage token system or the canonical 7-Space set.
 */

export type StatusTone = "online" | "active" | "standby" | "idle" | "warn" | "offline";

export interface OverviewRow {
  id: string;
  icon: string;
  label: string;
  detail: string;
  tone: StatusTone;
}

export interface FeedItem {
  id: string;
  icon: string;
  title: string;
  sub: string;
  tag: "INFO" | "WARN" | "TIP" | "FOCUS" | "LIVE" | "ALERT";
  action?: string;
}

export interface AgentCard {
  id: string;
  name: string;
  icon: string;
  tone: StatusTone;
  status: string;
}

export interface TimelineItem {
  id: string;
  time: string;
  title: string;
  meta: string;
  done: boolean;
}

export interface Gauge {
  id: string;
  label: string;
  value: number; // 0..100
}

export interface Provider {
  id: string;
  name: string;
  icon: string;
  state: "connected" | "linked" | "not-linked" | "no-models";
}

export const BRAND = {
  name: "K.R.I.A.",
  tagline: "AI CORE",
  coreTitle: "KRIA",
  coreSub: "A I   C O R E",
  coreVersion: "v3.0.0",
};

export const OVERVIEW: OverviewRow[] = [
  { id: "core", icon: "core", label: "AI Core", detail: "Active", tone: "active" },
  { id: "memory", icon: "memory", label: "Memory", detail: "3,380 Stored", tone: "online" },
  { id: "voice", icon: "mic", label: "Voice", detail: "Online", tone: "online" },
  { id: "agents", icon: "agents", label: "Agents", detail: "2 Running", tone: "active" },
  { id: "llms", icon: "spark", label: "LLMs", detail: "4 Connected", tone: "online" },
  { id: "system", icon: "shield", label: "System", detail: "Optimal", tone: "online" },
];

export const FEED: FeedItem[] = [
  { id: "f1", icon: "calendar", title: "Design review with the product team", sub: "Meeting", tag: "INFO" },
  { id: "f2", icon: "warn", title: "2 tasks are overdue — \"Polish voice…\"", sub: "Overdue", tag: "WARN" },
  { id: "f3", icon: "git", title: "3 pull requests are awaiting your review", sub: "GitHub", tag: "TIP" },
  { id: "f4", icon: "focus", title: "Your deep-work block is 2–4 PM. Notify…", sub: "Focus", tag: "FOCUS" },
  { id: "f5", icon: "cpu", title: "CPU usage at 15%", sub: "System load nominal", tag: "LIVE" },
  { id: "f6", icon: "warn", title: "2 tasks overdue", sub: "Review the board and reschedule…", tag: "ALERT", action: "View Tasks" },
];

export const AGENTS: AgentCard[] = [
  { id: "coding", name: "Coding Agent", icon: "code", tone: "active", status: "Active" },
  { id: "research", name: "Research Agent", icon: "search", tone: "active", status: "Active" },
  { id: "memory", name: "Memory Agent", icon: "memory", tone: "standby", status: "Standby" },
  { id: "browser", name: "Browser Agent", icon: "globe", tone: "standby", status: "Standby" },
  { id: "task", name: "Task Agent", icon: "tasks", tone: "standby", status: "Standby" },
  { id: "system", name: "System Agent", icon: "shield", tone: "standby", status: "Standby" },
];

export const TIMELINE: TimelineItem[] = [
  { id: "t1", time: "09:30 AM", title: "Daily Standup", meta: "Done", done: true },
  { id: "t2", time: "11:00 AM", title: "Finalise HUD panel spacing", meta: "In 42 min", done: false },
  { id: "t3", time: "02:00 PM", title: "Deep-work block: Voice pipeline", meta: "In 3h 10m", done: false },
  { id: "t4", time: "05:00 PM", title: "Design Review — Command Center V1", meta: "In 5h 30m", done: false },
];

export const GAUGES: Gauge[] = [
  { id: "cpu", label: "CPU", value: 15 },
  { id: "ram", label: "RAM", value: 54 },
  { id: "disk", label: "Disk", value: 40 },
];

export const MEMORY_STATS = [
  { id: "mem", label: "Memories", value: "3,380" },
  { id: "turns", label: "Session Turns", value: "22" },
  { id: "tools", label: "Tool Calls", value: "14" },
];

export const PROVIDERS: Provider[] = [
  { id: "claude", name: "Claude", icon: "spark", state: "linked" },
  { id: "openai", name: "OpenAI", icon: "spark", state: "not-linked" },
  { id: "gemini", name: "Gemini", icon: "spark", state: "not-linked" },
  { id: "groq", name: "Groq", icon: "spark", state: "connected" },
  { id: "openrouter", name: "OpenRouter", icon: "spark", state: "connected" },
  { id: "ollama", name: "Ollama", icon: "spark", state: "no-models" },
  { id: "claude-code", name: "Claude Code", icon: "code", state: "connected" },
  { id: "cursor", name: "Cursor", icon: "code", state: "connected" },
  { id: "copilot", name: "Copilot", icon: "code", state: "connected" },
];

export const BOTTOM = {
  location: "Bhimber, Pakistan",
  weather: "28°C Overcast",
  network: "Excellent",
  listening: "I am listening…",
};

export const PROVIDER_STATE_LABEL: Record<Provider["state"], string> = {
  connected: "Connected",
  linked: "Linked",
  "not-linked": "Not Linked",
  "no-models": "No Models",
};

// ── Presence homepage (Phase 4) — a few suggestions + at most ONE adaptive
//    context subject. The Presence Line itself is now context-driven (Phase 6,
//    see `context.ts`). Static demo data. ──

export interface ActionChip {
  id: string;
  label: string;
  icon: string;
}

/** ≤5 low-weight suggestions that follow the Composer (suggestions, not nav). */
export const ACTION_CHIPS: ActionChip[] = [
  { id: "continue", label: "Continue", icon: "play" },
  { id: "summarize", label: "Summarize", icon: "book" },
  { id: "search", label: "Search", icon: "search" },
  { id: "plan", label: "Plan", icon: "flow" },
  { id: "automate", label: "Automate", icon: "bolt" },
];

export interface ContextSubject {
  icon: string;
  title: string;
  line: string;
  time: string;
  priority: string;
  action: string;
}

/**
 * The single most-relevant thing KRIA wants surfaced right now. `null` → the
 * Adaptive Context Surface dissolves (the homepage stays calm, no placeholder).
 */
export const CONTEXT_SUBJECT: ContextSubject | null = {
  icon: "calendar",
  title: "Standup in 20 minutes",
  line: "Design review with the product team.",
  time: "09:10 PM",
  priority: "High Priority",
  action: "View Details",
};

// ── Right status rail (full HUD homepage layout) — static demo data. ──

export interface StatusRow {
  id: string;
  icon: string;
  label: string;
  value: string;
  tone: StatusTone;
}

/** ACTIVE STATUS card — system posture at a glance. */
export const ACTIVE_STATUS: StatusRow[] = [
  { id: "health", icon: "shield", label: "System Health", value: "Optimal", tone: "online" },
  { id: "memory", icon: "sync", label: "Memory Sync", value: "Synced", tone: "online" },
  { id: "models", icon: "grid", label: "AI Models", value: "All Online", tone: "active" },
  { id: "network", icon: "wifi", label: "Network", value: "Excellent", tone: "online" },
];

/** FOCUS SUGGESTION card — a single recommended focus window + a readiness ring. */
export const FOCUS_SUGGESTION = {
  title: "Deep work window",
  window: "8:00 PM – 10:00 PM",
  percent: 78,
  distractions: "Low",
};

export interface ActivityItem {
  id: string;
  label: string;
  time: string;
  tone: StatusTone | "violet";
}

/** RECENT ACTIVITY card — the last few things KRIA did. */
export const RECENT_ACTIVITY: ActivityItem[] = [
  { id: "a1", label: "Project Brief Analyzed", time: "8m ago", tone: "active" },
  { id: "a2", label: "Automation Completed", time: "12m ago", tone: "online" },
  { id: "a3", label: "Notes Summarized", time: "25m ago", tone: "violet" },
];
