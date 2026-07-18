import { describe, it, expect } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import { Tooltip } from "./Tooltip";

// Tooltip content is portalled to document.body, so query via `screen`.
describe("Tooltip", () => {
  it("renders its trigger", () => {
    const { getByText } = render(() => (
      <Tooltip content="Hint">
        <button>Trigger</button>
      </Tooltip>
    ));
    expect(getByText("Trigger")).toBeInTheDocument();
  });

  it("shows the tooltip content when open", () => {
    render(() => (
      <Tooltip content="Helpful hint" open openDelay={0}>
        <button>Trigger</button>
      </Tooltip>
    ));
    expect(screen.getByRole("tooltip")).toHaveTextContent("Helpful hint");
  });

  it("associates the trigger with the tooltip via aria-describedby when open", () => {
    render(() => (
      <Tooltip content="Desc" open openDelay={0}>
        <button>Trigger</button>
      </Tooltip>
    ));
    const tips = screen.getAllByRole("tooltip");
    const tip = tips[tips.length - 1];
    const trigger = document.querySelector(`[aria-describedby="${tip.id}"]`);
    expect(trigger).toBeTruthy();
    expect(trigger).toHaveTextContent("Trigger");
  });
});
