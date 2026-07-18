import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onMount, onCleanup } from "solid-js";
import { NotificationCenter } from "./NotificationCenter";
import { notificationStore, shellStore, type NotificationInput } from "../../stores";

/**
 * NotificationCenter — the batched, tiered, NON-blocking notice panel (Req 13).
 * Stories seed `notificationStore` and open the panel so the tiers, batching
 * (×N count), and the non-blocking "needs-you" tier are all visible.
 */
function Seed(props: { items: NotificationInput[] }) {
  onMount(() => {
    notificationStore.clear();
    props.items.forEach((n) => notificationStore.push(n));
    shellStore.setNotificationsOpen(true);
  });
  onCleanup(() => {
    shellStore.setNotificationsOpen(false);
    notificationStore.clear();
  });
  return <NotificationCenter />;
}

const meta = {
  title: "Shell/NotificationCenter",
  component: NotificationCenter,
} satisfies Meta<typeof NotificationCenter>;

export default meta;
type Story = StoryObj<typeof meta>;

/** All tiers together, including a batched row and the non-blocking needs-you tier. */
export const AllTiers: Story = {
  render: () => (
    <Seed
      items={[
        { id: "s1", level: "success", message: "Backup finished", source: "Automations" },
        { id: "i1", level: "info", message: "Indexed **12** documents into memory", groupKey: "idx" },
        { id: "i2", level: "info", message: "Indexed **13** documents into memory", groupKey: "idx" },
        { id: "w1", level: "warn", message: "Provider latency is elevated", detail: "openai · 4.2s p95" },
        { id: "e1", level: "error", message: "Sync to device *studio* failed" },
        {
          id: "y1",
          level: "needs-you",
          message: "Choose a file to continue the workflow",
          action: { label: "Open Automations", route: "automations" },
        },
      ]}
    />
  ),
};

/** Empty — the calm resting state. */
export const Empty: Story = {
  render: () => <Seed items={[]} />,
};
