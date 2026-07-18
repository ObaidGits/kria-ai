import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onMount, onCleanup } from "solid-js";
import { VoiceSurface } from "./VoiceSurface";
import { voiceStore } from "../../stores";
import type { VoiceUiState } from "../../stores/voiceStore";

/**
 * The compact VoiceSurface (task 5.1, Req 12.1). It reads `voiceStore`; these
 * stories drive the store into a given phase/transcript on mount so the Core +
 * transcript line render the state. Stop is stubbed so nothing calls the bridge.
 */
const meta = {
  title: "Shell/VoiceSurface",
  component: VoiceSurface,
} satisfies Meta<typeof VoiceSurface>;

export default meta;
type Story = StoryObj<typeof meta>;

function Scenario(props: { phase: VoiceUiState; transcript?: string; partial?: boolean }) {
  onMount(() => {
    voiceStore.activate();
    if (props.transcript) voiceStore.setTranscript(props.transcript, props.partial ?? false);
    voiceStore.setState(props.phase);
  });
  onCleanup(() => voiceStore.deactivate());
  return <VoiceSurface onStop={() => voiceStore.deactivate()} />;
}

export const Listening: Story = {
  render: () => <Scenario phase="listening" transcript="turn on the desk lamp" partial />,
};

export const Transcribing: Story = {
  render: () => <Scenario phase="transcribing" transcript="draft an email to the team" partial />,
};

export const Speaking: Story = {
  render: () => <Scenario phase="speaking" transcript="Done — the lamp is on." />,
};
