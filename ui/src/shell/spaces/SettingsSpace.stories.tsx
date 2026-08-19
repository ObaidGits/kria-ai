import type { Meta, StoryObj } from "storybook-solidjs-vite";
import SettingsSpace from "./SettingsSpace";
import { settingsStore, type SettingMeta } from "../../stores/settingsStore";

/**
 * Settings — the "AI & Models" group.
 *
 * Exists to make three changes visible without launching the desktop app:
 *
 *   1. The LLM provider editor renders HERE now. It was built in the capabilities
 *      space, so Settings previously offered a single legacy routing dropdown while
 *      the real editor (seven provider types, API key, endpoint, model) sat one space
 *      away and was never found.
 *   2. The legacy "AI routing" row is gone. It was derived from the active provider,
 *      so changing it appeared to work and silently reverted on the next config load.
 *   3. Long descriptions clamp to one line behind a "More" toggle. 24 of the 69
 *      described settings ran past 60 characters, which pushed the controls apart.
 *
 * The store is seeded directly — no backend — so the layout can be inspected in
 * isolation. The provider panel will show its own empty state, since nothing here
 * answers `list_providers`.
 */
const meta = {
  title: "Spaces/Settings/AiAndModels",
  component: SettingsSpace,
} satisfies Meta<typeof SettingsSpace>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A short description stays inline; a long one is folded behind "More". */
const rows: SettingMeta[] = [
  {
    key: "llm.active_model", section: "llm", field: "active_model",
    label: "Default local model", group: "intelligence", subsection: "Model runtime",
    type: "string", risk: "low", requiresRestart: false, envLocked: false, secret: false,
    description: "Used for local inference when the active provider does not specify a model.",
  } as SettingMeta,
  {
    key: "llm.temperature", section: "llm", field: "temperature",
    label: "Response creativity", group: "intelligence", subsection: "Generation behavior",
    type: "number", risk: "low", requiresRestart: false, envLocked: false, secret: false,
    description: "Lower values are more predictable.",
  } as SettingMeta,
  {
    key: "hardware.gpu_layers", section: "hardware", field: "gpu_layers",
    label: "GPU layer override", group: "intelligence", subsection: "Model performance",
    type: "number", risk: "medium", requiresRestart: true, envLocked: false, secret: false,
    description: "Leave at Automatic to let KRIA choose based on available GPU memory.",
  } as SettingMeta,
];

export const AiAndModels: Story = {
  render: () => {
    settingsStore.setSchema(rows);
    settingsStore.setSettings({
      llm: { active_model: "qwen2.5-vl-7b", temperature: 0.7 },
      hardware: { gpu_layers: -1 },
    });
    settingsStore.setActiveGroup("intelligence");
    settingsStore.setSearchQuery("");
    return <SettingsSpace />;
  },
};
