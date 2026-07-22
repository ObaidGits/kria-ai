import { expect, test } from "./fixtures";

const categoryLabels = [
  ["Project Atlas", "Release roadmap", "Workspace redesign", "Build pipeline", "Project KRIA", "Product research"],
  ["Knowledge architecture", "Memory systems", "Rust patterns", "Research notes", "Document intelligence", "Local AI models"],
  ["Goal: flagship UX", "Milestone beta", "Plan Q3", "Target quality", "Objective privacy"],
  ["Skill automation", "Agent orchestration", "Tool routing", "Capability graph", "Workflow builder", "Skill discovery"],
  ["Event launch", "Calendar sync", "Meeting product", "Task review", "Schedule research", "Event planning"],
  ["Idea semantic lens", "Concept living graph", "Insight clusters", "Thought ambient AI", "Hypothesis memory", "Idea camera", "Concept discovery"],
  ["Person Obaid", "People design team", "User researcher", "Client feedback", "Team collaborator"],
  ["Conversation roadmap", "Chat memory", "Discussion architecture", "Session planning", "Message feedback", "Conversation research"],
] as const;

function installMemoryGraphBackend(labelsByCategory: readonly (readonly string[])[]) {
  const backend = (window as any).__KRIA_E2E_BACKEND__;
  const originalInvoke = backend.invoke.bind(backend);
  const nodes: Array<{ entity: string; display_name: string; degree: number }> = [];
  const communities: string[][] = [];
  labelsByCategory.forEach((labels, categoryIndex) => {
    const community: string[] = [];
    labels.forEach((label, nodeIndex) => {
      const entity = `00000000-0000-4000-8${categoryIndex}00-${String(nodeIndex + 1).padStart(12, "0")}`;
      community.push(entity);
      nodes.push({ entity, display_name: label, degree: 18 - nodeIndex + categoryIndex });
    });
    communities.push(community);
  });
  backend.invoke = async (command: string, args?: Record<string, unknown>) => {
    if (command === "memory_graph_centrality") return { nodes, count: nodes.length };
    if (command === "memory_graph_communities") return { communities, count: communities.length };
    if (command === "memory_graph_relationships") {
      const source = String(args?.entityId ?? "");
      const community = communities.find((group) => group.includes(source)) ?? [];
      return community.filter((id) => id !== source).slice(0, 3).map((target_id) => ({ source_id: source, target_id, rel_type: "related_to" }));
    }
    if (command === "memory_graph_predict_links") {
      const source = String(args?.entityId ?? "");
      const flat = communities.flat().filter((id) => id !== source);
      const target = flat[(flat.indexOf(source) + 17 + flat.length) % flat.length] ?? flat[0];
      const match = nodes.find((node) => node.entity === target);
      return { predictions: target ? [{ target, display_name: match?.display_name ?? "Suggested memory", score: 0.86, shared_neighbors: 4 }] : [], count: target ? 1 : 0 };
    }
    if (command === "memory_graph_create_relationship") return "00000000-0000-4000-8000-999999999999";
    return originalInvoke(command, args);
  };
}

test.describe("Memory Graph current 2D view", () => {
  test("separates generated navigation facets from authority relationships", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 980 });
    await page.goto("/?e2e=1");
    await page.evaluate(installMemoryGraphBackend, categoryLabels);
    await page.getByRole("button", { name: "Memory", exact: true }).click();
    await page.getByRole("tab", { name: "Knowledge Graph" }).click();

    const universe = page.locator(".memory-universe");
    await expect(universe).toBeVisible();
    await expect(universe.locator('.memory-universe__hub[data-authority-class="navigation"][data-generated="true"]')).toHaveCount(9);
    await expect(universe.locator(".memory-universe__core")).toBeVisible();
    await expect(universe.locator(".memory-universe__memory")).toHaveCount(categoryLabels.flat().length);

    const generatedEmphasis = universe.getByRole("button", { name: "Generated facets" });
    const relationshipEmphasis = universe.getByRole("button", { name: "Relationships" });
    await expect(generatedEmphasis).toHaveAttribute("aria-pressed", "true");
    await expect(relationshipEmphasis).toHaveAttribute("aria-pressed", "false");
    await relationshipEmphasis.click();
    await expect(generatedEmphasis).toHaveAttribute("aria-pressed", "false");
    await expect(relationshipEmphasis).toHaveAttribute("aria-pressed", "true");

    const camera = universe.getByRole("group", { name: "Camera controls" });
    await expect(camera.getByRole("button")).toHaveCount(3);
    await expect(camera.getByRole("button", { name: "Reset view" })).toBeVisible();
    await expect(universe.getByRole("button", { name: /Timeline|Auto arrange|Center graph|Pin memory/ })).toHaveCount(0);
    await expect(universe.locator(".memory-universe__search kbd")).toHaveCount(0);

    // Generated facets use labeled containment hulls, never line-like hub/spoke edges.
    await expect(universe.locator(".memory-universe__core-links, .memory-universe__satellite-links")).toHaveCount(0);
    await expect(universe.getByText("Generated navigation facet", { exact: true })).toBeVisible();
    await expect(universe.getByText("Strong connection", { exact: true })).toHaveCount(0);
    await expect(universe.getByText("Weak connection", { exact: true })).toHaveCount(0);

    await page.waitForTimeout(900);
    await universe.screenshot({ path: "test-results/memory-universe-final.png" });

    await universe.locator('[aria-label^="Generated navigation facet Projects,"]').click();
    await expect(page.getByRole("complementary", { name: /Details for Projects/ })).toBeVisible();
    await expect(page.getByText("GRAPH DETAILS")).toBeVisible();
    await expect(page.getByText(/Facet membership is not a stored relationship/)).toBeVisible();
    await page.waitForTimeout(500);
    await universe.screenshot({ path: "test-results/memory-universe-inspector.png" });

    // Backend relationships use the authority edge layer and disclose their class.
    await universe.locator(".memory-universe__memory").first().click();
    await expect(universe.locator('[data-authority-class="stored"]')).toHaveCount(3);
    await expect(universe.locator('[data-authority-class="inferred"]')).toHaveCount(1);
    await expect(universe.getByText("Stored relationship", { exact: true })).toBeVisible();
    await expect(universe.getByText("Inferred candidate", { exact: true })).toBeVisible();
  });
});
