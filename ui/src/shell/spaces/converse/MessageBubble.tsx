/**
 * MessageBubble — a single conversation turn (design.md §6.1).
 *
 *  • Role-styled (user / assistant / system).
 *  • Body rendered from SANITIZED markdown (`renderMarkdown` — marked +
 *    DOMPurify + highlight.js). AI/tool HTML is never rendered raw (security
 *    critical, design.md §1.17).
 *  • AI-provenance cue on assistant/system turns distinguishes KRIA-authored
 *    content from the user's (Req 20.5).
 *  • Inline result cards render tool/result payloads, kept visually secondary
 *    to the reply text (Req 4.3 conversation-dominance).
 *  • Per-message actions (Req 4.8) via right-click (ContextMenu) AND an
 *    always-keyboard-reachable actions button revealed on selection/hover.
 *
 * Pure presentation + action-dispatch: actions route through converseStore
 * (typed event requests), never a prompt→tool shortcut (KRIA invariant).
 */
import { createMemo, For, Show } from "solid-js";
import { Icon } from "../../../components/Icon";
import { Card, ProvenanceCue } from "../../../kit";
import { renderMarkdown, sanitizeHtml } from "../../../lib/markdown";
import type { Message, MessageResult } from "../../../stores";
import { buildMessageActions } from "./messageActions";
import { MessageActionsMenu, MessageContextMenu } from "./MessageActionsMenu";
import "./MessageBubble.css";

const RESULT_ICON: Record<MessageResult["kind"], string> = {
  "tool-result": "terminal",
  memory: "brain",
  document: "file",
  image: "eye",
  custom: "sparkles",
};

function formatTime(ts: number): string {
  try {
    return new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  } catch {
    return "";
  }
}

/**
 * Copy-code handler for the buttons emitted by the markdown renderer. Delegated
 * from the body root so it survives re-render without per-button listeners.
 */
function onBodyClick(event: MouseEvent): void {
  const target = event.target;
  if (!(target instanceof Element)) return;
  const button = target.closest(".kria-md-code__copy") as HTMLButtonElement | null;
  if (!button) return;
  const header = button.closest(".kria-md-code__header");
  const code = header?.nextElementSibling?.querySelector("code");
  const text = code?.textContent ?? "";
  if (!text) return;
  void navigator.clipboard?.writeText(text).then(() => {
    const prev = button.textContent ?? "Copy";
    button.textContent = "Copied";
    window.setTimeout(() => (button.textContent = prev), 1500);
  });
}

export interface MessageBubbleProps {
  message: Message;
  selected?: boolean;
  onSelect?: (id: string) => void;
}

export function MessageBubble(props: MessageBubbleProps) {
  const actions = createMemo(() => buildMessageActions(props.message));
  const isKriaAuthored = createMemo(
    () => props.message.role === "assistant" || props.message.role === "system",
  );
  const bodyHtml = createMemo(() => renderMarkdown(props.message.content));

  const select = () => props.onSelect?.(props.message.id);

  return (
    <MessageContextMenu actions={actions()}>
      <article
        class="kria-msg"
        data-role={props.message.role}
        data-provenance={isKriaAuthored() ? "kria" : "user"}
        data-selected={props.selected ? "true" : undefined}
        aria-label={`${props.message.role} message`}
        tabindex="0"
        onClick={select}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            select();
          }
        }}
      >
        <header class="kria-msg__header">
          <span class="kria-msg__role">
            <ProvenanceCue source={isKriaAuthored() ? "kria" : "user"} />
          </span>
          <time class="kria-msg__time">{formatTime(props.message.timestamp)}</time>
          {/* Keyboard-reachable actions (Req 4.8 / 17.1). */}
          <div class="kria-msg__actions">
            <MessageActionsMenu actions={actions()} />
          </div>
        </header>

        {/* SANITIZED markdown body (security critical). */}
        <div class="kria-msg__body" onClick={onBodyClick} innerHTML={bodyHtml()} />

        {/* Inline result cards — secondary to the reply text (Req 4.3). */}
        <Show when={props.message.results && props.message.results.length > 0}>
          <div class="kria-msg__results" aria-label="Result cards">
            <For each={props.message.results}>
              {(result) => (
                <Card class="kria-msg__result" aria-label={result.title}>
                  <div class="kria-msg__result-head">
                    <Icon name={RESULT_ICON[result.kind]} size={14} />
                    <span class="kria-msg__result-title">{result.title}</span>
                  </div>
                  <Show when={result.summary}>
                    <p class="kria-msg__result-summary">{result.summary}</p>
                  </Show>
                  <Show when={result.html}>
                    {/* Tool/result HTML is untrusted → sanitized before display. */}
                    <div class="kria-msg__result-body" innerHTML={sanitizeHtml(result.html!)} />
                  </Show>
                </Card>
              )}
            </For>
          </div>
        </Show>
      </article>
    </MessageContextMenu>
  );
}

export default MessageBubble;
