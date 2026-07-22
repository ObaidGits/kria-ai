/**
 * §20.4 focus-fallback ladder for the controlled/opener owner (gap G4).
 *
 * Mirrors the approvalPlace fallback expectations (task 8.3) but for the
 * synchronous ModalHost opener owner: opener → owning region heading → region
 * container → #space-root → stable shell control, never a destructive anchor.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { captureFocusOwner, resolveFocusReturnTarget } from "./focusReturn";

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string> = {},
  parent: HTMLElement = document.body,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  parent.appendChild(node);
  return node;
}

describe("focusReturn — §20.4 fallback ladder (gap G4)", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("returns to the opener when it is still connected and non-destructive", () => {
    const region = el("section");
    const opener = el("button", { type: "button" }, region);
    const owner = captureFocusOwner(opener);
    expect(resolveFocusReturnTarget(owner)).toBe(opener);
  });

  it("falls back to the owning region heading when the opener is removed", () => {
    const region = el("section");
    const heading = el("h2", {}, region);
    const opener = el("button", { type: "button" }, region);
    const owner = captureFocusOwner(opener);
    opener.remove(); // opener gone while modal was up
    expect(resolveFocusReturnTarget(owner)).toBe(heading);
  });

  it("falls back to the owning region container when there is no heading", () => {
    const region = el("section");
    const opener = el("button", { type: "button" }, region);
    const owner = captureFocusOwner(opener);
    opener.remove();
    const target = resolveFocusReturnTarget(owner);
    expect(target).toBe(region);
    expect(region.getAttribute("tabindex")).toBe("-1"); // made focusable
  });

  it("falls back to #space-root when the region is also gone", () => {
    const spaceRoot = el("div", { id: "space-root", tabindex: "-1" });
    const region = el("section");
    const opener = el("button", { type: "button" }, region);
    const owner = captureFocusOwner(opener);
    region.remove();
    expect(resolveFocusReturnTarget(owner)).toBe(spaceRoot);
  });

  it("falls back to a stable shell control when #space-root is absent", () => {
    const shell = el("div", { "data-shell-root": "" });
    const region = el("section");
    const opener = el("button", { type: "button" }, region);
    const owner = captureFocusOwner(opener);
    region.remove();
    const target = resolveFocusReturnTarget(owner);
    expect(target).toBe(shell);
  });

  it("never lands on a destructive opener for a generic modal", () => {
    const region = el("section");
    const heading = el("h2", {}, region);
    const approve = el("button", { type: "button", class: "kria-approval-card__approve" }, region);
    const owner = captureFocusOwner(approve);
    // Generic modal: destructive opener is skipped → owning region heading.
    expect(resolveFocusReturnTarget(owner)).toBe(heading);
  });

  it("allows returning to a destructive opener when it is the designated owner", () => {
    const region = el("section");
    el("h2", {}, region);
    const approve = el("button", { type: "button", class: "kria-approval-card__approve" }, region);
    const owner = captureFocusOwner(approve);
    // approval-confirm: §20.3 owner IS the originating decision control.
    expect(resolveFocusReturnTarget(owner, { allowDestructiveOpener: true })).toBe(approve);
  });

  it("skips a destructive owning-region container in the fallback", () => {
    const region = el("section", { "data-destructive": "true" });
    const opener = el("button", { type: "button" }, region);
    const spaceRoot = el("div", { id: "space-root", tabindex: "-1" });
    const owner = captureFocusOwner(opener);
    opener.remove();
    // Region survives but is destructive → skip to #space-root.
    expect(resolveFocusReturnTarget(owner)).toBe(spaceRoot);
  });
});
