import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { CoreShell3D } from "./CoreShell3D";
import type { CoreState } from "../stores/coreStore";

// NOTE: the 3D Core needs a real WebGL context. Under jsdom (unit tests) and in
// some headless CI browsers WebGL is unavailable — CoreShell3D then gracefully
// renders the first-class 2D Core instead (design §20.3). In a real GPU browser
// these stories render the single-WebGL translucent shell + filament + motes +
// tilted ring + aura + faked rim (design §4.3).

const ALL_STATES: CoreState[] = [
  "idle",
  "listening",
  "thinking",
  "planning",
  "speaking",
  "responding",
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
  title: "Core/CoreShell3D",
  component: CoreShell3D,
  argTypes: {
    state: { control: "select", options: ALL_STATES },
    size: { control: "select", options: ["sm", "md", "lg"] },
    enabled: { control: "boolean" },
  },
  // Force-enable the 3D path in stories (bypasses the resolver); real WebGL
  // support still decides whether the canvas or the 2D fallback shows.
  args: { state: "idle", size: "lg", enabled: true },
} satisfies Meta<typeof CoreShell3D>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

/** The 3D Core across a few representative states — hue + breath track the same
 * `--presence-*` / `--core-breath-duration` tokens as the 2D Core. */
export const States: Story = {
  render: () => (
    <div style={{ display: "flex", "align-items": "center", gap: "28px" }}>
      <CoreShell3D state="idle" size="lg" enabled />
      <CoreShell3D state="thinking" size="lg" enabled />
      <CoreShell3D state="speaking" size="lg" enabled />
      <CoreShell3D state="blocked" size="lg" enabled />
    </div>
  ),
};

/** With `enabled={false}` the resolver's first-class 2D Core renders — proving
 * the 2D path stays intact and the 3D Core only mounts behind the gate. */
export const DisabledFallsBackTo2D: Story = {
  args: { enabled: false, state: "thinking", size: "lg" },
};
