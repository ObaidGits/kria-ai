/**
 * Per-message action definitions (Req 4.8): the SIX actions copy, retry,
 * explain, remember, branch, feedback. Built once here so the right-click
 * ContextMenu and the keyboard-reachable actions Menu render the SAME set
 * (Req 21.2). Feedback carries a good/bad submenu so it stays one top-level
 * action while still capturing sentiment.
 *
 * KRIA runtime-authority invariant: every action except `copy` dispatches
 * through `converseStore` action creators into canonical backend commands.
 * None run tools directly or use presentation-only request events.
 * `copy` is the only purely-local action (clipboard).
 */
import { converseStore, shellStore, type Message } from "../../../stores";
import { navigate } from "../../router";
import { announceCopyOutcome } from "./copyAnnouncer";

export interface MessageAction {
  id: string;
  label: string;
  icon: string;
  /** Leaf action. Absent when the action only groups `children`. */
  run?: () => void;
  /** Nested actions (e.g. feedback → good/bad). */
  children?: MessageAction[];
}

/**
 * Write text to the clipboard (local UI action). Guarded for jsdom/test.
 * Returns whether the write succeeded so callers can surface success/failure
 * (Req 12.3). A missing Clipboard API or a rejected write both resolve to
 * `false` — the copy did not happen, so it is a failure outcome, not success.
 */
export async function copyToClipboard(text: string): Promise<boolean> {
  const clipboard = navigator.clipboard;
  if (!clipboard) return false;
  try {
    await clipboard.writeText(text);
    return true;
  } catch {
    /* clipboard denied/unavailable — reported as a failure outcome. */
    return false;
  }
}

/**
 * Copy a message's content and announce the concise outcome to the polite
 * copy-status region without moving focus (Req 12.3, 12.5; UIE-M-009). Rapid
 * identical outcomes are deduplicated by the announcer.
 */
export async function copyMessageContent(text: string): Promise<void> {
  const ok = await copyToClipboard(text);
  announceCopyOutcome(ok ? "success" : "failure");
}

/**
 * The primary memory the runtime used to produce an answer, if any (Req 5.7).
 * Returns the first non-empty id in relevance order; `undefined` when the
 * message carries no memory provenance (→ the "why" affordance is hidden).
 */
export function primaryUsedMemoryId(message: Message): string | undefined {
  return message.usedMemoryIds?.find((id) => typeof id === "string" && id.length > 0);
}

/**
 * "Why did KRIA answer this?" (Req 5.7). Deep-links to the relevant memory in
 * the Memory Space and opens the shared Inspector on it, so the user can see
 * the provenance behind an answer.
 *
 * Navigation-only, and honest: it reads the provenance the runtime already
 * attached to the message and, if a memory is present, routes to
 * `memory/explorer/<memoryId>` AND opens the shared Inspector on that id
 * (`shellStore.openInspector`). It performs NO orchestration and NO prompt→tool
 * shortcut (KRIA runtime-authority invariant). No-op when there is no memory
 * provenance (the affordance isn't offered in that case).
 */
export function whyDidKriaAnswer(message: Message): void {
  const memoryId = primaryUsedMemoryId(message);
  if (!memoryId) return;
  // Deep-link the Memory Space to the relevant memory (MemorySpace honors the
  // routed entityId to open the Inspector), and open the shared Inspector now
  // so the detail is visible immediately.
  navigate("memory", "explorer", memoryId);
  // This action ALSO changes route, so the invoking Converse control unmounts.
  // Hand the stable primary-workspace landmark as the Focus_Return_Owner so a
  // later close resolves via the §20.4 ladder to a stable region, not a stray
  // element (task 9.3; target-removal specifics are task 9.4).
  shellStore.openInspector("memory", memoryId, undefined, { regionSelector: "#space-root" });
}

/**
 * Build the ordered per-message action list. The SIX base actions are always
 * present; the "Why did KRIA answer this?" affordance (Req 5.7) is appended
 * ONLY for assistant messages that carry memory provenance — so it is never a
 * dead/fake link.
 */
export function buildMessageActions(message: Message): MessageAction[] {
  const actions: MessageAction[] = [
    { id: "copy", label: "Copy", icon: "copy", run: () => void copyMessageContent(message.content) },
    { id: "retry", label: "Retry", icon: "refresh-cw", run: () => void converseStore.retryMessage(message.id) },
    { id: "explain", label: "Explain", icon: "lightbulb", run: () => void converseStore.explainMessage(message.id) },
    { id: "remember", label: "Remember", icon: "brain", run: () => void converseStore.rememberMessage(message.id) },
    { id: "branch", label: "Branch", icon: "git-branch", run: () => void converseStore.branchMessage(message.id) },
    {
      id: "feedback",
      label: "Feedback",
      icon: "message-circle",
      children: [
        { id: "feedback-up", label: "Good response", icon: "star", run: () => void converseStore.submitFeedback(message.id, "up") },
        { id: "feedback-down", label: "Poor response", icon: "alert-triangle", run: () => void converseStore.submitFeedback(message.id, "down") },
      ],
    },
  ];

  // Deep-link provenance affordance (Req 5.7) — assistant answers with memory.
  if (message.role === "assistant" && primaryUsedMemoryId(message)) {
    actions.push({
      id: "why",
      label: "Why did KRIA answer this?",
      icon: "circle-help",
      run: () => whyDidKriaAnswer(message),
    });
  }

  // Edit is offered ONLY on the user's own messages, and is placed after the base
  // six so their positions do not shift depending on who sent the message — muscle
  // memory for "second item is Retry" holds either way.
  //
  // Editing KRIA's reply and re-sending it would fabricate a question the user never
  // asked, which is why there is no assistant equivalent.
  if (message.role === "user") {
    actions.push({
      id: "edit",
      label: "Edit and resend",
      icon: "pencil",
      run: () => converseStore.requestMessageEdit(message),
    });
  }

  return actions;
}
