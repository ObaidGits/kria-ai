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
 * ── Primary idle task-entry (UIE-H-001, Req 5.1) ────────────────────────────
 * The root carries `data-primary-entry="true"` and a bordered, raised surface
 * so the Composer is the unmistakable primary Homepage entry point, dominating
 * the reduced-weight command-palette trigger in the PresenceBar. This is a
 * VISUAL hierarchy marker only — send/staging behavior and the runtime-authority
 * invariant above are unchanged.
 *
 * Requirements: 4.4, 4.5, 4.9, 5.1, 17.1, 17.2, 17.3
 */
import { createMemo, createUniqueId, For, Show } from "solid-js";
import { converseStore, coreStore, voiceStore } from "../../../stores";
import { bridgeInvokeOptional } from "../../../bridge/invoke";
import { Button, Chip, IconButton, type MenuItem } from "../../../kit";
import { OverflowControl } from "../../OverflowControl";
import { controlTier, partitionControls, type TieredControl } from "../../controlPriority";
import type { WidthProfile } from "../converseComposition";
import { Icon } from "../../../components/Icon";
import type { ComposerDraft } from "../../../stores/converseStore";
import { getTerm } from "../../terminology";
import "../../../kit/field.css";
import "./Composer.css";

/**
 * Concise Lab-mode outcome read from the terminology matrix (single source of
 * truth, task 7.5). Surfaced as the mode chip's description at this decision
 * point (Assistant ⇆ Lab) so the distinction is explained before choosing,
 * without re-authoring copy here (Req 7.6, 7.7).
 */
const LAB_MODE_OUTCOME = getTerm("lab-mode").outcome;

const MIN_ROWS = 1;
const MAX_ROWS = 8; // grow to this many rows, then scroll internally (Req 4.4)

/**
 * Composer tool-cluster inline capacity per Width Profile (task 8.6, UIE-M-003).
 *
 * Send⇄Stop is CRITICAL and is rendered OUTSIDE this partition — always inline
 * and full-size at every profile. Only the primary tools (mode chip, attach,
 * voice) are partitioned:
 *
 *   • focus → capacity 1: the mode chip stays inline; Attach + Voice move into
 *     ONE labelled disclosure (OverflowControl) — preserved, never dropped
 *     (design §11.5 "preserve attachment/voice"). No free-wrap → bounded height.
 *   • dual / assisted / full → capacity 3: all tools inline.
 */
const COMPOSER_TOOLS_CAPACITY: Readonly<Record<WidthProfile, number>> = {
  focus: 1,
  dual: 3,
  assisted: 3,
  full: 3,
};

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
  /**
   * Active Width Profile (from ConverseSpace). Drives which primary tools stay
   * inline vs. collapse into the labelled disclosure (task 8.6, UIE-M-003).
   * Defaults to "full" so standalone/story/test usage shows every tool inline.
   */
  widthProfile?: WidthProfile;
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

  // ── Tool cluster inline-vs-overflow by Width Profile (task 8.6, UIE-M-003) ──
  // Only the primary tools are partitioned; Send⇄Stop (critical) is rendered
  // separately and never overflows. Attach + Voice, when collapsed, remain
  // reachable through the labelled OverflowControl (preserved, never dropped).
  const composerToolProfile = (): WidthProfile => props.widthProfile ?? "full";
  const composerTools = createMemo<TieredControl[]>(() =>
    ["mode-chip", "attach", "voice"].map((id) => ({ id, tier: controlTier(id)!, label: id })),
  );
  const composerPartition = createMemo(() =>
    partitionControls(composerTools(), COMPOSER_TOOLS_CAPACITY[composerToolProfile()]),
  );
  const toolsInline = createMemo(() => new Set(composerPartition().inline.map((c) => c.id)));
  const composerOverflowItems = createMemo<MenuItem[]>(() => {
    const items: MenuItem[] = [];
    for (const control of composerPartition().overflow) {
      if (control.id === "attach") {
        items.push({ id: "attach", label: "Attach a file", icon: "file", onSelect: () => fileInput?.click() });
      } else if (control.id === "voice") {
        items.push({ id: "voice", label: "Start voice input", icon: "mic", onSelect: () => voiceStart() });
      }
    }
    return items;
  });

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
    <div
      class="kria-composer"
      data-primary-entry="true"
      data-mode={mode()}
      data-working={working() ? "true" : "false"}
    >
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
        aria-label="Message KRIA"
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
          {/* Mode chip — Assistant ⇆ Lab (tool-locked), Req 4.9. Primary tool;
              stays inline at every profile (seated first by capacity). */}
          <Show when={toolsInline().has("mode-chip")}>
            <Chip
              selected={isLab()}
              onToggle={toggleMode}
              title={`Lab mode: ${LAB_MODE_OUTCOME}`}
              class="kria-composer__mode"
            >
              <Icon name={isLab() ? "sparkles" : "message-circle"} size={13} />
              <span>{isLab() ? "Lab" : "Assistant"}</span>
            </Chip>
          </Show>

          {/* Attach — inline where it fits; otherwise reachable via the
              disclosure below (never dropped, §11.5). */}
          <Show when={toolsInline().has("attach")}>
            <IconButton
              icon="file"
              label="Attach a file"
              size="sm"
              onClick={() => fileInput?.click()}
            />
          </Show>
          {/* Hidden native file input is ALWAYS mounted so the disclosure's
              "Attach a file" item can trigger it while collapsed. */}
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
          <Show when={toolsInline().has("voice")}>
            <IconButton
              icon="mic"
              label="Start voice input"
              size="sm"
              onClick={voiceStart}
            />
          </Show>

          {/* ONE labelled disclosure for collapsed tools (narrowest profile).
              Preserves attachment/voice reachability without free-wrap. */}
          <Show when={composerOverflowItems().length > 0}>
            <OverflowControl label="More composer actions" items={composerOverflowItems()} />
          </Show>
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
            aria-label="Stop response"
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
