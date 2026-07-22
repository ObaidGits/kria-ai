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
  isPaletteDoHotkey,
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

describe("isPaletteDoHotkey", () => {
  it("matches Ctrl+Shift+P and Cmd+Shift+P", () => {
    expect(isPaletteDoHotkey(new KeyboardEvent("keydown", { key: "p", ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(isPaletteDoHotkey(new KeyboardEvent("keydown", { key: "P", metaKey: true, shiftKey: true }))).toBe(true);
  });

  it("requires Shift (so it never collides with the Shift-agnostic K matcher)", () => {
    expect(isPaletteDoHotkey(new KeyboardEvent("keydown", { key: "p", ctrlKey: true }))).toBe(false);
  });

  it("ignores a bare 'p' and Alt-modified chords", () => {
    expect(isPaletteDoHotkey(new KeyboardEvent("keydown", { key: "p" }))).toBe(false);
    expect(isPaletteDoHotkey(new KeyboardEvent("keydown", { key: "p", ctrlKey: true, shiftKey: true, altKey: true }))).toBe(false);
  });

  it("does not match the summon chord, and Ctrl+Shift+K is not a Do chord", () => {
    // Ctrl+Shift+K still summons (Go) and is NOT a Do-mode chord.
    expect(isPaletteDoHotkey(new KeyboardEvent("keydown", { key: "k", ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(isSummonHotkey(new KeyboardEvent("keydown", { key: "k", ctrlKey: true, shiftKey: true }))).toBe(true);
    // Ctrl+Shift+P is not a summon chord.
    expect(isSummonHotkey(new KeyboardEvent("keydown", { key: "p", ctrlKey: true, shiftKey: true }))).toBe(false);
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
    expect(shellStore.paletteMode()).toBe("go");
    expect(bridgeInvokeOptional).toHaveBeenCalledWith(SUMMON_COMMAND);
  });

  it("opens the palette in Do mode when summoned with \"do\"", () => {
    summon("do");
    expect(shellStore.paletteOpen()).toBe(true);
    expect(shellStore.paletteMode()).toBe("do");
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
    expect(shellStore.paletteMode()).toBe("go");
  });

  it("opens the palette in Do mode on Ctrl+Shift+P", () => {
    keydown({ key: "p", ctrlKey: true, shiftKey: true });
    expect(shellStore.paletteOpen()).toBe(true);
    expect(shellStore.paletteMode()).toBe("do");
  });

  it("does not fire Ctrl+Shift+P while typing in an input (guard)", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    keydown({ key: "p", ctrlKey: true, shiftKey: true }, input);
    expect(shellStore.paletteOpen()).toBe(false);
  });

  it("Ctrl+Shift+K still summons Go mode (no collision with Do chord)", () => {
    keydown({ key: "k", ctrlKey: true, shiftKey: true });
    expect(shellStore.paletteOpen()).toBe(true);
    expect(shellStore.paletteMode()).toBe("go");
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

  // ── Ctrl/Meta parity at the handler level (design §20.2) ──
  it("opens the palette on Cmd(Meta)+K — Ctrl/Meta parity for summon", () => {
    keydown({ key: "k", metaKey: true });
    expect(shellStore.paletteOpen()).toBe(true);
    expect(shellStore.paletteMode()).toBe("go");
  });

  it("opens the palette in Do mode on Cmd(Meta)+Shift+P — Ctrl/Meta parity for Do", () => {
    keydown({ key: "p", metaKey: true, shiftKey: true });
    expect(shellStore.paletteOpen()).toBe(true);
    expect(shellStore.paletteMode()).toBe("do");
  });

  // ── Fires on a non-typing target ──
  it("fires the summon chord on a non-typing target (e.g. a button)", () => {
    const button = document.createElement("button");
    document.body.appendChild(button);
    button.focus();
    keydown({ key: "k", ctrlKey: true }, button);
    expect(shellStore.paletteOpen()).toBe(true);
  });

  // ── Typing-guard parity across every editable surface (handler level) ──
  it("suppresses the summon chord inside a textarea (typing guard)", () => {
    const textarea = document.createElement("textarea");
    document.body.appendChild(textarea);
    textarea.focus();
    keydown({ key: "k", ctrlKey: true }, textarea);
    expect(shellStore.paletteOpen()).toBe(false);
  });

  it("suppresses the summon chord inside a select (typing guard)", () => {
    const select = document.createElement("select");
    document.body.appendChild(select);
    select.focus();
    keydown({ key: "k", ctrlKey: true }, select);
    expect(shellStore.paletteOpen()).toBe(false);
  });

  it("suppresses the summon chord inside a contenteditable element (typing guard)", () => {
    const editable = document.createElement("div");
    Object.defineProperty(editable, "isContentEditable", { value: true });
    document.body.appendChild(editable);
    keydown({ key: "k", ctrlKey: true }, editable);
    expect(shellStore.paletteOpen()).toBe(false);
  });

  // ── Alt-modified chords are rejected at the handler level ──
  it("rejects an Alt-modified summon chord (Alt+Ctrl+K does not open)", () => {
    keydown({ key: "k", ctrlKey: true, altKey: true });
    expect(shellStore.paletteOpen()).toBe(false);
  });

  it("rejects an Alt-modified Do chord (Alt+Ctrl+Shift+P does not open)", () => {
    keydown({ key: "p", ctrlKey: true, shiftKey: true, altKey: true });
    expect(shellStore.paletteOpen()).toBe(false);
  });

  // ── Key repeat: one action per intent, idempotent while held ──
  it("is idempotent under key repeat (held Ctrl+K keeps the palette open, one action)", () => {
    keydown({ key: "k", ctrlKey: true, repeat: false });
    expect(shellStore.paletteOpen()).toBe(true);
    // Auto-repeat events while the chord is held must not error or toggle it off.
    keydown({ key: "k", ctrlKey: true, repeat: true });
    keydown({ key: "k", ctrlKey: true, repeat: true });
    expect(shellStore.paletteOpen()).toBe(true);
    expect(shellStore.paletteMode()).toBe("go");
  });

  // ── IME composition: chords never mis-fire from composition input ──
  it("does not summon from a composing keystroke inside an input (composition + typing guard)", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    // A composing keystroke in a text field must never summon: the typing guard
    // covers the editable surface where IME composition occurs.
    keydown({ key: "k", ctrlKey: true, isComposing: true }, input);
    expect(shellStore.paletteOpen()).toBe(false);
  });

  it("a bare composing character key never triggers a summon", () => {
    keydown({ key: "k", isComposing: true });
    expect(shellStore.paletteOpen()).toBe(false);
  });
});
