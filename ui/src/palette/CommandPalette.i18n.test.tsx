/**
 * CommandPalette — long/expanded translation rendering (task 4.7).
 *
 * A component-level assertion that localized or expanded mode labels, hints, and
 * placeholders do not break the palette's control rendering (design §20.2 "long
 * translations"). Full visual/overflow validation is a later Linux/visual gate;
 * here we only prove the controls still render and stay reachable with long copy.
 *
 * The palette reads its mode labels/hints/placeholders from `./modes`; we mock
 * that module with deliberately long strings (simulating an expanded locale).
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import type { PaletteMode } from "./types";

vi.mock("./modes", () => {
  // Defined inside the factory: vi.mock is hoisted above module top-level vars.
  const LONG = (s: string) =>
    `${s} — ` + "sehr ausführliche lokalisierte Beschriftung ".repeat(4).trim();
  interface ModeDef {
    mode: PaletteMode;
    label: string;
    icon: string;
    prefix: string;
    placeholder: string;
    hint: string;
  }
  const MODES: readonly ModeDef[] = [
    { mode: "go", label: LONG("Gehe zu"), icon: "grid-3x3", prefix: "", placeholder: LONG("Zu einem Bereich navigieren"), hint: LONG("Navigiere überallhin") },
    { mode: "do", label: LONG("Ausführen"), icon: "zap", prefix: ">", placeholder: LONG("Befehl ausführen"), hint: LONG("Führe einen Befehl aus") },
    { mode: "ask", label: LONG("Fragen"), icon: "message-circle", prefix: "?", placeholder: LONG("Frage KRIA"), hint: LONG("Sende eine Nachricht") },
    { mode: "change", label: LONG("Ändern"), icon: "sliders-horizontal", prefix: "~", placeholder: LONG("Einstellung ändern"), hint: LONG("Ändere eine Einstellung") },
  ];
  const modeDef = (mode: PaletteMode): ModeDef => MODES.find((m) => m.mode === mode) ?? MODES[0];
  const parseQuery = (raw: string, baseMode: PaletteMode) => ({ mode: baseMode, text: raw, fromPrefix: false });
  return { MODES, MODE_ORDER: MODES.map((m) => m.mode), modeDef, parseQuery };
});

import { CommandPalette } from "./CommandPalette";
import { shellStore } from "../stores";

describe("CommandPalette — long translation rendering", () => {
  afterEach(() => {
    shellStore.setPaletteOpen(false);
    vi.restoreAllMocks();
  });

  it("renders all four mode tabs and the query control with long expanded labels", () => {
    render(() => <CommandPalette />);
    shellStore.setPaletteOpen(true);

    // The dialog and its combobox still render with expanded copy…
    const dialog = screen.getByRole("dialog", { name: "Command palette" });
    expect(dialog).toBeInTheDocument();
    const combobox = screen.getByRole("combobox");
    expect(combobox).toBeInTheDocument();

    // …and every mode tab is present and reachable (no control dropped/overflowed away).
    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(4);
    for (const label of ["Gehe zu", "Ausführen", "Fragen", "Ändern"]) {
      expect(screen.getByRole("tab", { name: new RegExp(label) })).toBeInTheDocument();
    }
    // The long placeholder is applied to the input (expanded hint text is used).
    expect(combobox.getAttribute("placeholder")).toContain("Zu einem Bereich navigieren");
  });
});
