/**
 * PresencePreferencesPanel — canonical Companion opt-out host (design §17, Req 19).
 *
 * Verifies Settings is the single host for the Companion Mode opt-out: the
 * toggle reflects the injected preference and flips it (on-by-default, one-
 * setting opt-out — Req 15.4). Uses an injected preference stub so no
 * localStorage is touched.
 */
import { describe, it, expect, afterEach } from "vitest";
import { render, cleanup, fireEvent } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

import PresencePreferencesPanel from "./PresencePreferencesPanel";
import { companionPreference } from "../home/companionEmber";

afterEach(cleanup);

/** A controllable companion-preference stub matching the real store's shape. */
function stubPreference(initial = true): typeof companionPreference {
  const [enabled, setEnabled] = createSignal(initial);
  return {
    enabled,
    setEnabled: (value: boolean) => setEnabled(value),
    refresh: () => {},
  };
}

describe("PresencePreferencesPanel — companion opt-out (Req 19, 15.4)", () => {
  it("reflects the on-by-default companion preference", () => {
    const preference = stubPreference(true);
    const { getByRole } = render(() => <PresencePreferencesPanel preference={preference} />);
    const toggle = getByRole("switch", { name: /Companion Mode/ });
    expect((toggle as HTMLInputElement).checked).toBe(true);
  });

  it("opts out (and back in) through the single canonical toggle", () => {
    const preference = stubPreference(true);
    const { getByRole } = render(() => <PresencePreferencesPanel preference={preference} />);
    const toggle = getByRole("switch", { name: /Companion Mode/ }) as HTMLInputElement;

    fireEvent.click(toggle);
    expect(preference.enabled()).toBe(false);

    fireEvent.click(toggle);
    expect(preference.enabled()).toBe(true);
  });
});
