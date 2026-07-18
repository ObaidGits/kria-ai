import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onCleanup } from "solid-js";
import { ConstellationFallback } from "./ConstellationFallback";
import { constellationData } from "./constellationData";
import { buildConstellation } from "./constellationModel";
import { LensModeToggle } from "../../../../platform/LensModeToggle";
import { initRenderMode } from "../../../../platform/renderMode";
import type {
  Capability,
  IntegrationView,
  ModelView,
  Provider,
  SkillView,
} from "../../../../stores";
import "../../memory/graph/KnowledgeGraphLens.css";
import "./constellation.css";

/**
 * Capabilities Constellation lens (task 8.3). The 3D scene needs WebGL, which
 * the docs workbench can't guarantee (and jsdom can't run at all), so these
 * stories exercise the ALWAYS-AVAILABLE 2D catalog representation — the
 * mandatory fallback + the DEFAULT on WebKitGTK (design.md §11.2 / task 0.6) —
 * plus the manual 2D/3D toggle. Each story seeds the constellationData
 * read-model directly (no bridge/Tauri needed).
 */
const meta = {
  title: "Spaces/Capabilities/ConstellationLens",
  component: ConstellationFallback,
  decorators: [
    (Story: () => unknown) => {
      onCleanup(() => constellationData.reset());
      return (
        <div
          class="kria-shell"
          data-window-mode="standard"
          style={{ height: "600px", padding: "24px" }}
        >
          {Story() as never}
        </div>
      );
    },
  ],
} satisfies Meta<typeof ConstellationFallback>;

export default meta;
type Story = StoryObj<typeof meta>;

function seed() {
  const capabilities: Capability[] = [
    { id: "files:read", name: "Read file", type: "tool", status: "active", description: "Read a file", source: "files", riskLevel: "green", providerId: "files", capabilityId: "read", tags: [], elevated: false },
    { id: "files:write", name: "Write file", type: "tool", status: "active", description: "Write a file", source: "files", riskLevel: "yellow", providerId: "files", capabilityId: "write", tags: [], elevated: true },
    { id: "web:search", name: "Web search", type: "tool", status: "active", description: "Search the web", source: "web", riskLevel: "green", providerId: "web", capabilityId: "search", tags: [], elevated: false },
  ];
  const providers: Provider[] = [
    { id: "local", name: "Local (llama.cpp)", type: "local", active: true },
    { id: "files", name: "Files provider", type: "local", active: true },
    { id: "web", name: "Web provider", type: "local", active: true },
  ];
  const models: ModelView[] = [
    { id: "qwen", name: "Qwen 2.5", provider: "local", detail: "32k ctx" },
    { id: "llama", name: "Llama 3.1", provider: "local", detail: "8k ctx" },
  ];
  const skills: SkillView[] = [
    { slug: "pdf", name: "PDF toolkit", description: "Work with PDFs", category: "docs", trustTier: "community", installed: true, enabled: true },
    { slug: "gh", name: "GitHub ops", description: "GitHub automations", category: "dev", trustTier: "verified", installed: true, enabled: true },
  ];
  const integrations: IntegrationView[] = [
    { id: "mcp:web", name: "Web MCP", kind: "mcp", status: "connected", detail: "1 tool" },
    { id: "google", name: "Google Workspace", kind: "google", status: "connected", detail: "you@example.com" },
  ];
  constellationData.seed(
    buildConstellation({ capabilities, models, providers, skills, integrations }),
  );
}

/** Populated 2D catalog fallback with the "showing N of M" cap notice. */
export const Fallback2D: Story = {
  render: () => {
    seed();
    return (
      <ConstellationFallback reason="2D default on this device (no WebGL / probe not passed)" />
    );
  },
};

/** Honest empty state before any capabilities are discovered. */
export const Empty: Story = {
  render: () => {
    constellationData.reset();
    return <ConstellationFallback />;
  },
};

/** The manual 2D/3D representation toggle (Req 7.5 / 17.5). */
export const ModeToggle: Story = {
  render: () => {
    initRenderMode({
      webglTier: "webgl2",
      hasWebGL: true,
      prefersReducedMotion: false,
      supportsBackdropFilter: true,
      probe: null,
    });
    return <LensModeToggle label="Constellation view mode" />;
  },
};
