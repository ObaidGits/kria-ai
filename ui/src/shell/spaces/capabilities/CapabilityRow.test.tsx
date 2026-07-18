import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { CapabilityRow } from "./CapabilityRow";
import { shellStore, type Capability } from "../../../stores";

function cap(over: Partial<Capability> = {}): Capability {
  return {
    id: "prov:web-search",
    name: "Web search",
    type: "tool",
    status: "active",
    description: "Search the web",
    source: "prov",
    riskLevel: "green",
    providerId: "prov",
    capabilityId: "web-search",
    tags: ["web"],
    elevated: false,
    ...over,
  };
}

describe("CapabilityRow (task 8.1, Req 7.1/7.2/17.3)", () => {
  beforeEach(() => shellStore.setInspectorTarget(null));
  afterEach(() => cleanup());

  it("renders the capability name + description as text", () => {
    render(() => <CapabilityRow capability={cap()} />);
    expect(screen.getByText("Web search")).toBeInTheDocument();
    expect(screen.getByText("Search the web")).toBeInTheDocument();
  });

  it("shows risk as icon + text, never color alone (Req 17.3)", () => {
    render(() => <CapabilityRow capability={cap({ riskLevel: "yellow", elevated: true })} />);
    expect(screen.getByText("Elevated")).toBeInTheDocument();
  });

  it("is a keyboard-operable button", () => {
    render(() => <CapabilityRow capability={cap()} />);
    expect(screen.getByRole("button")).toBeInTheDocument();
  });

  it("opens the shared Inspector on the capability descriptor (Req 1.6/7.2)", () => {
    render(() => <CapabilityRow capability={cap({ id: "p:c" })} />);
    fireEvent.click(screen.getByRole("button"));
    const t = shellStore.inspectorTarget();
    expect(t?.type).toBe("capability");
    expect(t?.id).toBe("p:c");
    expect((t?.data as { providerId: string }).providerId).toBe("prov");
    expect((t?.data as { capabilityId: string }).capabilityId).toBe("web-search");
  });

  it("uses an onInspect override when provided (stories/tests)", () => {
    let inspected: string | null = null;
    render(() => (
      <CapabilityRow capability={cap({ id: "z9" })} onInspect={(c) => (inspected = c.id)} />
    ));
    fireEvent.click(screen.getByRole("button"));
    expect(inspected).toBe("z9");
    // Override means the shared inspector target is NOT mutated here.
    expect(shellStore.inspectorTarget()).toBeNull();
  });
});
