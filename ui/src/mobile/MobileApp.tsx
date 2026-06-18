import { Component, createSignal, Show } from "solid-js";
import { mobileStore } from "./mobileStore";
import MobilePairing from "./MobilePairing";
import MobileChat from "./MobileChat";
import RemoteDesktopView from "./RemoteDesktopView";

type Tab = "chat" | "desktop" | "settings";

/**
 * Standalone mobile PWA shell (Phase 4.5 / 4.6).
 *
 * Served at `/m`, independent of the Tauri desktop app so it runs in a plain
 * browser on a phone. Tabs: Chat (agent over WS), Desktop (noVNC takeover),
 * Settings (pairing + revoke-local). Everything talks to `kria-server` over the
 * private mesh with a per-device token.
 */
const MobileApp: Component = () => {
  const [tab, setTab] = createSignal<Tab>("chat");

  return (
    <div class="mobile-app">
      <header class="mobile-header">
        <span class="mobile-brand">KRIA</span>
        <Show when={mobileStore.isPaired()}>
          <span class="mobile-server">{mobileStore.serverUrl()}</span>
        </Show>
      </header>

      <Show when={mobileStore.isPaired()} fallback={<MobilePairing />}>
        <main class="mobile-body">
          <Show when={tab() === "chat"}>
            <MobileChat />
          </Show>
          <Show when={tab() === "desktop"}>
            <RemoteDesktopView />
          </Show>
          <Show when={tab() === "settings"}>
            <div class="mobile-settings">
              <p class="mobile-hint">Connected to {mobileStore.serverUrl()}</p>
              <button class="danger" onClick={() => mobileStore.clear()}>
                Forget this device token
              </button>
            </div>
          </Show>
        </main>

        <nav class="mobile-tabs">
          <button class={tab() === "chat" ? "active" : ""} onClick={() => setTab("chat")}>
            Chat
          </button>
          <button class={tab() === "desktop" ? "active" : ""} onClick={() => setTab("desktop")}>
            Desktop
          </button>
          <button class={tab() === "settings" ? "active" : ""} onClick={() => setTab("settings")}>
            Settings
          </button>
        </nav>
      </Show>
    </div>
  );
};

export default MobileApp;
