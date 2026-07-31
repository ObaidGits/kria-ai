/**
 * HomeComposer — KRIA's primary natural-language command surface.
 *
 * It owns only draft/listening UI state. The homepage owns cognition state so a
 * future command runtime can replace the preview callbacks without a redesign.
 */
import { For, Show, createSignal } from "solid-js";
import { CcIcon } from "./CcIcon";
import { currentContext, type CoreState } from "./context";

interface HomeComposerProps {
  state: CoreState;
  onIntent: (value: string) => void;
  onListeningChange: (active: boolean) => void;
}

export function HomeComposer(props: HomeComposerProps) {
  const [draft, setDraft] = createSignal("");
  const [attachments, setAttachments] = createSignal<string[]>([]);
  const [dragging, setDragging] = createSignal(false);
  const [removedContextPills, setRemovedContextPills] = createSignal<string[]>([]);
  let attachmentInput: HTMLInputElement | undefined;
  const listening = () => props.state === "listening";
  const contextPills = () => [
    { id: "project", label: "KRIA" },
    { id: "context", label: currentContext().label },
    { id: "file", label: "CommandCenter.tsx" },
    { id: "memory", label: "Local memory" },
  ].filter((pill) => !removedContextPills().includes(pill.id));
  const removeContextPill = (id: string) => setRemovedContextPills((current) => [...current, id]);

  const addFiles = (files: FileList | null) => {
    if (!files?.length) return;
    setAttachments((current) => [...new Set([...current, ...Array.from(files, (file) => file.name)])].slice(0, 4));
  };
  const removeAttachment = (name: string) => setAttachments((current) => current.filter((item) => item !== name));
  const submit = (event: SubmitEvent) => {
    event.preventDefault();
    const value = draft().trim();
    if (!value) return;
    const context = attachments().length ? ` [Attached: ${attachments().join(", ")}]` : "";
    props.onIntent(`${value}${context}`);
    setDraft("");
    setAttachments([]);
  };

  return (
    <form
      class="cc-command"
      classList={{ "is-dragging": dragging() }}
      aria-label="Ask KRIA"
      onSubmit={submit}
      onDragOver={(event) => { event.preventDefault(); setDragging(true); }}
      onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(event) => { event.preventDefault(); setDragging(false); addFiles(event.dataTransfer?.files ?? null); }}
    >
      <Show when={dragging()}>
        <div class="cc-command__drop" aria-hidden="true"><CcIcon name="plus" size={16} />Drop files to add context</div>
      </Show>
      <div class="cc-command__context" aria-label="Active command context">
        <span>Using</span>
        <For each={contextPills()}>
          {(pill) => (
            <button type="button" aria-label={`Remove ${pill.label} context`} onClick={() => removeContextPill(pill.id)}>
              {pill.label}<b aria-hidden="true">×</b>
            </button>
          )}
        </For>
      </div>
      <div class="cc-composer" data-state={props.state}>
        <span class="cc-composer__mode" aria-hidden="true"><CcIcon name="spark" size={16} /></span>
        <input
          id="cc-command-input"
          type="text"
          class="cc-composer__field"
          value={draft()}
          onInput={(event) => setDraft(event.currentTarget.value)}
          placeholder="Ask KRIA anything…"
          aria-label="Ask KRIA anything or enter a slash command"
          autocomplete="off"
        />
        <kbd class="cc-composer__kbd">⌘K</kbd>
        <input
          ref={attachmentInput}
          class="cc-composer__file"
          type="file"
          multiple
          tabindex="-1"
          aria-hidden="true"
          onChange={(event) => { addFiles(event.currentTarget.files); event.currentTarget.value = ""; }}
        />
        <button type="button" class="cc-composer__attach" aria-label="Attach files" title="Attach files" onClick={() => attachmentInput?.click()}>
          <CcIcon name="plus" size={16} />
        </button>
        <button
          type="button"
          class="cc-composer__mic"
          classList={{ "is-listening": listening() }}
          aria-label={listening() ? "Stop listening" : "Speak to KRIA"}
          aria-pressed={listening()}
          onClick={() => props.onListeningChange(!listening())}
        >
          <CcIcon name="mic" size={18} />
        </button>
        <button type="submit" class="cc-composer__send" aria-label="Send to KRIA" disabled={!draft().trim()}>
          <CcIcon name="send" size={17} />
        </button>
      </div>
      <Show when={attachments().length > 0}>
        <div class="cc-command__attachments" aria-label="Attached files">
          <For each={attachments()}>
            {(name) => <button type="button" title={`Remove ${name}`} onClick={() => removeAttachment(name)}><CcIcon name="brief" size={12} /><span>{name}</span><b aria-hidden="true">×</b></button>}
          </For>
        </div>
      </Show>
      <div class="cc-command__meta" aria-hidden="true">
        <span>Natural language</span><span>/ commands</span><span>Drag files</span><span>Local context</span>
      </div>
    </form>
  );
}

export default HomeComposer;
