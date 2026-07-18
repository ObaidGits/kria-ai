import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup, waitFor } from "@solidjs/testing-library";
import { DescriptorInspector } from "./DescriptorInspector";
import type { CapabilityDescriptor } from "../../../stores";
import type { InspectorTarget } from "../../../stores/shellStore";

function descriptor(over: Partial<CapabilityDescriptor> = {}): CapabilityDescriptor {
  return {
    providerId: "prov",
    capabilityId: "web-search",
    name: "Web search",
    description: "Searches the web and returns results.",
    version: "1.2.0",
    schemaVersion: "1",
    tags: ["web", "search"],
    ioModality: ["text"],
    inputs: ["query"],
    outputs: ["results"],
    effectClasses: ["network.read"],
    reversible: "yes",
    idempotent: true,
    elevated: false,
    trustTier: "verified",
    signed: true,
    inputSchema: { type: "object", properties: { query: { type: "string" } } },
    ...over,
  };
}

const target: InspectorTarget = {
  type: "capability",
  id: "prov:web-search",
  data: { providerId: "prov", capabilityId: "web-search", name: "Web search" },
};

describe("DescriptorInspector (task 8.1, Req 7.2)", () => {
  afterEach(() => cleanup());

  it("discloses descriptor, effects, trust tier, and schema (Req 7.2)", async () => {
    render(() => (
      <DescriptorInspector
        target={target}
        fetch={async () => ({ ok: true, data: descriptor() })}
      />
    ));

    await waitFor(() => expect(screen.getByText("Searches the web and returns results.")).toBeInTheDocument());

    // Descriptor
    expect(screen.getByText("Descriptor")).toBeInTheDocument();
    // Effects
    expect(screen.getByText("network.read")).toBeInTheDocument();
    // Trust tier (icon + text)
    expect(screen.getByText("Tier: verified")).toBeInTheDocument();
    expect(screen.getByText("Signed")).toBeInTheDocument();
    // Schema — pretty-printed as escaped text
    expect(screen.getByText(/"query"/)).toBeInTheDocument();
  });

  it("marks an untrusted/unsigned capability honestly", async () => {
    render(() => (
      <DescriptorInspector
        target={target}
        fetch={async () => ({ ok: true, data: descriptor({ trustTier: null, signed: false }) })}
      />
    ));
    await waitFor(() => expect(screen.getByText("Untrusted")).toBeInTheDocument());
    expect(screen.getByText("Unsigned")).toBeInTheDocument();
  });

  it("shows an honest error state when the descriptor cannot load (Req 20.4)", async () => {
    render(() => (
      <DescriptorInspector
        target={target}
        fetch={async () => ({ ok: false, message: "descriptor unavailable" })}
      />
    ));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("descriptor unavailable"));
  });
});
