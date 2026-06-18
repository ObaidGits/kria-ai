import { Component, createSignal, Show } from "solid-js";
import { pairDevice } from "../lib/mobileClient";
import { mobileStore } from "./mobileStore";

/**
 * Device pairing screen (Phase 4.5.4). The user enters the laptop's tailnet
 * address and the pairing code shown by the desktop, and receives a per-device
 * token stored locally. The token can be revoked from the desktop at any time.
 */
const MobilePairing: Component = () => {
  const [server, setServer] = createSignal(mobileStore.serverUrl());
  const [code, setCode] = createSignal("");
  const [name, setName] = createSignal("My phone");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  const submit = async (e: Event) => {
    e.preventDefault();
    setError("");
    setBusy(true);
    try {
      const url = server().trim();
      const token = await pairDevice(url, code().trim(), name().trim());
      mobileStore.setServerUrl(url);
      mobileStore.setToken(token);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form class="mobile-pairing" onSubmit={submit}>
      <h2>Pair this device</h2>
      <p class="mobile-hint">
        Enter your laptop's private (Tailscale) address and the pairing code shown in KRIA on the
        desktop.
      </p>
      <label>
        Server URL
        <input
          type="url"
          placeholder="https://laptop.tailnet.ts.net:8787"
          value={server()}
          onInput={(e) => setServer(e.currentTarget.value)}
          required
        />
      </label>
      <label>
        Pairing code
        <input
          type="text"
          placeholder="code from desktop"
          value={code()}
          onInput={(e) => setCode(e.currentTarget.value)}
          required
        />
      </label>
      <label>
        Device name
        <input type="text" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
      </label>
      <Show when={error()}>
        <div class="mobile-error">{error()}</div>
      </Show>
      <button type="submit" disabled={busy()}>
        {busy() ? "Pairing…" : "Pair device"}
      </button>
    </form>
  );
};

export default MobilePairing;
