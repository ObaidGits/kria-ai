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

  it("uses a single consistent stroke weight (currentColor, stroke-width 2)", () => {
    const { container } = render(() => <Icon name="check" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("stroke")).toBe("currentColor");
    expect(svg?.getAttribute("stroke-width")).toBe("2");
  });
});
