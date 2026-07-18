import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";
import CapabilitiesSpace from "./CapabilitiesSpace";
import { capabilityStore, shellStore } from "../../stores";
import type { Capability, SkillView, Provider, IntegrationView } from "../../stores";
import { navigate, currentRoute } from "../router";

function cap(id: string, name: string, over: Partial<Capability> = {}): Capability {
  return {
    id,
    name,
    type: "tool",
    status: "active",
    description: "",
    source: "prov",
    riskLevel: "green",
    providerId: "prov",
    capabilityId: id,
    tags: [],
    elevated: false,
    ...over,
  };
}

describe("CapabilitiesSpace — segments + descriptor Inspector (task 8.1, Req 7.1/7.2)", () => {
  beforeEach(() => {
    // The Space loads each segment on mount; stub it so seeded data survives
    // and the honest loading flag stays settled.
    vi.spyOn(capabilityStore, "loadSegment").mockResolvedValue(undefined);
    capabilityStore.setCapabilities([]);
    capabilityStore.setSkills([]);
    capabilityStore.setProviders([]);
    capabilityStore.setModels([]);
    capabilityStore.setIntegrations([]);
    capabilityStore.setGrants([]);
    capabilityStore.setProposals([]);
    capabilityStore.setActiveSegment("tools");
    shellStore.setInspectorTarget(null);
    navigate("capabilities");
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders a tablist with all six segments (Req 7.1)", () => {
    render(() => <CapabilitiesSpace />);
    expect(screen.getByRole("tablist")).toBeInTheDocument();
    for (const name of ["Tools", "Skills", "Models", "Integrations", "Governance", "Generate"]) {
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    }
  });

  it("defaults to the Tools segment", () => {
    render(() => <CapabilitiesSpace />);
    expect(screen.getByRole("tab", { name: "Tools" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("heading", { name: "Tools" })).toBeInTheDocument();
  });

  it("lists tool capabilities where data exists", () => {
    capabilityStore.setCapabilities([cap("web", "Web search"), cap("files", "File read")]);
    render(() => <CapabilitiesSpace />);
    expect(screen.getByText("Web search")).toBeInTheDocument();
    expect(screen.getByText("File read")).toBeInTheDocument();
  });

  it("filters tools by the search box", () => {
    capabilityStore.setCapabilities([cap("web", "Web search"), cap("files", "File read")]);
    render(() => <CapabilitiesSpace />);
    const search = screen.getByRole("searchbox", { name: "Search tools" });
    fireEvent.input(search, { target: { value: "web" } });
    expect(screen.getByText("Web search")).toBeInTheDocument();
    expect(screen.queryByText("File read")).toBeNull();
  });

  it("routes the segment via the typed router and swaps the region (Req 1.5/7.1)", () => {
    render(() => <CapabilitiesSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Skills" }));
    expect(currentRoute().space).toBe("capabilities");
    expect(currentRoute().segment).toBe("skills");
    expect(capabilityStore.activeSegment()).toBe("skills");
    expect(screen.getByRole("heading", { name: "Skills" })).toBeInTheDocument();
  });

  it("opens a deep-linked capability in the shared Inspector", async () => {
    capabilityStore.setCapabilities([cap("web", "Web search")]);
    navigate("capabilities", "tools", "web");
    render(() => <CapabilitiesSpace />);

    await waitFor(() => {
      expect(shellStore.inspectorTarget()?.type).toBe("capability");
      expect(shellStore.inspectorTarget()?.id).toBe("web");
    });
  });

  it("reacts to provider deep links while mounted and focuses the provider", async () => {
    const provider: Provider = { id: "local", name: "Local llama", type: "local", active: true };
    capabilityStore.setProviders([provider]);
    render(() => <CapabilitiesSpace />);
    navigate("capabilities", "models", "local");

    await waitFor(() => {
      expect(screen.getByRole("tab", { name: "Models" })).toHaveAttribute("aria-selected", "true");
      expect(document.activeElement).toBe(
        document.querySelector<HTMLElement>('[data-provider-id="local"]'),
      );
    });
  });

  it("shows a skill with its trust tier in the Skills segment", () => {
    const skill: SkillView = {
      slug: "pdf",
      name: "PDF reader",
      description: "reads pdfs",
      category: "productivity",
      trustTier: "verified",
      installed: true,
      enabled: true,
    };
    capabilityStore.setSkills([skill]);
    render(() => <CapabilitiesSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Skills" }));
    expect(screen.getByText("PDF reader")).toBeInTheDocument();
    expect(screen.getByText("Verified")).toBeInTheDocument();
  });

  it("shows providers in the Models segment", () => {
    const provider: Provider = { id: "local", name: "Local llama", type: "local", active: true };
    capabilityStore.setProviders([provider]);
    render(() => <CapabilitiesSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Models" }));
    expect(screen.getByText("Local llama")).toBeInTheDocument();
  });

  it("shows integrations with honest connection state", () => {
    const integration: IntegrationView = {
      id: "mcp:x",
      name: "Filesystem MCP",
      kind: "mcp",
      status: "connected",
      detail: "3 tools",
    };
    capabilityStore.setIntegrations([integration]);
    render(() => <CapabilitiesSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Integrations" }));
    expect(screen.getByText("Filesystem MCP")).toBeInTheDocument();
    expect(screen.getByText("3 tools")).toBeInTheDocument();
  });

  it("shows an honest empty state in Tools when there are no tools", () => {
    render(() => <CapabilitiesSpace />);
    expect(screen.getByRole("heading", { name: "No tools" })).toBeInTheDocument();
  });

  it("shows an honest empty state in Governance when nothing to govern", () => {
    render(() => <CapabilitiesSpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Governance" }));
    expect(screen.getByRole("heading", { name: "Nothing to govern yet" })).toBeInTheDocument();
  });

  it("selecting a tool opens the shared Inspector on its descriptor (Req 7.2)", () => {
    capabilityStore.setCapabilities([cap("web", "Web search")]);
    render(() => <CapabilitiesSpace />);
    fireEvent.click(screen.getByRole("button", { name: /Web search/ }));
    expect(shellStore.inspectorTarget()?.type).toBe("capability");
    expect(shellStore.inspectorTarget()?.id).toBe("web");
  });
});
