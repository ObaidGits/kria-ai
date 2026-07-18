/**
 * ProviderCard action tests (task 8.2, Req 7.4).
 *
 * Proves the switch + test controls dispatch through the injected handlers
 * (wired to `switch_provider` / `test_provider_connection_cmd`) and surface an
 * honest reachable / error result. The active provider's Switch is disabled.
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@solidjs/testing-library";
import { ProviderCard } from "./ProviderCard";
import type { Provider } from "../../../stores";

function provider(over: Partial<Provider> = {}): Provider {
  return { id: "openai", name: "OpenAI", type: "cloud", active: false, ...over };
}

describe("ProviderCard actions (Req 7.4)", () => {
  it("switches an inactive provider via the injected handler", async () => {
    const onSwitch = vi.fn().mockResolvedValue({ ok: true, data: undefined });
    render(() => <ProviderCard provider={provider()} models={[]} onSwitch={onSwitch} onTest={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /Switch to/ }));
    await waitFor(() => expect(onSwitch).toHaveBeenCalledWith("openai"));
  });

  it("disables Switch for the active provider (no dead control)", () => {
    render(() => <ProviderCard provider={provider({ active: true })} models={[]} onSwitch={vi.fn()} onTest={vi.fn()} />);
    expect(screen.getByRole("button", { name: /Active/ })).toBeDisabled();
  });

  it("surfaces a reachable result after a successful test", async () => {
    const onTest = vi.fn().mockResolvedValue({ ok: true, data: {} });
    render(() => <ProviderCard provider={provider()} models={[]} onSwitch={vi.fn()} onTest={onTest} />);
    fireEvent.click(screen.getByRole("button", { name: /Test/ }));
    await waitFor(() => expect(screen.getByText("Reachable")).toBeInTheDocument());
  });

  it("surfaces the error message after a failed test", async () => {
    const onTest = vi.fn().mockResolvedValue({ ok: false, message: "unreachable host" });
    render(() => <ProviderCard provider={provider()} models={[]} onSwitch={vi.fn()} onTest={onTest} />);
    fireEvent.click(screen.getByRole("button", { name: /Test/ }));
    await waitFor(() => expect(screen.getByText("unreachable host")).toBeInTheDocument());
  });
});
