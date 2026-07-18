import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { WakeWordTest, WakeWordTestView } from "./WakeWordTest";

/**
 * The REAL wake-word test panel (task 5.3, Req 12.4). These stories render the
 * presentation-only {@link WakeWordTestView} in each state so the visual states
 * (idle / listening / detected / failed / unavailable) can be reviewed in
 * isolation. The live container (`WakeWordTest`) drives these states from real
 * backend readiness + detection events — never a canned success.
 */
const meta = {
  title: "Shell/WakeWordTest",
  component: WakeWordTest,
} satisfies Meta<typeof WakeWordTest>;

export default meta;
type Story = StoryObj<typeof meta>;

const noop = () => {};

export const Idle: Story = {
  render: () => (
    <WakeWordTestView status="idle" detail="" onStart={noop} onCancel={noop} />
  ),
};

export const Listening: Story = {
  render: () => (
    <WakeWordTestView
      status="listening"
      detail="Say the wake word now."
      onStart={noop}
      onCancel={noop}
    />
  ),
};

export const Detected: Story = {
  render: () => (
    <WakeWordTestView
      status="detected"
      detail="Heard the wake word (confidence 92%)."
      onStart={noop}
      onCancel={noop}
    />
  ),
};

export const Failed: Story = {
  render: () => (
    <WakeWordTestView
      status="failed"
      detail="No wake word was detected. Check your mic, then try again."
      onStart={noop}
      onCancel={noop}
    />
  ),
};

export const Unavailable: Story = {
  render: () => (
    <WakeWordTestView
      status="unavailable"
      detail="Wake-word model files are missing (expected near /models/wake/hey_ria.onnx)."
      onStart={noop}
      onCancel={noop}
    />
  ),
};
