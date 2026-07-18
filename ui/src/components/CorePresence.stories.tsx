import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { CorePresence } from "./CorePresence";
import type { CoreState } from "../stores/coreStore";

const ALL_STATES: CoreState[] = [
  "idle",
  "listening",
  "thinking",
  "planning",
  "speaking",
  "acting",
  "running-automation",
  "watching",
  "remembering",
  "reflecting",
  "learning",
  "waiting",
  "blocked",
  "error",
  "recovering",
];

const meta = {
  title: "Core/CorePresence",
  component: CorePresence,
  argTypes: {
    state: { control: "select", options: ALL_STATES },
    size: { control: "select", options: ["sm", "md", "lg"] },
    reducedMotion: { control: "boolean" },
  },
  args: { state: "idle", size: "lg" },
} satisfies Meta<typeof CorePresence>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Sizes: Story = {
  render: () => (
    <div style={{ display: "flex", "align-items": "center", gap: "24px" }}>
      <CorePresence state="thinking" size="sm" />
      <CorePresence state="thinking" size="md" />
      <CorePresence state="thinking" size="lg" />
    </div>
  ),
};

/** Every Core state, side by side, so the breath/density/temperature/light
 * treatments are directly comparable. */
export const AllStates: Story = {
  render: () => (
    <div
      style={{
        display: "grid",
        "grid-template-columns": "repeat(auto-fill, minmax(120px, 1fr))",
        gap: "20px",
      }}
    >
      {ALL_STATES.map((s) => (
        <div style={{ display: "flex", "flex-direction": "column", "align-items": "center", gap: "10px" }}>
          <CorePresence state={s} size="lg" />
          <span style={{ "font-size": "12px", opacity: 0.8 }}>{s}</span>
        </div>
      ))}
    </div>
  ),
};

/** Static representation used under reduced-motion (no ambient animation). */
export const ReducedMotionStatic: Story = {
  render: () => (
    <div style={{ display: "flex", "align-items": "center", gap: "24px" }}>
      <CorePresence state="thinking" size="lg" reducedMotion />
      <CorePresence state="acting" size="lg" reducedMotion />
      <CorePresence state="blocked" size="lg" reducedMotion />
    </div>
  ),
};
