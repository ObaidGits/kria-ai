/**
 * Ask / Change dispatch (Req 2.2, architecture invariant).
 *
 * These are the two *free-text* palette modes. Crucially, neither one executes
 * a tool/capability directly:
 *
 *   • Ask   → stage the text into the Converse composer, switch to Converse,
 *             and emit `palette:ask-submitted`. The normal Converse send
 *             pipeline (Intent→Capability→Policy→…) is what actually acts. The
 *             palette only *requests a message send* — it is not a prompt→tool
 *             shortcut.
 *
 *   • Change → stage the natural-language request into the Settings search and
 *             emit `palette:change-submitted`, switching to Settings. If the
 *             backend NL-change command is not wired yet, the intent is staged
 *             and Settings is opened so the user completes it — we never fake a
 *             toggle (Req 10.6).
 *
 * Handlers are injectable so the shell can override them (and tests can spy),
 * but the defaults implement the invariant-safe behaviour above.
 */
import { converseStore, settingsStore, eventBus } from "../stores";
import { navigate } from "../shell/router";

export type TextModeHandler = (text: string) => void;

function defaultAsk(text: string): void {
  const trimmed = text.trim();
  if (!trimmed) return;
  // Stage into the Converse composer (assistant mode) and switch there.
  converseStore.updateDraft({ text: trimmed, mode: "assistant" });
  navigate("converse");
  // Request a send through the normal pipeline — NOT a direct tool call.
  eventBus.emit("palette:ask-submitted", { text: trimmed });
}

function defaultChange(text: string): void {
  const trimmed = text.trim();
  if (!trimmed) return;
  // Stage in the dedicated NL-change bar. Settings submits it only through
  // `config_prompt`, preserving config policy/approval/verification authority.
  settingsStore.stageNaturalLanguageChange(trimmed);
  navigate("settings");
  // Route to the settings NL-change path (no faked toggle).
  eventBus.emit("palette:change-submitted", { text: trimmed });
}

let askHandler: TextModeHandler = defaultAsk;
let changeHandler: TextModeHandler = defaultChange;

/** Override the Ask handler (shell wiring / tests). Pass null to reset. */
export function setAskHandler(handler: TextModeHandler | null): void {
  askHandler = handler ?? defaultAsk;
}

/** Override the Change handler (shell wiring / tests). Pass null to reset. */
export function setChangeHandler(handler: TextModeHandler | null): void {
  changeHandler = handler ?? defaultChange;
}

export function dispatchAsk(text: string): void {
  askHandler(text);
}

export function dispatchChange(text: string): void {
  changeHandler(text);
}
