import { Component, For, Show, createEffect, createSignal, createMemo, onCleanup, untrack } from "solid-js";
import { appStore } from "../stores/app";
import MessageBubble from "./MessageBubble";
import ExportDropdown from "./ExportDropdown";
import ImageProgressChip from "./ImageProgressChip";

interface SlashCmd {
  name: string;
  desc: string;
  action: (args: string) => void;
}

const ChatView: Component = () => {
  let messagesEnd: HTMLDivElement | undefined;
  let fileInput: HTMLInputElement | undefined;
  let textareaRef: HTMLTextAreaElement | undefined;
  const {
    messages,
    isThinking,
    inputText,
    setInputText,
    sendMessage,
    sendImageMessage,
    sendDocumentMessage,
    transcribeUploadedAudio,
    pendingFiles,
    addPendingFile,
    removePendingFile,
    clearPendingFiles,
    cancelTurn,
    toggleVoice,
    voiceActive,
    voiceState,
    toolChoiceRequest,
    submitToolChoice,
    dismissToolChoice,
    currentSession,
    sessions,
    isSwapping,
    degradationLevel,
  } = appStore;

  // Derive the title of the current session for exports
  const currentSessionTitle = () => {
    const id = currentSession();
    if (!id) return null;
    return sessions().find((s) => s.id === id)?.title ?? null;
  };

  const [pendingImage, setPendingImage] = createSignal<{ data: Uint8Array; mime: string; preview: string } | null>(null);
  const [isDragOver, setIsDragOver] = createSignal(false);
  const [showSlash, setShowSlash] = createSignal(false);
  const [slashIndex, setSlashIndex] = createSignal(0);

  const clearPendingImage = () => {
    const img = pendingImage();
    if (img) URL.revokeObjectURL(img.preview);
    setPendingImage(null);
    if (fileInput) fileInput.value = "";
  };

  const isDocumentFile = (file: File): boolean => {
    if (file.type.startsWith("image/")) {
      // True images stay in the image pipeline UNLESS the extension says otherwise
      return !!file.name.match(/\.(pdf|docx|xlsx|pptx|txt|md|csv|json|yaml|yml|toml|py|rs|ts|js|ipynb|log|html|xml)$/i);
    }
    // Everything that isn't image/* goes to the document pipeline
    return true;
  };

  const isAudioFile = (file: File): boolean => {
    if (file.type.startsWith("audio/")) return true;
    return !!file.name.match(/\.(wav|mp3|m4a|flac|ogg|webm)$/i);
  };

  const fileTypeIcon = (mime: string, name: string): string => {
    if (mime === "application/pdf" || name.endsWith(".pdf")) return "📄";
    if (mime.includes("spreadsheet") || name.match(/\.(xlsx|xls|csv)$/i)) return "📊";
    if (mime.includes("presentation") || name.match(/\.(pptx|ppt)$/i)) return "📑";
    if (mime.includes("wordprocessing") || name.match(/\.(docx|doc)$/i)) return "📝";
    if (name.match(/\.(py|rs|ts|js|go|java|c|cpp|h|rb|php|kt|swift|lua|sh|sql|r)$/i)) return "💻";
    if (name.endsWith(".ipynb")) return "📓";
    if (name.match(/\.(json|yaml|yml|toml|xml)$/i)) return "⚙️";
    if (name.match(/\.(md|markdown|txt|log)$/i)) return "📃";
    if (mime.startsWith("audio/") || name.match(/\.(wav|mp3|m4a|flac|ogg|webm)$/i)) return "🎙️";
    return "📎";
  };

  const formatFileSize = (bytes: number): string => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  };

  const slashCommands: SlashCmd[] = [
    { name: "/clear", desc: "Clear current messages", action: () => { /* handled in store if needed */ sendMessage("/clear"); } },
    { name: "/session", desc: "Create a new session", action: () => { appStore.createSession(); } },
    { name: "/voice", desc: "Toggle voice input", action: () => { toggleVoice(); } },
    { name: "/settings", desc: "Open settings", action: () => { appStore.setShowSettings(true); } },
  ];

  const filteredSlash = createMemo(() => {
    const text = inputText();
    if (!text.startsWith("/")) return [];
    const query = text.toLowerCase();
    return slashCommands.filter((c) => c.name.startsWith(query));
  });

  // Show slash menu when typing /
  createEffect(() => {
    const cmds = filteredSlash();
    setShowSlash(cmds.length > 0 && inputText().startsWith("/"));
    setSlashIndex(0);
  });

  // Auto-scroll to bottom on new messages
  createEffect(() => {
    messages(); // track
    messagesEnd?.scrollIntoView({ behavior: "smooth" });
  });

  // Reset pending image when session changes to avoid stale preview/input state.
  createEffect(() => {
    currentSession();
    // Avoid tracking `pendingImage` here; otherwise selecting an image retriggers this
    // effect and clears the preview immediately.
    untrack(() => clearPendingImage());
  });

  onCleanup(() => {
    clearPendingImage();
  });

  // Auto-grow textarea
  const autoGrow = () => {
    if (textareaRef) {
      textareaRef.style.height = "auto";
      textareaRef.style.height = Math.min(textareaRef.scrollHeight, 150) + "px";
    }
  };

  const executeSlash = (cmd: SlashCmd) => {
    const args = inputText().slice(cmd.name.length).trim();
    cmd.action(args);
    setInputText("");
    setShowSlash(false);
    if (textareaRef) {
      textareaRef.style.height = "auto";
    }
  };

  const handleSubmit = (e: Event) => {
    e.preventDefault();
    if (showSlash() && filteredSlash().length > 0) {
      executeSlash(filteredSlash()[slashIndex()]);
      return;
    }
    const files = pendingFiles();
    const img = pendingImage();
    if (files.length > 0) {
      const snapshot = [...files];
      sendDocumentMessage(snapshot, inputText() || undefined);
    } else if (img) {
      const data = img.data;
      const mime = img.mime;
      clearPendingImage();
      sendImageMessage(data, mime, inputText() || undefined);
    } else {
      sendMessage(inputText());
    }
    if (textareaRef) textareaRef.style.height = "auto";
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (showSlash() && filteredSlash().length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSlashIndex((i) => Math.min(i + 1, filteredSlash().length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSlashIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        executeSlash(filteredSlash()[slashIndex()]);
        return;
      }
      if (e.key === "Escape") {
        setShowSlash(false);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      const files = pendingFiles();
      const img = pendingImage();
      if (files.length > 0) {
        const snapshot = [...files];
        sendDocumentMessage(snapshot, inputText() || undefined);
      } else if (img) {
        const data = img.data;
        const mime = img.mime;
        clearPendingImage();
        sendImageMessage(data, mime, inputText() || undefined);
      } else {
        sendMessage(inputText());
      }
      if (textareaRef) textareaRef.style.height = "auto";
    }
  };

  const processFile = async (file: File) => {
    if (isAudioFile(file)) {
      await transcribeUploadedAudio(file);
      return;
    }
    // Route to document pipeline for non-image or explicit doc extensions
    if (isDocumentFile(file)) {
      addPendingFile(file);
      return;
    }
    // Image path — existing flow
    const previous = pendingImage();
    if (previous) URL.revokeObjectURL(previous.preview);
    const buffer = await file.arrayBuffer();
    const data = new Uint8Array(buffer);
    const preview = URL.createObjectURL(file);
    setPendingImage({ data, mime: file.type, preview });
    if (fileInput) fileInput.value = "";
  };

  const handlePaste = async (e: ClipboardEvent) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith("image/")) {
        e.preventDefault();
        const file = item.getAsFile();
        if (file) await processFile(file);
        return;
      }
    }
  };

  const handleDrop = async (e: DragEvent) => {
    e.preventDefault();
    setIsDragOver(false);
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      await Promise.all(Array.from(files).map(processFile));
    }
  };

  return (
    <div
      class={`chat-view ${isDragOver() ? "drag-over" : ""}`}
      onDragOver={(e) => { e.preventDefault(); setIsDragOver(true); }}
      onDragLeave={() => setIsDragOver(false)}
      onDrop={handleDrop}
    >
      <div class="chat-toolbar">
        <span class="chat-toolbar-title">
          {currentSessionTitle() ?? "New conversation"}
        </span>
        <ExportDropdown messages={messages} sessionTitle={currentSessionTitle} />
      </div>

      <Show when={isSwapping()}>
        <div class="gpu-swap-alert" role="status" aria-live="polite">
          <span class="dot" /><span class="dot" /><span class="dot" />
          <span class="swap-label">Optimizing GPU layers...</span>
        </div>
      </Show>

      <div class="chat-messages">
        <Show when={messages().length === 0 && !isThinking()}>
          <div class="assistant-welcome-card">
            <div class="assistant-welcome-eyebrow">Personal Mission Control</div>
            <h2>How can I help you today?</h2>
            <p>Type a message, attach a file, or use the 🎤 button to speak.</p>
          </div>
        </Show>

        <For each={messages()}>
          {(msg, i) => {
            const prevUserText = () => {
              const list = messages();
              for (let j = i() - 1; j >= 0; j--) {
                if (list[j].role === "user") return list[j].content;
              }
              return "";
            };
            return (
              <MessageBubble
                message={msg}
                sessionId={currentSession() ?? ""}
                userText={prevUserText()}
              />
            );
          }}
        </For>

        {isThinking() && (
          <div class="thinking-row">
            <div class="thinking-avatar">K</div>
            <div class="thinking-bubble">
              <span class="dot" /><span class="dot" /><span class="dot" />
            </div>
          </div>
        )}

        <ImageProgressChip />

        <div ref={messagesEnd} />
      </div>

      <form class="chat-input-form" onSubmit={handleSubmit}>
        <Show when={degradationLevel && degradationLevel() === "critical"}>
          <div class="degradation-banner">⚠️ Operating in reduced capacity mode.</div>
        </Show>

        {/* File chips (pending document attachments) */}
        <Show when={pendingFiles().length > 0}>
          <div class="file-chips-bar">
            <For each={pendingFiles()}>
              {(pf, i) => (
                <div class="file-chip">
                  <span class="file-chip-icon">{fileTypeIcon(pf.mime, pf.name)}</span>
                  <span class="file-chip-name" title={pf.name}>{pf.name}</span>
                  <span class="file-chip-size">{formatFileSize(pf.size)}</span>
                  <button
                    type="button"
                    class="file-chip-remove"
                    onClick={() => removePendingFile(i())}
                    title={`Remove ${pf.name}`}
                  >✕</button>
                </div>
              )}
            </For>
          </div>
        </Show>

        {/* Image preview (pending image attachment) */}
        <Show when={pendingImage()}>
          <div class="image-preview-bar">
            <img
              src={pendingImage()!.preview}
              alt="Pending upload"
              class="image-preview-thumb"
            />
            <span class="image-preview-label">Image attached</span>
            <button
              type="button"
              class="image-preview-remove"
              onClick={clearPendingImage}
            >✕</button>
          </div>
        </Show>

        {/* Slash command menu */}
        <Show when={showSlash() && filteredSlash().length > 0}>
          <div class="slash-menu">
            {filteredSlash().map((cmd, i) => (
              <div
                class={`slash-command-item ${i === slashIndex() ? "selected" : ""}`}
                onClick={() => executeSlash(cmd)}
              >
                <span class="slash-cmd-name">{cmd.name}</span>
                <span class="slash-cmd-desc">{cmd.desc}</span>
              </div>
            ))}
          </div>
        </Show>

        <div class="input-row">
          <button
            type="button"
            class={`voice-btn ${voiceActive() ? "active" : ""} ${voiceActive() ? `voice-state-${voiceState()}` : ""}`}
            onClick={() => toggleVoice()}
            title={voiceActive() ? `Voice: ${voiceState()}` : "Toggle voice input"}
          >
            {voiceState() === "speaking" ? "🔊" : "🎤"}
          </button>

          <label class="attach-btn" title="Attach file or image" role="button" tabIndex={0}>
            📎
            <input
              ref={fileInput}
              type="file"
              multiple
              style={{ display: "none" }}
              onChange={async (e) => {
                const files = e.currentTarget.files;
                if (files) await Promise.all(Array.from(files).map(processFile));
                e.currentTarget.value = "";
              }}
            />
          </label>

          <textarea
            ref={textareaRef}
            class="chat-input"
            placeholder={
              isSwapping()
                ? "Model is swapping GPU layers…"
                : pendingFiles().length > 0
                ? "Add a message about your files, or press Send…"
                : pendingImage()
                ? "Describe what you want to know about this image…"
                : "Ask KRIA anything… (type / for commands)"
            }
            value={inputText()}
            onInput={(e) => {
              setInputText(e.currentTarget.value);
              autoGrow();
            }}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            rows={1}
            disabled={isSwapping()}
          />

          <Show when={isThinking()}>
            <button
              type="button"
              class="stop-btn"
              onClick={() => cancelTurn("assistant")}
              title="Stop generating"
            >
              <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor" style="vertical-align: middle; margin-right: 4px;">
                <rect x="1" y="1" width="12" height="12" rx="2" />
              </svg>
              Stop
            </button>
          </Show>

          <Show when={!isThinking()}>
            <button
              type="submit"
              class="send-btn"
              disabled={isSwapping() || (!inputText().trim() && !pendingImage() && pendingFiles().length === 0)}
            >
              Send
            </button>
          </Show>
        </div>
      </form>

      <Show when={toolChoiceRequest()}>
        {(req) => (
          <div class="modal-overlay tool-choice-overlay">
            <div class="modal tool-choice-modal">
              <div class="modal-header">
                <h2>Choose a Tool</h2>
              </div>
              <div class="modal-body">
                <p>
                  Confidence {Math.round(req().confidence * 100)}% is below the auto-run threshold
                  ({Math.round(req().minConfidence * 100)}%). Pick the tool to continue.
                </p>
                <div class="tool-choice-list">
                  <For each={req().candidates}>
                    {(candidate) => (
                      <button
                        class="tool-choice-item"
                        type="button"
                        onClick={() => submitToolChoice(candidate.name)}
                      >
                        <span class="tool-choice-title">{candidate.label}</span>
                        <span class="tool-choice-meta">
                          {candidate.name} • {Math.round(candidate.confidence * 100)}%
                        </span>
                        <span class="tool-choice-reason">{candidate.reason}</span>
                      </button>
                    )}
                  </For>
                </div>
              </div>
              <div class="modal-footer">
                <button class="btn-secondary" type="button" onClick={dismissToolChoice}>
                  Cancel
                </button>
              </div>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
};

export default ChatView;
