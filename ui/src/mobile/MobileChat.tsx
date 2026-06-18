import { Component, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { MobileClient, ServerFrame } from "../lib/mobileClient";
import { mobileStore } from "./mobileStore";

interface ChatLine {
  role: "user" | "assistant" | "system";
  text: string;
}

interface PendingApproval {
  requestId: string;
  action: string;
  risk: string;
}

/**
 * Mobile chat over the kria-server agent WebSocket (Phase 4.5.3). Streams agent
 * tokens and surfaces HITL approvals on the phone (4.5 security gate).
 */
const MobileChat: Component = () => {
  const sessionId = `mobile_${Math.random().toString(36).slice(2, 10)}`;
  const [lines, setLines] = createSignal<ChatLine[]>([]);
  const [draft, setDraft] = createSignal("");
  const [connected, setConnected] = createSignal(false);
  const [approval, setApproval] = createSignal<PendingApproval | null>(null);
  let client: MobileClient | null = null;
  let streamingIdx = -1;

  const append = (line: ChatLine) => setLines((prev) => [...prev, line]);

  const handleFrame = (frame: ServerFrame) => {
    switch (frame.type) {
      case "connected":
        setConnected(true);
        break;
      case "token": {
        const text = String(frame.text ?? "");
        setLines((prev) => {
          const next = [...prev];
          if (streamingIdx < 0 || next[streamingIdx]?.role !== "assistant") {
            next.push({ role: "assistant", text });
            streamingIdx = next.length - 1;
          } else {
            next[streamingIdx] = { role: "assistant", text: next[streamingIdx].text + text };
          }
          return next;
        });
        break;
      }
      case "approval_required":
        setApproval({
          requestId: String(frame.request_id ?? ""),
          action: String(frame.action ?? "action"),
          risk: String(frame.risk_level ?? ""),
        });
        break;
      case "done":
        streamingIdx = -1;
        break;
      case "error":
        append({ role: "system", text: `⚠️ ${String(frame.message ?? "error")}` });
        streamingIdx = -1;
        break;
    }
  };

  const ensureClient = (): MobileClient => {
    if (client) return client;
    client = new MobileClient({
      serverUrl: mobileStore.serverUrl(),
      token: mobileStore.token(),
      onFrame: handleFrame,
      onClose: () => setConnected(false),
      onError: () => setConnected(false),
    });
    client.connect();
    return client;
  };

  const send = (e: Event) => {
    e.preventDefault();
    const text = draft().trim();
    if (!text) return;
    append({ role: "user", text });
    streamingIdx = -1;
    ensureClient().sendChat(sessionId, text);
    setDraft("");
  };

  const respond = (approved: boolean) => {
    const a = approval();
    if (!a || !client) return;
    if (approved) client.approve(a.requestId);
    else client.deny(a.requestId);
    setApproval(null);
  };

  onCleanup(() => client?.close());

  // Establish the connection up front so the first message isn't sent while the
  // socket is still CONNECTING.
  onMount(() => {
    ensureClient();
  });

  return (
    <div class="mobile-chat">
      <div class="mobile-chat-log">
        <For each={lines()}>
          {(line) => <div class={`mobile-line ${line.role}`}>{line.text}</div>}
        </For>
      </div>

      <Show when={approval()}>
        {(a) => (
          <div class="mobile-approval">
            <div>
              Approve <strong>{a().action}</strong> ({a().risk})?
            </div>
            <div class="mobile-approval-actions">
              <button onClick={() => respond(true)}>Approve</button>
              <button class="danger" onClick={() => respond(false)}>
                Deny
              </button>
            </div>
          </div>
        )}
      </Show>

      <form class="mobile-chat-input" onSubmit={send}>
        <input
          type="text"
          placeholder={connected() ? "Message KRIA…" : "Send to connect…"}
          value={draft()}
          onInput={(e) => setDraft(e.currentTarget.value)}
        />
        <button type="submit">Send</button>
      </form>
    </div>
  );
};

export default MobileChat;
