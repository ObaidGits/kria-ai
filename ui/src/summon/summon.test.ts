/**
 * Summon tests (task 2.5).
 *
 * Verifies (Req 2.5, 18.2):
 *  - the summon action opens the palette AND calls the optional focus bridge
 *  - summon degrades silently when the focus command is unavailable
 *  - the guaranteed in-app hotkey (Ctrl/Cmd+K) opens the palette
 *  - the hotkey is guarded so it does NOT fire while typing in an input
 *  - the hotkey matcher only matches the intended chord
 *  - dispose detaches the listener
 *
 * The global system hotkey is a backend enhancement (registered in Rust,
 * try/degrade); it is exercised at the Rust layer, so here we assert the
 * webview fallback path that must always work.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Spy the optional bridge invoke (window focus is best-effort / enhancement).
const bridgeInvokeOptional = vi.fn((..._args: unknown[]) => Promise.resolve<unknown>(null));
vi.mock("../bridge", () => ({
  bridgeInvokeOptional: (...args: unknown[]) => bridgeInvokeOptional(...args),
}));

// The Tauri event API is unavailable in jsdom; provide an inert listen mock so
// the module import resolves. `initSummon` only subscribes when the Tauri
// runtime is present (it is not in tests), so this is defensive.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

import {
  summon,
  initSummon,
  disposeSummon,
  isSummonHotkey,
  isTypingTarget,
  SUMMON_COMMAND,
} from "./summon";
import { shellStore } from "../stores";

function keydown(init: KeyboardEventInit, target?: EventTarget): void {
  const event = new KeyboardEvent("keydown", { bubbles: true, ...init });
  (target ?? document).dispatchEvent(event);
}

describe("isSummonHotkey", () => {
  it("matches Ctrl+K and Cmd+K", () => {
    expect(isSummonHotkey(new KeyboardEvent("keydown", { key: "k", ctrlKey: true }))).toBe(true);
    expect(isSummonHotkey(new KeyboardEvent("keydown", { key: "K", metaKey: true }))).toBe(true);
  });

  it("ignores a bare 'k' and Alt-modified chords", () => {
    expect(isSummonHotkey(new KeyboardEvent("keydown", { key: "k" }))).toBe(false);
    expect(isSummonHotkey(new KeyboardEvent("keydown", { key: "k", ctrlKey: true, altKey: true }))).toBe(false);
  });

  it("ignores other keys with the modifier", () => {
    expect(isSummonHotkey(new KeyboardEvent("keydown", { key: "j", ctrlKey: true }))).toBe(false);
  });
});

describe("isTypingTarget", () => {
  it("is true for input/textarea/select", () => {
    expect(isTypingTarget(document.createElement("input"))).toBe(true);
    expect(isTypingTarget(document.createElement("textarea"))).toBe(true);
    expect(isTypingTarget(document.createElement("select"))).toBe(true);
  });

  it("is true for contenteditable and false otherwise", () => {
    const editable = document.createElement("div");
    editable.contentEditable = "true";
    // jsdom doesn't compute isContentEditable from the attribute; set directly.
    Object.defineProperty(editable, "isContentEditable", { value: true });
    expect(isTypingTarget(editable)).toBe(true);
    expect(isTypingTarget(document.createElement("div"))).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
  });
});

describe("summon()", () => {
  beforeEach(() => {
    bridgeInvokeOptional.mockClear();
    shellStore.setPaletteOpen(false);
  });

  it("opens the palette and calls the optional focus bridge", () => {
    summon();
    expect(shellStore.paletteOpen()).toBe(true);
    expect(bridgeInvokeOptional).toHaveBeenCalledWith(SUMMON_COMMAND);
  });

  it("still opens the palette when the focus command is unavailable", () => {
    // bridgeInvokeOptional resolves null (unavailable) — summon must not throw
    // and must still open the palette (guaranteed path).
    bridgeInvokeOptional.mockReturnValueOnce(Promise.resolve(null));
    expect(() => summon()).not.toThrow();
    expect(shellStore.paletteOpen()).toBe(true);
  });
});

describe("initSummon() in-app hotkey", () => {
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    bridgeInvokeOptional.mockClear();
    shellStore.setPaletteOpen(false);
    dispose = initSummon();
  });

  afterEach(() => {
    dispose?.();
    disposeSummon();
    document.body.innerHTML = "";
  });

  it("opens the palette on Ctrl+K", () => {
    keydown({ key: "k", ctrlKey: true });
    expect(shellStore.paletteOpen()).toBe(true);
  });

  it("does not fire while typing in an input (guard)", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    keydown({ key: "k", ctrlKey: true }, input);
    expect(shellStore.paletteOpen()).toBe(false);
  });

  it("stops responding after dispose", () => {
    disposeSummon();
    keydown({ key: "k", ctrlKey: true });
    expect(shellStore.paletteOpen()).toBe(false);
  });
});
