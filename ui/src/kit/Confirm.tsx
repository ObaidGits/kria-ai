/**
 * Confirm — a Dialog specialised for deliberate confirmation (design.md §4.2,
 * Req 11.3). Cancel/keep-paused is always one action; confirm is a deliberate
 * button; high-risk uses the danger variant plus an icon+text risk banner so
 * consequence is never conveyed by color alone (Req 17.3).
 */
import { createSignal, splitProps, Show } from "solid-js";
import { Dialog } from "./Dialog";
import { Button } from "./Button";
import { Icon } from "../components/Icon";
import "./Dialog.css";

export interface ConfirmProps {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  risk?: "none" | "warning" | "danger";
  /** Optional inline trigger button label (omit when controlling `open`). */
  triggerLabel?: string;
  triggerIcon?: string;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  onConfirm?: () => void;
  onCancel?: () => void;
}

export function Confirm(props: ConfirmProps) {
  const [local] = splitProps(props, [
    "title",
    "message",
    "confirmLabel",
    "cancelLabel",
    "risk",
    "triggerLabel",
    "triggerIcon",
    "open",
    "defaultOpen",
    "onOpenChange",
    "onConfirm",
    "onCancel",
  ]);
  const risk = () => local.risk ?? "none";
  const [internalOpen, setInternalOpen] = createSignal(local.defaultOpen ?? false);
  const open = () => local.open ?? internalOpen();
  const setOpen = (value: boolean) => {
    if (local.open === undefined) setInternalOpen(value);
    local.onOpenChange?.(value);
  };

  return (
    <Dialog
      title={local.title}
      description={local.message}
      triggerLabel={local.triggerLabel}
      triggerIcon={local.triggerIcon}
      triggerVariant={risk() === "danger" ? "danger" : "secondary"}
      open={open()}
      onOpenChange={setOpen}
      footer={
        <>
          <Button
            variant="ghost"
            onClick={() => {
              local.onCancel?.();
              setOpen(false);
            }}
          >
            {local.cancelLabel ?? "Cancel"}
          </Button>
          <Button
            variant={risk() === "danger" ? "danger" : "primary"}
            onClick={() => {
              local.onConfirm?.();
              setOpen(false);
            }}
          >
            {local.confirmLabel ?? "Confirm"}
          </Button>
        </>
      }
    >
      <Show when={risk() !== "none"}>
        <div class={`kit-dialog__risk kit-dialog__risk--${risk()}`} role="note">
          <Icon name={risk() === "danger" ? "alert-triangle" : "alert-circle"} size={16} />
          <span>
            {risk() === "danger"
              ? "This action is irreversible."
              : "This action has notable effects."}
          </span>
        </div>
      </Show>
    </Dialog>
  );
}

export default Confirm;
