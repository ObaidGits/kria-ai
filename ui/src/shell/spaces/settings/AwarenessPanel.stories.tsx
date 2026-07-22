import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { AwarenessPanel } from "./AwarenessPanel";
import {
  createDefaultDesktopAwarenessRegistry,
  type DesktopAwarenessRegistry,
} from "../../../stores/desktopAwarenessBridge";

/**
 * "What KRIA can sense" Settings panel (task 3.8, Req 25.5). Each story builds an
 * ISOLATED registry (injected bridge no-ops so the workbench never touches the
 * real Focus engine) and drives the panel through its `registry` prop. Sources
 * are off by default; the "Some sources on" story opts a couple in and marks one
 * as remembered to show the ephemeral-vs-memory toggle.
 */
const meta = {
  title: "Spaces/Settings/AwarenessPanel",
  component: AwarenessPanel,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-settings" style={{ padding: "24px", "max-width": "820px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof AwarenessPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Build an isolated registry on a Wayland session, decoupled from the engine. */
function isolatedRegistry(): DesktopAwarenessRegistry {
  return createDefaultDesktopAwarenessRegistry({
    platform: "wayland",
    setBridge: () => {},
    clearBridge: () => {},
  });
}

export const AllOff: Story = {
  render: () => <AwarenessPanel registry={isolatedRegistry()} />,
};

export const SomeSourcesOn: Story = {
  render: () => {
    const registry = isolatedRegistry();
    registry.optIn("battery");
    registry.optIn("media");
    registry.optIn("calendar");
    // Remember one low-tier source to showcase the opt-into-memory toggle.
    registry.optInToMemory("media");
    return <AwarenessPanel registry={registry} />;
  },
};
