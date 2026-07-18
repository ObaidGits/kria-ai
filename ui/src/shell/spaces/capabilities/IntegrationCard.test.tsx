/**
 * IntegrationCard connect tests (task 8.2, Req 7.4 / 10.6 / 20.4).
 *
 * Proves a disconnected/errored integration offers a Connect/Retry control
 * routed through the injected handler, a connected one offers none, and no
 * connect control is shown when no handler is provided (never a dead control).
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@solidjs/testing-library";
import { IntegrationCard } from "./IntegrationCard";
import type { IntegrationView } from "../../../stores";

function integration(over: Partial<IntegrationView> = {}): IntegrationView {
  return {
    id: "google",
    name: "Google Workspace",
    kind: "google",
    status: "disconnected",
    detail: "Not connected",
    ...over,
  };
}

describe("IntegrationCard connect (Req 7.4)", () => {
  it("routes a connect request through the injected handler", async () => {
    const onConnect = vi.fn().mockResolvedValue(undefined);
    render(() => (
      <ul>
        <IntegrationCard integration={integration()} onConnect={onConnect} />
      </ul>
    ));
    fireEvent.click(screen.getByRole("button", { name: /Connect/ }));
    await waitFor(() => expect(onConnect).toHaveBeenCalledTimes(1));
  });

  it("labels the action Retry when the integration errored", () => {
    render(() => (
      <ul>
        <IntegrationCard integration={integration({ status: "error", detail: "boom" })} onConnect={vi.fn()} />
      </ul>
    ));
    expect(screen.getByRole("button", { name: /Retry/ })).toBeInTheDocument();
  });

  it("offers no connect control for a connected integration", () => {
    render(() => (
      <ul>
        <IntegrationCard integration={integration({ status: "connected", detail: "ok" })} onConnect={vi.fn()} />
      </ul>
    ));
    expect(screen.queryByRole("button", { name: /Connect|Retry/ })).toBeNull();
  });

  it("shows no connect control without a handler (no dead control, Req 10.6)", () => {
    render(() => (
      <ul>
        <IntegrationCard integration={integration()} />
      </ul>
    ));
    expect(screen.queryByRole("button", { name: /Connect|Retry/ })).toBeNull();
  });
});
