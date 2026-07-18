import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onMount } from "solid-js";
import { VoiceModeSwitcher } from "./VoiceModeSwitcher";
import { voiceStore } from "../../stores";
import type { VoiceMode } from "../../stores";

/**
 * The in-surface voice mode + engine switcher (task 5.2, Req 12.2/12.3). It
 * reads/writes `voiceStore`; these stories seed a starting mode/engine on mount.
 * Selecting a mode/engine routes through the existing `patch_config` voice
 * command (a no-op here, outside Tauri) and updates the store.
 */
const meta = {
  title: "Shell/VoiceModeSwitcher",
  component: VoiceModeSwitcher,
} satisfies Meta<typeof VoiceModeSwitcher>;

export default meta;
type Story = StoryObj<typeof meta>;

function Scenario(props: { mode: VoiceMode; stt?: string; tts?: string }) {
  onMount(() => {
    voiceStore.setMode(props.mode);
    voiceStore.setHealth({
      sttHealthy: true,
      ttsHealthy: true,
      sttEngine: props.stt ?? "faster-whisper",
      ttsEngine: props.tts ?? "piper-rs",
    });
  });
  return <VoiceModeSwitcher />;
}

export const Conversation: Story = {
  render: () => <Scenario mode="conversation" />,
};

export const WakeWord: Story = {
  render: () => <Scenario mode="wake-word" stt="whisper-rs" tts="kokoro" />,
};

export const Coding: Story = {
  render: () => <Scenario mode="coding" />,
};
