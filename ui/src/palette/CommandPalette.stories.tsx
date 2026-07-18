import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onMount, onCleanup } from "solid-js";
import { CommandPalette } from "./CommandPalette";
import { shellStore, converseStore, memoryStore, automationStore, capabilityStore, machineStore, settingsStore } from "../stores";
import { registerCommands } from "./commands";
import { registerShortcuts, DEFAULT_SHORTCUTS } from "./shortcuts";
import { Button } from "../kit";

/**
 * The Command Palette is a global overlay controlled by `shellStore.paletteOpen`.
 * These stories seed the stores with sample entities so every source (Spaces,
 * commands, settings, memories, workflows, capabilities, models, threads,
 * devices, shortcuts) has something to show, then open it.
 */
const meta = {
  title: "Palette/CommandPalette",
  component: CommandPalette,
} satisfies Meta<typeof CommandPalette>;

export default meta;
type Story = StoryObj<typeof meta>;

function seed() {
  converseStore.setThreads([
    { id: "t1", title: "Trip planning", createdAt: 0, updatedAt: 0, pinned: false, archived: false, temporary: false },
    { id: "t2", title: "Rust refactor", createdAt: 0, updatedAt: 0, pinned: false, archived: false, temporary: false },
  ]);
  memoryStore.setFacts([
    { id: "m1", content: "User prefers dark mode", confidence: 0.9, worth: 0.8, staleness: 0.1, source: "chat", createdAt: 0, updatedAt: 0, tags: ["preference"] },
  ]);
  automationStore.setWorkflows([
    { id: "w1", name: "Morning briefing", description: "Summarize overnight news", status: "idle", lastRunAt: null, createdAt: 0 },
  ]);
  capabilityStore.setCapabilities([
    { id: "c1", name: "Web search", type: "tool", status: "active", description: "Search the web", source: "native", riskLevel: "green" },
  ]);
  capabilityStore.setProviders([{ id: "p1", name: "Local llama", type: "local", active: true }]);
  machineStore.setDevices([
    { id: "d1", name: "workstation", type: "desktop", status: "online", os: "Ubuntu", lastSeen: 0 },
  ]);
  settingsStore.setSchema([
    { key: "voice.speed", section: "voice", field: "speed", label: "Voice speed", group: "voice", type: "number", risk: "none", requiresRestart: false, envLocked: false, secret: false, description: "TTS playback rate" },
  ]);
}

export const Interactive: Story = {
  render: () => {
    onMount(() => {
      seed();
      const undoCmd = registerCommands([
        { id: "cmd.theme", title: "Toggle theme", icon: "eye", run: () => shellStore.toggleTheme() },
      ]);
      const undoSc = registerShortcuts(DEFAULT_SHORTCUTS);
      onCleanup(() => {
        undoCmd();
        undoSc();
        shellStore.setPaletteOpen(false);
      });
    });
    return (
      <div style={{ padding: "24px" }}>
        <Button onClick={() => shellStore.setPaletteOpen(true)}>Open Command Palette</Button>
        <CommandPalette />
      </div>
    );
  },
};
