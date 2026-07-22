/**
 * capabilities — the single source of truth for what KRIA can do (Phase 6).
 *
 * Every capability is defined ONCE here: its Orbit presentation (label, icon,
 * short preview `description`) and its single Context Surface content (title,
 * summary, rows, optional actions). Contexts (see `context.ts`) reference these
 * by id, so the Orbit and the Context Surface never duplicate capability data —
 * adding a capability or reusing one across contexts is a one-line change.
 *
 * Static demo content (frontend-only). No backend, no stores.
 */

export interface CapabilityRow {
  icon: string;
  label: string;
  detail: string;
}

export interface CapabilityAction {
  label: string;
  icon: string;
}

export interface Capability {
  id: string;
  label: string;
  icon: string;
  /** Lightweight preview shown on Orbit hover/focus (no screen-covering tooltip). */
  description: string;
  panel: {
    summary: string;
    rows: CapabilityRow[];
    actions?: CapabilityAction[];
  };
}

function cap(
  id: string,
  label: string,
  icon: string,
  description: string,
  summary: string,
  rows: CapabilityRow[],
  actions?: CapabilityAction[],
): Capability {
  return { id, label, icon, description, panel: { summary, rows, actions } };
}

export const CAPABILITIES: Record<string, Capability> = {
  memory: cap("memory", "Memory", "brain", "Recall facts and past context", "3,380 memories · indexed 4 min ago", [
    { icon: "spark", label: "Recent recall", detail: "Voice pipeline decisions" },
    { icon: "book", label: "Knowledge base", detail: "128 documents" },
    { icon: "focus", label: "Decay review", detail: "12 fading facts" },
  ], [{ label: "Search memory", icon: "search" }]),

  documents: cap("documents", "Documents", "book", "Open and search your files", "128 indexed · 3 opened today", [
    { icon: "book", label: "Command Center spec", detail: "edited 1h ago" },
    { icon: "book", label: "Voice roadmap", detail: "edited yesterday" },
    { icon: "search", label: "Search documents", detail: "semantic + lexical" },
  ], [{ label: "New document", icon: "plus" }]),

  automation: cap("automation", "Automation", "bolt", "Run and schedule workflows", "2 running · 5 scheduled", [
    { icon: "flow", label: "Nightly index", detail: "running · 62%" },
    { icon: "play", label: "Morning briefing", detail: "scheduled 07:30" },
    { icon: "tasks", label: "History", detail: "41 runs this week" },
  ], [{ label: "New workflow", icon: "plus" }]),

  calendar: cap("calendar", "Calendar", "calendar", "Your schedule at a glance", "Standup in 20 minutes", [
    { icon: "calendar", label: "Daily standup", detail: "09:30 · product team" },
    { icon: "focus", label: "Deep-work block", detail: "14:00 – 16:00" },
    { icon: "calendar", label: "Design review", detail: "17:00 · Command Center" },
  ]),

  agents: cap("agents", "Agents", "agents", "Delegate to specialist agents", "2 active · 4 on standby", [
    { icon: "code", label: "Coding Agent", detail: "active" },
    { icon: "search", label: "Research Agent", detail: "active" },
    { icon: "memory", label: "Memory Agent", detail: "standby" },
  ]),

  terminal: cap("terminal", "Terminal", "code", "Run commands in your project", "Project: KRIA · branch main", [
    { icon: "code", label: "Run build", detail: "cargo build" },
    { icon: "git", label: "Git status", detail: "2 files changed" },
    { icon: "cpu", label: "Processes", detail: "3 background tasks" },
  ], [{ label: "Open terminal", icon: "play" }]),

  git: cap("git", "Git", "git", "Review changes and history", "branch main · 2 changed", [
    { icon: "git", label: "Uncommitted", detail: "command-center.css, data.ts" },
    { icon: "check", label: "Last commit", detail: "Phase 5 — AI navigation" },
    { icon: "flow", label: "Open PRs", detail: "3 awaiting review" },
  ], [{ label: "Stage all", icon: "check" }, { label: "Diff", icon: "code" }]),

  projectMemory: cap("projectMemory", "Project Memory", "brain", "What KRIA knows about this project", "KRIA · 412 project facts", [
    { icon: "spark", label: "Architecture", detail: "surface router, 7-Space cap" },
    { icon: "focus", label: "Conventions", detail: "cyan/violet HUD theme" },
    { icon: "book", label: "Decisions", detail: "18 recorded ADRs" },
  ]),

  debugger: cap("debugger", "Debugger", "warn", "Inspect and step through issues", "No active breakpoints", [
    { icon: "warn", label: "Recent errors", detail: "0 in this session" },
    { icon: "cpu", label: "Watches", detail: "none set" },
    { icon: "code", label: "Call stack", detail: "idle" },
  ]),

  logs: cap("logs", "Logs", "brief", "Tail recent output", "tracing · info level", [
    { icon: "cpu", label: "CPU usage", detail: "15% · nominal" },
    { icon: "spark", label: "Last event", detail: "index complete" },
    { icon: "warn", label: "Warnings", detail: "0 in last hour" },
  ]),

  research: cap("research", "Research", "search", "Gather and synthesise sources", "Ready to search the web", [
    { icon: "search", label: "Recent query", detail: "Tauri v2 IPC patterns" },
    { icon: "book", label: "Saved sources", detail: "9 in this topic" },
    { icon: "spark", label: "Synthesis", detail: "draft summary ready" },
  ], [{ label: "New search", icon: "search" }]),

  drafts: cap("drafts", "Drafts", "book", "Your works in progress", "3 drafts · 1 updated today", [
    { icon: "book", label: "Blog: local-first AI", detail: "edited 20 min ago" },
    { icon: "chat", label: "Reply to design team", detail: "draft" },
    { icon: "book", label: "Release notes v3", detail: "outline" },
  ], [{ label: "New draft", icon: "plus" }]),

  summarize: cap("summarize", "Summarize", "book", "Condense the current content", "Ready to summarize", [
    { icon: "book", label: "Selected document", detail: "Voice roadmap · 12 pages" },
    { icon: "spark", label: "Length", detail: "short · bullet points" },
    { icon: "chat", label: "Last summary", detail: "Command Center spec" },
  ], [{ label: "Summarize now", icon: "spark" }]),

  rewrite: cap("rewrite", "Rewrite", "chat", "Rephrase and refine text", "Ready to rewrite", [
    { icon: "chat", label: "Tone", detail: "clear · concise" },
    { icon: "spark", label: "Last rewrite", detail: "intro paragraph" },
    { icon: "book", label: "Target", detail: "selected draft" },
  ], [{ label: "Rewrite selection", icon: "spark" }]),

  recorder: cap("recorder", "Recorder", "mic", "Capture and transcribe audio", "Idle · mic ready", [
    { icon: "mic", label: "Live transcription", detail: "Whisper · on device" },
    { icon: "brief", label: "Last recording", detail: "standup · 12 min" },
    { icon: "book", label: "Transcripts", detail: "7 saved" },
  ], [{ label: "Start recording", icon: "mic" }]),

  notes: cap("notes", "Notes", "book", "Quick meeting notes", "2 notes today", [
    { icon: "book", label: "Standup notes", detail: "3 action items" },
    { icon: "check", label: "Follow-ups", detail: "2 open" },
    { icon: "spark", label: "Auto-notes", detail: "from transcript" },
  ], [{ label: "New note", icon: "plus" }]),

  tasks: cap("tasks", "Tasks", "tasks", "Track what needs doing", "3 due today · 2 overdue", [
    { icon: "warn", label: "Overdue", detail: "Polish voice overlay" },
    { icon: "tasks", label: "Due today", detail: "HUD panel spacing" },
    { icon: "check", label: "Done", detail: "Daily standup" },
  ], [{ label: "New task", icon: "plus" }]),

  summary: cap("summary", "Summary", "brief", "Recap of the meeting", "Last meeting · 12 min", [
    { icon: "brief", label: "Key points", detail: "5 captured" },
    { icon: "check", label: "Decisions", detail: "2 made" },
    { icon: "tasks", label: "Action items", detail: "3 assigned" },
  ], [{ label: "Share summary", icon: "send" }]),

  runningTasks: cap("runningTasks", "Running Tasks", "play", "What KRIA is doing now", "2 automations running", [
    { icon: "flow", label: "Nightly index", detail: "62% · 4 min left" },
    { icon: "play", label: "Doc embedding", detail: "queued" },
    { icon: "cpu", label: "Resource use", detail: "moderate" },
  ]),

  queue: cap("queue", "Queue", "flow", "Pending automation runs", "5 items queued", [
    { icon: "flow", label: "Morning briefing", detail: "07:30" },
    { icon: "book", label: "Weekly digest", detail: "Friday 17:00" },
    { icon: "spark", label: "Re-index", detail: "on file change" },
  ], [{ label: "Pause queue", icon: "focus" }]),

  history: cap("history", "History", "tasks", "Past automation runs", "41 runs this week", [
    { icon: "check", label: "Nightly index", detail: "succeeded · 02:00" },
    { icon: "check", label: "Morning briefing", detail: "succeeded · 07:30" },
    { icon: "warn", label: "Web scrape", detail: "retried once" },
  ]),

  triggers: cap("triggers", "Triggers", "spark", "What starts automations", "6 active triggers", [
    { icon: "calendar", label: "Schedule", detail: "3 time-based" },
    { icon: "book", label: "File change", detail: "2 watchers" },
    { icon: "chat", label: "On message", detail: "1 rule" },
  ], [{ label: "New trigger", icon: "plus" }]),
};
