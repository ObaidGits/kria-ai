import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { G1VirtualRows } from "./G1VirtualRows";
import { G4LiveCharts } from "./G4LiveCharts";
import { G2Probe, G5PaletteProbe, G8BlurProbe } from "./G2G5G8Probes";

/**
 * Prototype validation gate harness (design.md §11.3). Each story mounts a gate
 * probe for on-device measurement. Run these in Storybook on each device of the
 * GNOME+KDE × Wayland+X11 × NVIDIA+AMD+Intel matrix and record results in
 * .kiro/specs/kria-ui-redesign/PROTOTYPE_GATES.md.
 */
const meta = {
  title: "Prototypes/Gates",
} satisfies Meta;

export default meta;
type Story = StoryObj;

export const G1_WebKitGTKBaseline: Story = {
  name: "G1 · WebKitGTK baseline (5k virtualized rows)",
  render: () => <G1VirtualRows count={5000} />,
};

export const G2_ThreeDViability: Story = {
  name: "G2 · 3D graph viability",
  render: () => <G2Probe />,
};

export const G4_LiveCharts: Story = {
  name: "G4 · uPlot live charts (5 series @1Hz)",
  render: () => <G4LiveCharts series={5} intervalMs={1000} />,
};

export const G5_PaletteFuzzy: Story = {
  name: "G5 · Command-palette fuzzy",
  render: () => <G5PaletteProbe />,
};

export const G8_BlurAuraGlass: Story = {
  name: "G8 · Blur / aura-glass",
  render: () => <G8BlurProbe />,
};
