import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { EmptyState } from "./EmptyState";
import { Button } from "./Button";

const meta = {
  title: "Kit/EmptyState",
  component: EmptyState,
  args: {
    icon: "message-circle",
    title: "Start a conversation",
    description: "Ask KRIA anything, or pick one of the example intents below.",
  },
} satisfies Meta<typeof EmptyState>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const WithAction: Story = {
  args: {
    icon: "brain",
    title: "No memories yet",
    description: "KRIA will remember useful facts as you work together.",
    action: <Button>Add a memory</Button>,
  },
};

export const TitleOnly: Story = {
  args: { icon: undefined, title: "Nothing here", description: undefined },
};
