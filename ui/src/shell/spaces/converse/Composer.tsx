/**
 * Composer — the sticky Converse input (task 3.4, Req 4.4 / 4.5 / 4.9).
 *
 * Responsibilities:
 *   • Auto-growing textarea that grows to a MAX then scrolls internally
 *     (grow-then-scroll, Req 4.4). Growth is deterministic (line-count → rows,
 *     capped at MAX_ROWS); past the cap the control scrolls (CSS overflow).
 *   • Attachments — reads selected file bytes into bounded in-memory draft
 *     payloads, previews/removes them as chips, then dispatches through real
 *     document/image/audio backend commands on Send.
 *   • Mode chip — Assistant ⇆ Lab (tool-locked). Lab is a MODE OF THE THREAD
 *     (Req 4.9), persisted per thread via the draft (Req 4.5), not a hidden
 *     environment. Icon + text so it is never color-only (Req 17.3).
 *   • Voice-entry button — triggers the existing voice-start path only (the full
 *     VoiceSurface is task 5.x); this is the entry affordance, not the pipeline.
 *   • A SINGLE primary action — Send when idle, becoming a prominent Stop while
 *     KRIA works (coreStore.isActive(), Req 4.4). Send is disabled when empty;
 *     Stop is always enabled while working.
 *   • Enter sends; Shift+Enter inserts a newline.
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * The Composer NEVER shortcuts prompt→tool. Send calls
 * `converseStore.sendMessage`, which routes through the EXISTING converse send
 * commands (Intent→Capability→Policy). Lab/tool-lock is a mode flag selecting
 * which existing command runs (`send_lab_message` constrains capability
 * selection server-side) — the Composer does not lock or run tools itself. Stop
 * calls `converseStore.stopTurn`, which uses the existing `cancel_turn`
 * cancellation command (propagation preserved). The `on*` props exist only so
 * stories/tests can inject stubs; production uses the store defaults.
 *
 * Requirements: 4.4, 4.5, 4.9, 17.1, 17.2, 17.3
 */
import { createMemo, createUniqueId, For, Show } from "solid-js";
import { converseStore, coreStore, voiceStore } from "../../../stores";
import { bridgeInvokeOptional } from "../../../bridge/invoke";
import { Button, Chip, IconButton } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { ComposerDraft } from "../../../stores/converseStore";
import "../../../kit/field.css";
import "./Composer.css";

const MIN_ROWS = 1;
const MAX_ROWS = 8; // grow to this many rows, then scroll internally (Req 4.4)

/**
 * Default voice-entry: activate the voice store surface and ask the backend to
 * start voice via the EXISTING optional command. Optional → silently degrades
 * if voice isn't available on this system (Req 18.2 / 20.4).
 */
function startVoiceEntry(): void {
  voiceStore.activate();
  void bridgeInvokeOptional("start_voice");
}

export interface ComposerProps {
  /** Send handler (defaults to the store's pipeline send). Tests/stories only. */
  onSend?: () => void;
  /** Stop handler (defaults to the store's cancellation). Tests/stories only. */
  onStop?: () => void;
  /** Voice-entry handler (defaults to the existing voice-start path). */
  onVoiceStart?: () => void;
}

