/** Persisted mobile-client connection state (server URL + device token). */
import { createSignal } from "solid-js";

const SERVER_KEY = "kria_mobile_server";
const TOKEN_KEY = "kria_mobile_token";

function load(key: string): string {
  try {
    return localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

const [serverUrl, setServerUrlSignal] = createSignal(load(SERVER_KEY));
const [token, setTokenSignal] = createSignal(load(TOKEN_KEY));

export const mobileStore = {
  serverUrl,
  token,
  setServerUrl(url: string) {
    setServerUrlSignal(url);
    try {
      localStorage.setItem(SERVER_KEY, url);
    } catch {
      /* ignore */
    }
  },
  setToken(t: string) {
    setTokenSignal(t);
    try {
      localStorage.setItem(TOKEN_KEY, t);
    } catch {
      /* ignore */
    }
  },
  isPaired(): boolean {
    return token().length > 0 && serverUrl().length > 0;
  },
  clear() {
    setTokenSignal("");
    try {
      localStorage.removeItem(TOKEN_KEY);
    } catch {
      /* ignore */
    }
  },
};
