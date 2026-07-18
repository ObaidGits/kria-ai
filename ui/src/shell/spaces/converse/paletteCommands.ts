/**
 * Converse palette commands (task 3.5, Req 4.7).
 *
 * The legacy chat had a SEPARATE slash-command menu (typing "/" opened an
 * inline dropdown in the composer). Req 4.7 folds those actions into the
 * Command Palette so there is a SINGLE home for commands — no competing menu.
 * These are the former slash commands, re-expressed as palette "Do" mode
 * commands (Req 2.2) and registered when the Converse Space mounts.
 *
 * Slash → palette mapping (Do mode):
 *   /clear    → "Clear conversation"     cmd.converse.clear
 *   /session  → "New conversation"       cmd.converse.new
 *   /voice    → "Toggle voice input"     cmd.converse.voice
 *   /settings → "Open Settings"          cmd.converse.settings
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Every command here is a pure UI action or routes through an EXISTING command
 * — never a direct prompt→tool shortcut (design.md invariant, commands.ts):
 *   • Clear conversation → converseStore.clearMessages() (local UI state only).
 *   • New conversation   → the existing `create_session` command via the bridge.
 *   • Toggle voice input → voiceStore + the existing `start_voice`/`stop_voice`
 *                          optional commands (silently degrades if absent).
 *   • Open Settings      → navigation only.
 */
import { converseStore, voiceStore } from "../../../stores";
import { bridgeInvokeOptional } from "../../../bridge/invoke";
import { navigate } from "../../router";
import { registerCommands, type PaletteCommand } from "../../../palette/commands";

/**
 * Toggle voice input, mirroring the Composer's voice-entry path (Composer.tsx):
 * activate + start_voice when idle, deactivate + stop_voice when active. Both
 * backend commands are optional → this degrades silently when voice isn't
 * available on the system (Req 18.2 / 20.4).
 */
function toggleVoiceInput(): void {
  if (voiceStore.active()) {
    voiceStore.deactivate();
    void bridgeInvokeOptional("stop_voice");
  } else {
    voiceStore.activate();
    void bridgeInvokeOptional("start_voice");
  }
}

/** The former slash commands, as palette "Do" commands. */
export function converseCommands(): PaletteCommand[] {
  return [
    {
      id: "cmd.converse.clear",
      title: "Clear conversation",
      subtitle: "Clear the current messages",
      icon: "eraser",
      keywords: "clear reset messages slash /clear",
      run: () => converseStore.clearMessages(),
    },
    {
      id: "cmd.converse.new",
      title: "New conversation",
      subtitle: "Start a fresh conversation",
      icon: "message-square-plus",
      keywords: "new session thread conversation slash /session",
      run: () => void bridgeInvokeOptional("create_session"),
    },
    {
      id: "cmd.converse.voice",
      title: "Toggle voice input",
      subtitle: "Start or stop voice input",
      icon: "mic",
      keywords: "voice speak microphone slash /voice",
      run: toggleVoiceInput,
    },
    {
      id: "cmd.converse.settings",
      title: "Open Settings",
      icon: "settings",
      keywords: "settings preferences config slash /settings",
      run: () => navigate("settings"),
    },
  ];
}

/**
 * Register the Converse "Do" commands. Call on ConverseSpace mount; the returned
 * disposer unregisters them on unmount (and in tests).
 */
export function registerConverseCommands(): () => void {
  return registerCommands(converseCommands());
}
