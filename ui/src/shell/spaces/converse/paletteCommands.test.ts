/**
 * Converse palette commands (task 3.5, Req 4.7).
 *
 * Proves the former slash commands are folded into the Command Palette:
 *   • each former slash command is registered as a "Do" palette item,
 *   • is discoverable via palette search (including by its old "/name"),
 *   • dispatches the correct action (routing through existing paths only).
 *
 * The bridge is mocked so we can assert routed commands without a Tauri runtime.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const bridgeInvokeOptional = vi.fn(async () => null);
vi.mock("../../../bridge/invoke", () => ({
  bridgeInvoke: vi.fn(async () => ({ ok: false, code: "unavailable", message: "", command: "" })),
  bridgeInvokeOptional: (...args: unknown[]) => bridgeInvokeOptional(...(args as [])),
}));

import { registerConverseCommands, converseCommands } from "./paletteCommands";
import { clearCommands, listCommands } from "../../../palette/commands";
import { collectItems } from "../../../palette/sources";
import { searchItems } from "../../../palette/search";
import { clearRecents } from "../../../palette/recents";
import { converseStore, voiceStore } from "../../../stores";
import { navigate, currentRoute } from "../../router";

let dispose: (() => void) | null = null;

beforeEach(() => {
  clearCommands();
  clearRecents();
  bridgeInvokeOptional.mockClear();
  converseStore.clearMessages();
  if (voiceStore.active()) voiceStore.deactivate();
  navigate("converse");
  dispose = registerConverseCommands();
});

afterEach(() => {
  dispose?.();
  dispose = null;
});

describe("Converse palette commands — registration (Req 4.7)", () => {
  it("registers the four former slash commands as Do commands", () => {
    const ids = listCommands().map((c) => c.id);
    expect(ids).toEqual(
      expect.arrayContaining([
        "cmd.converse.clear",
        "cmd.converse.new",
        "cmd.converse.voice",
        "cmd.converse.settings",
      ]),
    );
  });

  it("contributes them to the palette's Do mode as items", () => {
    const doIds = collectItems("do").map((i) => i.id);
    expect(doIds).toEqual(
      expect.arrayContaining([
        "cmd.converse.clear",
        "cmd.converse.new",
        "cmd.converse.voice",
        "cmd.converse.settings",
      ]),
    );
  });

  it("unregisters them on dispose (no leftover slash commands)", () => {
    dispose?.();
    dispose = null;
    const ids = listCommands().map((c) => c.id);
    expect(ids).not.toContain("cmd.converse.clear");
  });
});

describe("Converse palette commands — discoverable by search (Req 4.7 / 2.1)", () => {
  const items = () => collectItems("do");

  it.each([
    ["clear", "cmd.converse.clear"],
    ["new conversation", "cmd.converse.new"],
    ["voice", "cmd.converse.voice"],
    ["settings", "cmd.converse.settings"],
  ])("finds %s by label", (query, id) => {
    const found = searchItems(items(), query).map((r) => r.item.id);
    expect(found).toContain(id);
  });

  it.each([
    ["/clear", "cmd.converse.clear"],
    ["/session", "cmd.converse.new"],
    ["/voice", "cmd.converse.voice"],
    ["/settings", "cmd.converse.settings"],
  ])("finds %s by its former slash name", (slash, id) => {
    const found = searchItems(items(), slash).map((r) => r.item.id);
    expect(found).toContain(id);
  });
});

describe("Converse palette commands — actions dispatch correctly (Req 4.7)", () => {
  function run(id: string): void {
    const cmd = converseCommands().find((c) => c.id === id)!;
    cmd.run();
  }

  it("Clear conversation clears the message stream", () => {
    converseStore.addMessage({
      id: "m1",
      threadId: "",
      role: "user",
      content: "hi",
      timestamp: Date.now(),
    });
    expect(converseStore.messages()).toHaveLength(1);

    run("cmd.converse.clear");
    expect(converseStore.messages()).toHaveLength(0);
  });

  it("New conversation routes through the existing create_session command", () => {
    run("cmd.converse.new");
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("create_session");
  });

  it("Toggle voice input activates then stops via existing voice commands", () => {
    expect(voiceStore.active()).toBe(false);

    run("cmd.converse.voice");
    expect(voiceStore.active()).toBe(true);
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("start_voice");

    run("cmd.converse.voice");
    expect(voiceStore.active()).toBe(false);
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("stop_voice");
  });

  it("Open Settings navigates to the Settings Space", () => {
    run("cmd.converse.settings");
    expect(currentRoute().space).toBe("settings");
  });
});
