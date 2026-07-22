/**
 * KRIA Terminology Matrix — single source of truth (task 7.5; IU-08; UIE-M-016, UIE-M-017).
 *
 * design.md §12: "Threads, Tools, Skills, Integrations, and Lab are concepts or
 * nested surfaces—not top-level Spaces. Explain Machines versus Observatory,
 * Threads versus Memory, Tools versus Skills versus Integrations, Temporary
 * threads, and Lab mode without implying new destinations."
 *
 * This module is the ONE canonical terminology matrix (UIE-M-017). Later
 * sub-tasks (7.6/7.7) surface these entries as concise outcome descriptions at
 * navigation, empty, and decision points — they must READ from here rather than
 * inventing parallel copy, so the distinctions stay consistent across surfaces.
 *
 * Design constraints honored here:
 *   • Read-only presentation data. Owns no runtime lifecycle, no route, no
 *     store. It never changes architecture or the seven Space_Routes.
 *   • Route-vs-concept status is encoded per §12 and enforced by Req 7.11:
 *       - Machines, Observatory, Memory ARE top-level Space_Routes.
 *       - Threads, Tools, Skills, Integrations, Temporary threads, Lab mode are
 *         concepts/surfaces nested WITHIN a Space — never top-level routes.
 *   • Copy is concise and grounded in ACTUAL KRIA capabilities (native tools,
 *     ClawHub/OpenClaw skills, MCP/Google/Colab/Telegram integrations, the
 *     temporary-thread flag, the tool-locked Lab send path). Nothing invented.
 *
 * i18n note: like existing starter/action copy (`groundedStarters.ts`,
 * `messageActions.ts`), these strings are English typed literals rather than
 * locale keys. That matches the current project convention for starter copy.
 * Follow-up: if/when this copy is routed through `ui/src/locales`, swap the
 * literals for i18n keys — the matrix shape (ids + four columns) stays stable.
 */
import type { Space } from "./router";

/**
 * Route/concept status (UIE-M-017, Req 7.11).
 *   • "space-route" — a top-level canonical Space_Route (one of the seven).
 *   • "concept"     — a concept or nested surface within a Space; NOT a route.
 */
export type TermStatus = "space-route" | "concept";

/** The nine terms the matrix must distinguish (Req 7.3–7.6). */
export type TermId =
  | "machines"
  | "observatory"
  | "threads"
  | "memory"
  | "tools"
  | "skills"
  | "integrations"
  | "temporary-threads"
  | "lab-mode";

/**
 * One matrix row: a term distinguished by four columns plus the Space it
 * relates to. For a "space-route" term, `space` is the Space it IS; for a
 * "concept" term, `space` is the Space it lives WITHIN (its home surface).
 */
export interface TerminologyEntry {
  /** Stable identifier (kebab-case). */
  id: TermId;
  /** Human-readable label as shown in the UI. */
  label: string;
  /** Route/concept status (UIE-M-017, Req 7.11). */
  status: TermStatus;
  /** Canonical Space this term is, or the Space it is nested within. */
  space: Space;
  /** Outcome — what it does for the user (concise, outcome-oriented). */
  outcome: string;
  /** Persistence — what persists / for how long. */
  persistence: string;
  /** Authority — the runtime / safety authority it carries. */
  authority: string;
}

/**
 * THE terminology matrix. Ordered to read as the paired distinctions the spec
 * calls out (Machines vs Observatory, Threads vs Memory,
 * Tools vs Skills vs Integrations, then the two consequential modes).
 */
