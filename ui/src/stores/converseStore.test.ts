/**
 * converseStore — Send/Stop pipeline + per-thread draft persistence (task 3.4).
 *
 * Verifies the KRIA runtime-authority invariant for the Composer's actions:
 *   • Send routes through the EXISTING converse send commands (send_message /
 *     send_lab_message) — a pipeline call, never a prompt→tool shortcut (Req 4.4/4.9).
 *   • Stop routes through the existing `cancel_turn` cancellation command (Req 4.4).
 *   • Drafts persist per thread, restore on thread switch, and survive relaunch
 *     via localStorage (Req 4.5).
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Mock the bridge invoke layer so we can assert WHICH existing command runs.
vi.mock("../bridge/invoke", () => ({
  bridgeInvoke: vi.fn(async () => ({ ok: true, data: {} })),
  bridgeInvokeOptional: vi.fn(async () => null),
}));

import { converseStore } from "./converseStore";
import { eventBus } from "./eventBus";
import { bridgeInvoke, bridgeInvokeOptional } from "../bridge/invoke";

const mockInvoke = bridgeInvoke as unknown as ReturnType<typeof vi.fn>;
const mockInvokeOptional = bridgeInvokeOptional as unknown as ReturnType<typeof vi.fn>;

const DRAFTS_KEY = "kria.converse.drafts";

function resetStore(): void {
  eventBus.clear();
  converseStore.clearMessages();
  converseStore.setThreads([]);
  void converseStore.searchThreads("");
  converseStore.setActiveThread(null);
  converseStore.updateDraft({ text: "", attachments: [], mode: "assistant" });
  window.localStorage.clear();
  mockInvoke.mockClear();
  mockInvokeOptional.mockClear();
  mockInvoke.mockResolvedValue({ ok: true, data: {} });
}

beforeEach(resetStore);
afterEach(() => {
  window.localStorage.clear();
});

describe("converseStore.sendMessage — routes through the existing pipeline (Req 4.4/4.9)", () => {
  it("Assistant mode sends via the existing send_message command", async () => {
    converseStore.updateDraft({ text: "hello there", mode: "assistant" });
    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("send_message", { message: "hello there" });
  });

  it("Lab mode sends via the existing send_lab_message command (tool-locked)", async () => {
    converseStore.updateDraft({ text: "run analysis", mode: "lab" });
    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenCalledWith("send_lab_message", { message: "run analysis" });
  });

  it("appends an optimistic user turn and clears the draft on send", async () => {
    converseStore.updateDraft({ text: "  spaced  ", mode: "assistant" });
    await converseStore.sendMessage();

    const msgs = converseStore.messages();
    expect(msgs).toHaveLength(1);
    expect(msgs[0]).toMatchObject({ role: "user", content: "spaced" });
    // Draft text cleared; mode preserved.
    expect(converseStore.composerDraft().text).toBe("");
    expect(converseStore.composerDraft().mode).toBe("assistant");
  });

  it("does nothing when the draft is empty (no pipeline call)", async () => {
    converseStore.updateDraft({ text: "   ", mode: "assistant" });
    await converseStore.sendMessage();
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(converseStore.messages()).toHaveLength(0);
  });

  it("restores the draft text when the send fails (nothing lost)", async () => {
    mockInvoke.mockResolvedValueOnce({ ok: false, code: "error", message: "boom" });
    converseStore.updateDraft({ text: "keep me", mode: "assistant" });
    await converseStore.sendMessage();
    expect(converseStore.composerDraft().text).toBe("keep me");
  });
});

describe("converseStore.submitIntent — Mini uses authoritative pipeline (Req 15.7)", () => {
  it("sends via send_message without replacing the Composer draft", async () => {
    converseStore.updateDraft({ text: "keep this draft", mode: "lab" });

    await expect(converseStore.submitIntent("  bounded mini intent  ")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("send_message", { message: "bounded mini intent" });
    expect(converseStore.composerDraft()).toMatchObject({ text: "keep this draft", mode: "lab" });
  });

  it("rejects empty intent without entering the runtime pipeline", async () => {
    await expect(converseStore.submitIntent("   ")).resolves.toBe(false);
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});

describe("converseStore.stopTurn — reuses the existing cancellation (Req 4.4)", () => {
  it("cancels via the existing cancel_turn command", async () => {
    converseStore.setActiveThread("thread-9");
    await converseStore.stopTurn();
    expect(mockInvokeOptional).toHaveBeenCalledWith("cancel_turn", { sessionId: "thread-9" });
  });
});

describe("converseStore drafts — per-thread persistence (Req 4.5)", () => {
  it("saves the draft on switch and restores it when returning to the thread", () => {
    converseStore.setActiveThread("t1");
    converseStore.updateDraft({ text: "draft for one", mode: "lab" });

    converseStore.setActiveThread("t2");
    // New thread starts clean.
    expect(converseStore.composerDraft().text).toBe("");
    expect(converseStore.composerDraft().mode).toBe("assistant");

    converseStore.setActiveThread("t1");
    // Returning restores text + mode.
    expect(converseStore.composerDraft().text).toBe("draft for one");
    expect(converseStore.composerDraft().mode).toBe("lab");
  });

  it("persists drafts to localStorage so they survive relaunch", async () => {
    converseStore.setActiveThread("t-persist");
    converseStore.updateDraft({ text: "survive relaunch", mode: "lab" });

    // Wait out the debounced write.
    await new Promise((r) => setTimeout(r, 260));

    const raw = window.localStorage.getItem(DRAFTS_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!);
    expect(parsed["t-persist"]).toMatchObject({ text: "survive relaunch", mode: "lab" });
  });
});

function addActionConversation(overrides: Record<string, unknown> = {}): void {
  converseStore.setActiveThread("thread-actions");
  converseStore.addMessage({
    id: "user-1",
    threadId: "thread-actions",
    role: "user",
    content: "original question",
    timestamp: 1,
  });
  converseStore.addMessage({
    id: "assistant-1",
    threadId: "thread-actions",
    role: "assistant",
    content: "original answer",
    timestamp: 2,
    ...overrides,
  });
}

describe("converseStore per-message actions — authoritative backend commands", () => {
  it("retries the originating user turn through send_message", async () => {
    addActionConversation();

    await expect(converseStore.retryMessage("assistant-1")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("send_message", { message: "original question" });
  });

  it("requests explanation through the assistant pipeline", async () => {
    addActionConversation();

    await expect(converseStore.explainMessage("assistant-1")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("send_message", {
      message: "Explain this response, including assumptions and evidence:\n\noriginal answer",
    });
  });

  it("persists remembered content through memory_remember", async () => {
    addActionConversation();

    await expect(converseStore.rememberMessage("assistant-1")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("memory_remember", { text: "original answer" });
  });

  it("branches through branch_session then activates the persisted branch", async () => {
    addActionConversation();
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === "branch_session") return { ok: true, data: { session_id: "branch-1" } };
      if (command === "list_sessions") {
        return { ok: true, data: [{ id: "branch-1", title: "Branch" }] };
      }
      if (command === "get_session_history") return { ok: true, data: [] };
      return { ok: true, data: undefined };
    });

    await expect(converseStore.branchMessage("assistant-1")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("branch_session", {
      sourceSessionId: "thread-actions",
      throughIndex: 1,
    });
    expect(mockInvoke).toHaveBeenCalledWith("switch_session", { sessionId: "branch-1" });
    expect(converseStore.activeThreadId()).toBe("branch-1");
  });

  it("records positive feedback against attached grounding memories", async () => {
    addActionConversation({ usedMemoryIds: ["memory-1", "memory-2"] });

    await expect(converseStore.submitFeedback("assistant-1", "up")).resolves.toBe(true);

    expect(mockInvoke).not.toHaveBeenCalledWith("memory_reason", expect.anything());
    expect(mockInvoke).toHaveBeenCalledWith("memory_record_feedback", {
      targetId: "memory-1",
      targetKind: "memory",
      signal: "thumbs_up",
      detail: undefined,
      context: "original question",
    });
    expect(mockInvoke).toHaveBeenCalledWith("memory_record_feedback", {
      targetId: "memory-2",
      targetKind: "memory",
      signal: "thumbs_up",
      detail: undefined,
      context: "original question",
    });
  });

  it("resolves missing provenance, records negative memory and routing feedback", async () => {
    addActionConversation({ metadata: { toolName: "web_search" } });
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === "memory_reason") {
        return { ok: true, data: { results: [{ id: "memory-grounding" }] } };
      }
      return { ok: true, data: {} };
    });

    await expect(converseStore.submitFeedback("assistant-1", "down")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("memory_record_feedback", {
      targetId: "memory-grounding",
      targetKind: "memory",
      signal: "thumbs_down",
      detail: "assistant_response",
      context: "original question",
    });
    expect(mockInvoke).toHaveBeenCalledWith("submit_turn_feedback", {
      sessionId: "thread-actions",
      userText: "original question",
      toolSelected: "web_search",
      outcomeType: "try_differently",
    });
  });

  it("reports an honest no-grounding result for positive feedback", async () => {
    addActionConversation();
    mockInvoke.mockResolvedValue({ ok: true, data: { results: [] } });
    const notifications: string[] = [];
    eventBus.on("notification:push", (payload) => notifications.push(payload.message));

    await expect(converseStore.submitFeedback("assistant-1", "up")).resolves.toBe(false);

    expect(notifications).toContain("No grounding memories were attached to this response");
    expect(mockInvoke).not.toHaveBeenCalledWith("memory_record_feedback", expect.anything());
  });

  it("surfaces backend action failures through shared notifications", async () => {
    addActionConversation();
    mockInvoke.mockResolvedValueOnce({ ok: false, code: "error", message: "disk full" });
    const notifications: Array<{ level: string; message: string }> = [];
    eventBus.on("notification:push", (payload) => notifications.push(payload));

    await expect(converseStore.rememberMessage("assistant-1")).resolves.toBe(false);

    expect(notifications).toContainEqual(expect.objectContaining({
      level: "error",
      message: "Couldn't remember message: disk full",
    }));
  });
});


describe("converseStore session management — backend authority", () => {
  it("hydrates temporary state and pins pinned sessions first", async () => {
    mockInvoke.mockResolvedValueOnce({
      ok: true,
      data: [
        { id: "recent", title: "Recent", last_active: "2026-01-02T00:00:00Z" },
        { id: "pinned", title: "Pinned", last_active: "2026-01-01T00:00:00Z", pinned: true, temporary: true },
      ],
    });

    await converseStore.loadThreads();

    expect(converseStore.threads().map((thread) => thread.id)).toEqual(["pinned", "recent"]);
    expect(converseStore.threads()[0].temporary).toBe(true);
  });

  it("searches persisted conversation content through search_sessions", async () => {
    mockInvoke.mockResolvedValueOnce({
      ok: true,
      data: [{
        session_id: "thread-found",
        role: "assistant",
        content: "matching answer",
        timestamp: "2026-01-03T00:00:00Z",
      }],
    });

    const hits = await converseStore.searchThreads("matching");

    expect(mockInvoke).toHaveBeenCalledWith("search_sessions", { query: "matching" });
    expect(hits).toEqual([expect.objectContaining({
      sessionId: "thread-found",
      role: "assistant",
      content: "matching answer",
    })]);
  });

  it("persists pin/archive/temporary flags and updates the canonical read model", async () => {
    converseStore.setThreads([{
      id: "thread-flags",
      title: "Flags",
      createdAt: 1,
      updatedAt: 1,
      pinned: false,
      archived: false,
      temporary: false,
    }]);

    await expect(converseStore.setThreadPinned("thread-flags", true)).resolves.toBe(true);
    await expect(converseStore.setThreadArchived("thread-flags", true)).resolves.toBe(true);
    await expect(converseStore.setThreadTemporary("thread-flags", true)).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("set_session_pinned", {
      sessionId: "thread-flags",
      pinned: true,
    });
    expect(mockInvoke).toHaveBeenCalledWith("set_session_archived", {
      sessionId: "thread-flags",
      archived: true,
    });
    expect(mockInvoke).toHaveBeenCalledWith("set_session_temporary", {
      sessionId: "thread-flags",
      temporary: true,
    });
    expect(converseStore.threads()[0]).toMatchObject({ pinned: true, archived: true, temporary: true });
  });

  it("exports the active persisted conversation as Markdown through save_export_file", async () => {
    converseStore.setThreads([{
      id: "thread-export",
      title: "Export me",
      createdAt: 1,
      updatedAt: 1,
      pinned: false,
      archived: false,
      temporary: false,
    }]);
    converseStore.setActiveThread("thread-export");
    converseStore.addMessage({
      id: "export-user",
      threadId: "thread-export",
      role: "user",
      content: "Hello export",
      timestamp: Date.UTC(2026, 0, 1),
    });
    mockInvoke.mockResolvedValueOnce({ ok: true, data: "/tmp/Export-me.md" });

    await expect(converseStore.exportActiveConversation()).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("save_export_file", expect.objectContaining({
      defaultName: expect.stringMatching(/^kria-Export-me-\d{4}-\d{2}-\d{2}\.md$/),
      filterName: "Markdown Files",
      extensions: ["md"],
      content: expect.stringMatching(/# KRIA Conversation — Export me[\s\S]*## You · [^\n]+[\s\S]*Hello export/),
    }));
  });
});


describe("converseStore attachments — real backend payloads", () => {
  it("indexes document bytes then sends the returned prompt through the assistant pipeline", async () => {
    converseStore.setActiveThread("thread-doc");
    converseStore.updateDraft({
      text: "Summarize this",
      attachments: [{
        id: "doc-1",
        name: "report.txt",
        mime: "text/plain",
        size: 3,
        bytes: new Uint8Array([97, 98, 99]),
      }],
    });
    mockInvoke
      .mockResolvedValueOnce({ ok: true, data: { status: "indexed", prompt: "indexed prompt" } })
      .mockResolvedValueOnce({ ok: true, data: {} });

    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenNthCalledWith(1, "send_document_message", {
      sessionId: "thread-doc",
      files: [{ name: "report.txt", mime: "text/plain", bytes: [97, 98, 99] }],
      text: "Summarize this",
    }, { timeoutMs: 120_000 });
    expect(mockInvoke).toHaveBeenNthCalledWith(2, "send_message", { message: "indexed prompt" });
    expect(converseStore.composerDraft().attachments).toEqual([]);
  });

  it("sends one image's real bytes through send_image_message", async () => {
    converseStore.setActiveThread("thread-image");
    converseStore.updateDraft({
      text: "Read this screenshot",
      attachments: [{
        id: "image-1",
        name: "screen.png",
        mime: "image/png",
        size: 2,
        bytes: new Uint8Array([1, 2]),
      }],
    });

    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenCalledWith("send_image_message", {
      imageData: [1, 2],
      mimeType: "image/png",
      text: "Read this screenshot",
    }, { timeoutMs: 120_000 });
  });

  it("transcribes uploaded audio and renders the authoritative transcript", async () => {
    converseStore.setActiveThread("thread-audio");
    converseStore.updateDraft({
      text: "",
      attachments: [{
        id: "audio-1",
        name: "note.wav",
        mime: "audio/wav",
        size: 2,
        bytes: new Uint8Array([3, 4]),
      }],
    });
    mockInvoke.mockResolvedValueOnce({
      ok: true,
      data: { text: "spoken words", language: "en", confidence: 0.91, engine: "Whisper" },
    });

    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenCalledWith("voice_transcribe_uploaded_audio", {
      name: "note.wav",
      bytes: [3, 4],
    }, { timeoutMs: 120_000 });
    const transcriptMessages = converseStore.messages();
    expect(transcriptMessages[transcriptMessages.length - 1]).toMatchObject({
      role: "assistant",
      content: "Transcript (Whisper, 91%):\n\nspoken words",
    });
    expect(converseStore.thinking()).toBe(false);
  });

  it("rejects mixed attachment classes without dropping or dispatching files", async () => {
    converseStore.setActiveThread("thread-mixed");
    converseStore.updateDraft({
      text: "mixed",
      attachments: [
        { id: "doc", name: "a.txt", mime: "text/plain", size: 1, bytes: new Uint8Array([1]) },
        { id: "image", name: "b.png", mime: "image/png", size: 1, bytes: new Uint8Array([2]) },
      ],
    });
    const notifications: string[] = [];
    eventBus.on("notification:push", (payload) => notifications.push(payload.message));

    await converseStore.sendMessage();

    expect(mockInvoke).not.toHaveBeenCalled();
    expect(converseStore.composerDraft().attachments).toHaveLength(2);
    expect(notifications).toContain("Send documents together, or one image/audio file per message");
  });
});