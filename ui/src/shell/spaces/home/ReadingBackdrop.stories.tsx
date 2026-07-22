import type { Meta, StoryObj } from "storybook-solidjs-vite";
import ReadingBackdrop from "./ReadingBackdrop";

/**
 * ReadingBackdrop — the depth-recession layer for Reading Mode (design.md §11,
 * task 8.4, Requirement 11.1 / 11.2 / 11.3).
 *
 * This exercises the receded-Room + ambient-Core + hard-dim layer that sits
 * BEHIND the conversation while Reading Mode is active. In the app it renders as
 * a `z-index:-1` child of the conversation surface (see `ConverseSpace`), with
 * the reused message stream + its near-solid reading backing painting above it.
 * Here it is wrapped in a `position: relative` shell with a light foreground
 * "reading column" so the recession + hard-dim read correctly in the workbench.
 *
 * The recession, dim, and reading backing are all token-driven (zero raw color)
 * and the settle motion is reduced-motion safe (instant).
 *
 * Requirements: 11.1, 11.2, 11.3 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/ReadingBackdrop",
  component: ReadingBackdrop,
  decorators: [
    (Story: () => unknown) => (
      <div
        class="kria-shell"
        data-window-mode="immersive"
        style={{ height: "600px", position: "relative", overflow: "hidden" }}
      >
        {Story() as never}
        {/* Foreground stand-in for the reused message stream + near-solid
            reading backing (owned by ConverseSpace in the app). */}
        <div
          style={{
            position: "relative",
            "z-index": "1",
            width: "min(720px, 92%)",
            "margin-inline": "auto",
            "margin-top": "80px",
            padding: "var(--space-6)",
            background: "var(--reading-backing)",
            color: "var(--color-text-primary)",
            "border-radius": "var(--radius-md)",
          }}
        >
          The conversation reads on a near-solid backing while the Room recedes
          hard behind it. Legibility is never sacrificed for atmosphere.
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof ReadingBackdrop>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Reading Mode backdrop — Room receded, Core dimmed to an ambient glow. */
export const Default: Story = {
  render: () => <ReadingBackdrop />,
};