export const TERMINOLOGY_MATRIX: readonly TerminologyEntry[] = [
  // ── Machines vs Observatory (Req 7.3) — both top-level Space_Routes ──
  {
    id: "machines",
    label: "Machines",
    status: "space-route",
    space: "machines",
    outcome: "Run and manage work on your enrolled machines and remote targets.",
    persistence: "Enrolled targets, leases, and health stay until you remove them.",
    authority: "Executes real commands on remote systems under safety policy and approvals.",
  },
  {
    id: "observatory",
    label: "Observatory",
    status: "space-route",
    space: "observatory",
    outcome: "Watch live system activity, telemetry, and KRIA's own operation.",
    persistence: "A read-only view of current signals; keeps no work of its own.",
    authority: "Observation only — it runs no commands and changes no state.",
  },

  // ── Threads vs Memory (Req 7.4) — Threads is a concept in Converse; Memory is a Space ──
  {
    id: "threads",
    label: "Threads",
    status: "concept",
    space: "converse",
    outcome: "Revisit and continue a specific conversation and its history in Converse.",
    persistence: "Saved conversation history you can reopen, pin, or archive.",
    authority: "Uses the active conversation's normal assistant capabilities.",
  },
  {
    id: "memory",
    label: "Memory",
    status: "space-route",
    space: "memory",
    outcome: "Find and manage retained knowledge — facts and indexed documents KRIA recalls.",
    persistence: "Durable across sessions; facts and documents carry decay scoring.",
    authority: "Stores and retrieves knowledge; it runs no tools and contacts no machines.",
  },

  // ── Tools vs Skills vs Integrations (Req 7.5) — all concepts within Capabilities ──
  {
    id: "tools",
    label: "Tools",
    status: "concept",
    space: "capabilities",
    outcome: "KRIA's built-in native actions — files, web, system, and more — that it runs for you.",
    persistence: "Always-available built-in capability; nothing to install per item.",
    authority: "Runs under KRIA orchestration with risk classification and approvals.",
  },
  {
    id: "skills",
    label: "Skills",
    status: "concept",
    space: "capabilities",
    outcome: "Installable ClawHub / OpenClaw abilities that extend what KRIA can do.",
    persistence: "Installed skills stay until removed and carry a trust tier.",
    authority: "Run sandboxed in the OpenClaw substrate under capability grants.",
  },
  {
    id: "integrations",
    label: "Integrations",
    status: "concept",
    space: "capabilities",
    outcome: "Connections to external services — MCP servers, Google, Colab, Telegram.",
    persistence: "Connection state persists per service; each can be unavailable on its own.",
    authority: "Bridges to external systems, scoped by each connection's granted access.",
  },

  // ── Consequential modes (Req 7.6) — concepts within Converse, explained before choosing ──
  {
    id: "temporary-threads",
    label: "Temporary threads",
    status: "concept",
    space: "converse",
    outcome: "Hold a throwaway conversation you don't want kept.",
    persistence: "Not retained as durable history — cleared instead of saved long-term.",
    authority: "Same assistant capabilities as a normal thread while it is active.",
  },
  {
    id: "lab-mode",
    label: "Lab mode",
    status: "concept",
    space: "converse",
    outcome: "Draft and test prompts in a constrained Composer mode.",
    persistence: "Sends through the existing Lab path; adds no special long-term store.",
    authority: "Tool-locked — capabilities are restricted so it cannot freely invoke tools.",
  },
] as const;

/** All term ids the matrix must cover (Req 7.3–7.6), for coverage checks. */
export const REQUIRED_TERM_IDS: readonly TermId[] = [
  "machines",
  "observatory",
  "threads",
  "memory",
  "tools",
  "skills",
  "integrations",
  "temporary-threads",
  "lab-mode",
] as const;

/** Look up a single term by id (used by navigation/empty/decision surfaces). */
export function getTerm(id: TermId): TerminologyEntry {
  const entry = TERMINOLOGY_MATRIX.find((t) => t.id === id);
  if (!entry) {
    // Unreachable given the typed id/matrix pairing; guards accidental drift.
    throw new Error(`Unknown terminology id: ${id}`);
  }
  return entry;
}

/** True when the term is a top-level canonical Space_Route (Req 7.11). */
export function isSpaceRouteTerm(id: TermId): boolean {
  return getTerm(id).status === "space-route";
}
