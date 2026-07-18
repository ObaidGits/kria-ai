import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";
import { Dialog } from "./Dialog";
import { Confirm } from "./Confirm";
import { Button } from "./Button";

const meta = {
  title: "Kit/Dialog",
  component: Dialog,
} satisfies Meta<typeof Dialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Basic: Story = {
  args: { title: "Rename thread" },
  render: () => (
    <Dialog
      triggerLabel="Open dialog"
      title="Rename thread"
      description="Give this conversation a memorable name."
      footer={<Button>Save</Button>}
    >
      <input
        placeholder="New name"
        style={{ width: "100%", padding: "8px", "border-radius": "8px" }}
      />
    </Dialog>
  ),
};

export const ConfirmDanger: Story = {
  args: { title: "Delete this memory?" },
  render: () => {
    const [open, setOpen] = createSignal(false);
    return (
      <>
        <Button variant="danger" onClick={() => setOpen(true)}>
          Delete memory
        </Button>
        <Confirm
          open={open()}
          onOpenChange={setOpen}
          title="Delete this memory?"
          message="KRIA will permanently forget this fact."
          risk="danger"
          confirmLabel="Delete"
          onConfirm={() => {}}
        />
      </>
    );
  },
};

export const ConfirmWithTrigger: Story = {
  args: { title: "Reset all settings?" },
  render: () => (
    <Confirm
      triggerLabel="Reset settings"
      title="Reset all settings?"
      message="This restores defaults. You can reconfigure afterwards."
      risk="warning"
      confirmLabel="Reset"
    />
  ),
};
