/**
 * SkillCard action tests (task 8.2, Req 7.4 / 10.6).
 *
 * Proves enable/disable + uninstall dispatch through the injected handlers
 * (wired to `clawhub_toggle_skill` / `clawhub_uninstall_skill`) for an installed
 * skill, and that a not-installed skill shows no action controls (never a dead
 * control).
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@solidjs/testing-library";
import { SkillCard } from "./SkillCard";
import type { SkillView } from "../../../stores";

function skill(over: Partial<SkillView> = {}): SkillView {
  return {
    slug: "web-search",
    name: "Web Search",
    description: "Search the web",
    category: "web",
    trustTier: "community",
    installed: true,
    enabled: true,
    ...over,
  };
}

describe("SkillCard actions (Req 7.4)", () => {
  it("disables an enabled installed skill via the injected handler", async () => {
    const onToggle = vi.fn().mockResolvedValue({ ok: true, data: undefined });
    render(() => (
      <ul>
        <SkillCard skill={skill()} onToggle={onToggle} onUninstall={vi.fn()} />
      </ul>
    ));
    fireEvent.click(screen.getByRole("button", { name: /Disable/ }));
    await waitFor(() => expect(onToggle).toHaveBeenCalledWith("web-search", false));
  });

  it("uninstalls via the injected handler", async () => {
    const onUninstall = vi.fn().mockResolvedValue({ ok: true, data: undefined });
    render(() => (
      <ul>
        <SkillCard skill={skill()} onToggle={vi.fn()} onUninstall={onUninstall} />
      </ul>
    ));
    fireEvent.click(screen.getByRole("button", { name: /Uninstall/ }));
    await waitFor(() => expect(onUninstall).toHaveBeenCalledWith("web-search"));
  });

  it("shows no action controls for a not-installed skill (no dead control, Req 10.6)", () => {
    render(() => (
      <ul>
        <SkillCard skill={skill({ installed: false })} onToggle={vi.fn()} onUninstall={vi.fn()} />
      </ul>
    ));
    expect(screen.queryByRole("button", { name: /Enable|Disable/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Uninstall/ })).toBeNull();
  });
});
