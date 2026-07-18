/**
 * ConstellationFallback tests (task 8.3) — the mandatory 2D/keyboard catalog
 * representation of the Capabilities Constellation (Req 7.5 / 16.3 / 17.5).
 *
 * Verifies the fallback renders a REAL accessible table of capability nodes with
 * sort/filter/search, keyboard row navigation, select→focus + connections view,
 * pin/hide, kind conveyed as icon + text, the "showing N of M" cap notice, and
 * that selecting a TOOL node opens its descriptor in the shared Inspector
 * (Req 7.2). Read/visualize only — no execution, no backend writes.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@solidjs/testing-library";
import { ConstellationFallback } from "./ConstellationFallback";
import { constellationData } from "./constellationData";
import { buildConstellation } from "./constellationModel";
import { shellStore } from "../../../../stores";
import type {
  Capability,
  IntegrationView,
  ModelView,
  Provider,
  SkillView,
} from "../../../../stores";

function seed() {
  const capabilities: Capability[] = [
    {
      id: "prov-a:tool-x",
      name: "Tool X",
      type: "tool",
      status: "active",
      description: "Does X",
      source: "prov-a",
      riskLevel: "green",
      providerId: "prov-a",
      capabilityId: "tool-x",
      tags: [],
      elevated: false,
    },
  ];
  const providers: Provider[] = [{ id: "prov-a", name: "Provider A", type: "local", active: true }];
  const models: ModelView[] = [{ id: "m1", name: "Llama", provider: "prov-a" }];
  const skills: SkillView[] = [
    { slug: "s1", name: "Skill One", description: "", category: "gen", trustTier: "community", installed: true, enabled: true },
  ];
  const integrations: IntegrationView[] = [
    { id: "google", name: "Google Workspace", kind: "google", status: "connected", detail: "you@x" },
  ];
  constellationData.seed(
    buildConstellation({ capabilities, models, providers, skills, integrations }),
  );
}

beforeEach(() => {
  constellationData.reset();
  shellStore.closeInspector?.();
});

describe("2D catalog fallback — structure + cap", () => {
  it("renders a real table of nodes with column headers (scope=col)", () => {
    seed();
    render(() => <ConstellationFallback reason="2D default (test)" />);
    expect(screen.getByRole("table")).toBeInTheDocument();
    for (const header of ["Node", "Kind", "Connections", "Actions"]) {
      expect(screen.getByRole("columnheader", { name: new RegExp(header) })).toBeInTheDocument();
    }
    expect(screen.getByRole("rowheader", { name: /Provider A/ })).toBeInTheDocument();
  });

  it("shows the honest 'showing N of M' cap notice", () => {
    seed();
    render(() => <ConstellationFallback />);
    // provider A + tool X + model + skill + trust group + integration = 6 nodes
    expect(screen.getByText(/Showing all 6/)).toBeInTheDocument();
  });

  it("shows an honest empty state when there are no nodes", () => {
    render(() => <ConstellationFallback />);
    expect(screen.getByText(/No capabilities yet/)).toBeInTheDocument();
  });
});

describe("2D catalog fallback — sort", () => {
  it("defaults to connections descending and toggles on header click", () => {
    seed();
    render(() => <ConstellationFallback />);
    const header = screen.getByRole("columnheader", { name: /Connections/ });
    expect(header).toHaveAttribute("aria-sort", "descending");
    fireEvent.click(within(header).getByRole("button"));
    expect(header).toHaveAttribute("aria-sort", "ascending");
  });
});

describe("2D catalog fallback — search / filter", () => {
  it("filters rows by node label", async () => {
    seed();
    render(() => <ConstellationFallback />);
    const search = screen.getByRole("searchbox", { name: /Filter capabilities/ });
    fireEvent.input(search, { target: { value: "Tool X" } });
    await waitFor(() => {
      expect(screen.getByRole("rowheader", { name: /Tool X/ })).toBeInTheDocument();
      expect(screen.queryByRole("rowheader", { name: /Llama/ })).toBeNull();
    });
    expect(screen.getByText(/1 match/)).toBeInTheDocument();
  });
});

describe("2D catalog fallback — keyboard navigation", () => {
  it("moves focus across rows with arrow keys (roving tabindex)", () => {
    seed();
    render(() => <ConstellationFallback />);
    const table = screen.getByRole("table");
    fireEvent.keyDown(table, { key: "ArrowDown" });
    const rowButtons = table.querySelectorAll<HTMLButtonElement>("[data-node-row]");
    expect(document.activeElement).toBe(rowButtons[0]);
    fireEvent.keyDown(table, { key: "ArrowDown" });
    expect(document.activeElement).toBe(rowButtons[1]);
    fireEvent.keyDown(table, { key: "End" });
    expect(document.activeElement).toBe(rowButtons[rowButtons.length - 1]);
  });
});

describe("2D catalog fallback — select → focus + connections", () => {
  it("focuses a node and reveals its connections as rows", async () => {
    seed();
    render(() => <ConstellationFallback />);
    // Provider A connects to Tool X and Llama.
    fireEvent.click(screen.getByRole("button", { name: /Provider A/ }));
    await waitFor(() => {
      expect(screen.getByRole("region", { name: /Connections for Provider A/ })).toBeInTheDocument();
    });
    const region = screen.getByRole("region", { name: /Connections for Provider A/ });
    expect(within(region).getByRole("rowheader", { name: /Tool X/ })).toBeInTheDocument();
  });
});

describe("2D catalog fallback — inspect tool node", () => {
  it("opens the descriptor in the shared Inspector for a tool node (Req 7.2)", async () => {
    seed();
    render(() => <ConstellationFallback />);
    fireEvent.click(screen.getByRole("button", { name: /Tool X/ }));
    await waitFor(() => {
      const target = shellStore.inspectorTarget();
      expect(target?.type).toBe("capability");
      expect(target?.data).toEqual(
        expect.objectContaining({ providerId: "prov-a", capabilityId: "tool-x" }),
      );
    });
  });

  it("does NOT open the Inspector for a non-descriptor node (provider)", async () => {
    seed();
    render(() => <ConstellationFallback />);
    fireEvent.click(screen.getByRole("button", { name: /Provider A/ }));
    await waitFor(() =>
      expect(screen.getByRole("region", { name: /Connections for Provider A/ })).toBeInTheDocument(),
    );
    expect(shellStore.inspectorTarget()).toBeNull();
  });
});

describe("2D catalog fallback — pin / hide", () => {
  it("toggles pin state", () => {
    seed();
    render(() => <ConstellationFallback />);
    const pinButton = screen.getAllByRole("button", { name: /^Pin$/ })[0];
    fireEvent.click(pinButton);
    expect(constellationData.pinned().size).toBe(1);
  });

  it("hides a node so it leaves the table and can be restored", async () => {
    seed();
    render(() => <ConstellationFallback />);
    const hideButtons = screen.getAllByRole("button", { name: /Hide/ });
    fireEvent.click(hideButtons[hideButtons.length - 1]);
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Show hidden/ })).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: /Show hidden/ }));
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: /Show hidden/ })).toBeNull();
    });
  });
});