export function Composer(props: ComposerProps) {
  const draft = () => converseStore.composerDraft();
  const text = () => draft().text;
  const attachments = () => draft().attachments;
  const mode = () => draft().mode;
  const isLab = () => mode() === "lab" || mode() === "tool-lock";

  // Working = the Core is active → the single Send becomes a prominent Stop.
  const working = () => coreStore.isActive();
  const canSend = () => text().trim().length > 0 || attachments().length > 0;

  // Grow-then-scroll (Req 4.4): rows track the line count up to MAX_ROWS; past
  // the cap the textarea keeps MAX_ROWS and scrolls internally (see CSS).
  const rows = createMemo(() => {
    const lines = text().length === 0 ? 1 : text().split("\n").length;
    return Math.min(Math.max(MIN_ROWS, lines), MAX_ROWS);
  });

  const send = () => (props.onSend ?? (() => void converseStore.sendMessage()))();
  const stop = () => (props.onStop ?? (() => void converseStore.stopTurn()))();
  const voiceStart = () => (props.onVoiceStart ?? startVoiceEntry)();

  function onKeyDown(e: KeyboardEvent): void {
    // Enter sends; Shift+Enter newlines. Ignore while composing (IME).
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      if (canSend() && !working()) send();
    }
  }

  function toggleMode(): void {
    // Assistant ⇆ Lab (tool-locked). Persisted per thread via the draft.
    const next: ComposerDraft["mode"] = isLab() ? "assistant" : "lab";
    converseStore.updateDraft({ mode: next });
  }

  const fieldId = createUniqueId();
  let fileInput: HTMLInputElement | undefined;

  function onFilesPicked(e: Event): void {
    const input = e.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    if (files.length > 0) void converseStore.addAttachments(files);
    input.value = ""; // allow re-picking the same file
  }

  function removeAttachment(attachmentId: string): void {
    converseStore.removeAttachment(attachmentId);
  }

  return (
    <div class="kria-composer" data-mode={mode()} data-working={working() ? "true" : "false"}>
      {/* ── Attachment previews (add/remove chips) ────────────────────────── */}
      <Show when={attachments().length > 0}>
        <ul class="kria-composer__attachments" aria-label="Attachments">
          <For each={attachments()}>
            {(attachment) => (
              <li>
                <Chip
                  onRemove={() => removeAttachment(attachment.id)}
                  removeLabel={`Remove ${attachment.name}`}
                >
                  <Icon name="file" size={13} />
                  <span class="kria-composer__attachment-name">{attachment.name}</span>
                </Chip>
              </li>
            )}
          </For>
        </ul>
      </Show>

      {/* ── Textarea (grow-then-scroll) ───────────────────────────────────── */}
      {/* Native textarea (controlled) for deterministic grow-then-scroll: rows
          track the line count up to MAX_ROWS, then CSS caps + scrolls. Styled
          with the kit field classes for visual parity. Labelled (Req 17.2). */}
      <label class="kit-visually-hidden" for={fieldId}>
        Message KRIA
      </label>
      <textarea
        id={fieldId}
        class="kit-field__control kit-field__textarea kria-composer__textarea"
        placeholder="Message KRIA…"
        rows={rows()}
        value={text()}
        aria-keyshortcuts="Enter"
        enterkeyhint="send"
        onInput={(e) => converseStore.updateDraft({ text: e.currentTarget.value })}
        onKeyDown={onKeyDown}
      />

      {/* ── Controls row ──────────────────────────────────────────────────── */}
      <div class="kria-composer__controls">
        <div class="kria-composer__tools">
          {/* Mode chip — Assistant ⇆ Lab (tool-locked), Req 4.9. */}
          <Chip
            selected={isLab()}
            onToggle={toggleMode}
            class="kria-composer__mode"
          >
            <Icon name={isLab() ? "sparkles" : "message-circle"} size={13} />
            <span>{isLab() ? "Lab" : "Assistant"}</span>
          </Chip>

          {/* Attach */}
          <IconButton
            icon="file"
            label="Attach a file"
            size="sm"
            onClick={() => fileInput?.click()}
          />
          <input
            ref={fileInput}
            type="file"
            multiple
            class="kria-composer__file-input"
            aria-hidden="true"
            tabindex={-1}
            onChange={onFilesPicked}
          />

          {/* Voice entry (affordance only — full surface is task 5.x). */}
          <IconButton
            icon="mic"
            label="Start voice input"
            size="sm"
            onClick={voiceStart}
          />
        </div>

        {/* SINGLE primary action — Send ⇄ Stop (Req 4.4). */}
        <Show
          when={working()}
          fallback={
            <Button
              variant="primary"
              class="kria-composer__send"
              disabled={!canSend()}
              aria-label="Send message"
              onClick={() => canSend() && send()}
            >
              <Icon name="send" size={14} />
              <span>Send</span>
            </Button>
          }
        >
          <Button
            variant="danger"
            class="kria-composer__stop"
            aria-label="Stop"
            onClick={stop}
          >
            <Icon name="square" size={14} />
            <span>Stop</span>
          </Button>
        </Show>
      </div>
    </div>
  );
}

export default Composer;
