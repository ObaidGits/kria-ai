import { describe, it, expect } from "vitest";
import { render } from "@solidjs/testing-library";
import Icon from "./Icon";

describe("Icon", () => {
  it("references the bundled Lucide sprite symbol via <use>", () => {
    const { container } = render(() => <Icon name="search" />);
    const use = container.querySelector("use");
    expect(use).toBeTruthy();
    expect(use?.getAttribute("href")).toBe("/icons/lucide-sprite.svg#search");
  });

  it("is decorative (aria-hidden) by default", () => {
    const { container } = render(() => <Icon name="bell" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("aria-hidden")).toBe("true");
    expect(svg?.getAttribute("role")).toBeNull();
  });

  it("exposes an accessible name when title is provided", () => {
    const { container } = render(() => <Icon name="bell" title="Notifications" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("aria-hidden")).toBeNull();
    expect(svg?.getAttribute("role")).toBe("img");
    expect(svg?.getAttribute("aria-label")).toBe("Notifications");
    expect(container.querySelector("title")?.textContent).toBe("Notifications");
  });

  it("defaults size to 1em and accepts a numeric px size", () => {
    const { container: c1 } = render(() => <Icon name="x" />);
    expect(c1.querySelector("svg")?.getAttribute("width")).toBe("1em");

    const { container: c2 } = render(() => <Icon name="x" size={20} />);
    expect(c2.querySelector("svg")?.getAttribute("width")).toBe("20px");
  });

  it("maps type-scale size roles to the matching font-size token", () => {
    const roles = {
      micro: "var(--font-size-micro)",
      caption: "var(--font-size-caption)",
      body: "var(--font-size-body)",
      heading: "var(--font-size-heading)",
      title: "var(--font-size-title)",
      display: "var(--font-size-display)",
    } as const;
    for (const [role, token] of Object.entries(roles)) {
      const { container } = render(() => <Icon name="x" size={role as keyof typeof roles} />);
      const svg = container.querySelector("svg");
      expect(svg?.getAttribute("width"), `role ${role} width`).toBe(token);
      expect(svg?.getAttribute("height"), `role ${role} height`).toBe(token);
    }
  });

  it("passes through an arbitrary CSS length that is not a role keyword", () => {
    const { container } = render(() => <Icon name="x" size="1.5rem" />);
    expect(container.querySelector("svg")?.getAttribute("width")).toBe("1.5rem");
  });

  it("uses a single consistent stroke weight (currentColor, stroke-width 2)", () => {
    const { container } = render(() => <Icon name="check" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("stroke")).toBe("currentColor");
    expect(svg?.getAttribute("stroke-width")).toBe("2");
  });
});
