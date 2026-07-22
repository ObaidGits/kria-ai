/**
 * Long/expanded localization rendering for the Dock (task 7.8; design §12
 * "localization expansion" edge case, design §20.2 "long translations").
 *
 * Component-level assertion only: when Space labels expand far beyond the
 * English originals (a common localization outcome), the Dock must still render
 * exactly the seven canonical one-click buttons in canonical order, keep each
 * button's full accessible NAME, keep the concise terminology outcome
 * DESCRIPTION intact, and keep one-click switching + aria-current working.
 * Pixel-level overflow is a later visual/Linux gate; here we prove no button is
 * dropped and rendering does not break.
 *
 * Requirements: 7.1, 7.7, 7.8, 17.1, 17.2
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within, cleanup } from "@solidjs/testing-library";

// Expanded, localized Space labels (deliberately much longer than the English
// originals). The Dock reads labels from SPACE_META, so mocking it here injects
// the expansion at the single source the Dock consumes. `vi.hoisted` lets the
// hoisted `vi.mock` factory reference these safely.
const LONG_LABELS: Record<string, string> = vi.hoisted(() => ({
  converse:
    "Unterhaltungs-Arbeitsbereich mit erweiterter lokalisierter Bezeichnung, die deutlich länger ist",
  memory: "Wissensspeicher und dauerhaft aufbewahrte Erkenntnisse über lange Zeiträume hinweg",
  automations: "Automatisierte Arbeitsabläufe, geplante Aufgaben und wiederkehrende Routinen",
  capabilities: "Werkzeuge, Fähigkeiten, Modelle und Integrationen des gesamten Systems",
  machines: "Verwaltete Remote-Geräte und deren Verbindungs- und Bereitstellungszustand",
  observatory: "Systembeobachtung, Telemetrie und laufende Aktivitätsüberwachung in Echtzeit",
  settings: "Konfiguration, Präferenzen und Dienstprogramme der gesamten Anwendung",
}));

vi.mock("./spaces", () => ({
  SPACE_META: {
    converse: { label: LONG_LABELS.converse, icon: "message-circle" },
    memory: { label: LONG_LABELS.memory, icon: "brain" },
    automations: { label: LONG_LABELS.automations, icon: "workflow" },
    capabilities: { label: LONG_LABELS.capabilities, icon: "sparkles" },
    machines: { label: LONG_LABELS.machines, icon: "monitor" },
    observatory: { label: LONG_LABELS.observatory, icon: "activity" },
    settings: { label: LONG_LABELS.settings, icon: "settings" },
  },
}));

// Imported AFTER the mock so the Dock binds to the long-label SPACE_META.
import { Dock, spaceOutcome } from "./Dock";
import { navigate, currentRoute, ALL_SPACES } from "./router";
import { getTerm } from "./terminology";

describe("Dock — long/expanded localized labels render without breaking (task 7.8)", () => {
  beforeEach(() => {
    navigate("converse");
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("still renders exactly the seven canonical buttons in canonical order", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    const buttons = within(nav).getAllByRole("button");

    expect(buttons).toHaveLength(7);
    expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual(
      ALL_SPACES.map((s) => LONG_LABELS[s]),
    );
  });

  it("keeps the full expanded label as the accessible name and visible label", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    for (const space of ALL_SPACES) {
      const button = within(nav).getByRole("button", { name: LONG_LABELS[space] });
      // Accessible name is the full (untruncated) expanded label.
      expect(button.getAttribute("aria-label")).toBe(LONG_LABELS[space]);
      // The visible label node also carries the full expanded text.
      expect(button.querySelector(".kria-dock__label")?.textContent).toBe(LONG_LABELS[space]);
    }
  });

  it("keeps terminology outcome descriptions intact alongside long labels", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    // Matrix Spaces still surface their concise outcome as an aria description,
    // unaffected by label expansion.
    for (const space of ["memory", "machines", "observatory"] as const) {
      const outcome = getTerm(space).outcome;
      expect(spaceOutcome(space)).toBe(outcome);
      const button = within(nav).getByRole("button", { name: LONG_LABELS[space] });
      const descId = button.getAttribute("aria-describedby");
      expect(descId).toBeTruthy();
      expect(nav.querySelector(`#${descId}`)?.textContent).toBe(outcome);
      // Tooltip combines the expanded label + outcome without breaking.
      expect(button.getAttribute("title")).toBe(`${LONG_LABELS[space]}: ${outcome}`);
    }
  });

  it("preserves one-click switching + aria-current with expanded labels", async () => {
    render(() => <Dock />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    fireEvent.click(within(nav).getByRole("button", { name: LONG_LABELS.observatory }));
    expect(currentRoute().space).toBe("observatory");

    const current = within(nav)
      .getAllByRole("button")
      .filter((b) => b.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0].getAttribute("aria-label")).toBe(LONG_LABELS.observatory);
  });
});
