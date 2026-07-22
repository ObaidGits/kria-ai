/**
 * Popover — on Kobalte Popover (design.md §1.5). Focus is moved into the panel
 * on open and restored on close; Escape + outside-click dismiss; correct
 * dialog semantics. Floating layer (aura-glass, blur) via floating.css.
 */
import { Popover as KPopover } from "@kobalte/core/popover";
import { splitProps, Show, type JSX } from "solid-js";
import { Icon } from "../components/Icon";
import "./kit.base.css";
import "./floating.css";

export interface PopoverProps {
  /** Accessible name for the trigger button (required). */
  triggerLabel: string;
  /** Icon id → icon-only trigger; otherwise the label is shown. */
  triggerIcon?: string;
  title?: string;
  children: JSX.Element;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  placement?: "top" | "bottom" | "left" | "right";
}

export function Popover(props: PopoverProps) {
  const [local] = splitProps(props, [
    "triggerLabel",
    "triggerIcon",
    "title",
    "children",
    "open",
    "defaultOpen",
    "onOpenChange",
    "placement",
  ]);
  const iconMode = () => !!local.triggerIcon;
  const triggerClass = () =>
    iconMode()
      ? "kit-icon-button kit-icon-button--ghost kit-icon-button--md kit-focusable kit-transition"
      : "kit-button kit-button--secondary kit-button--md kit-focusable kit-transition";

  return (
    <KPopover
      open={local.open}
      defaultOpen={local.defaultOpen}
      onOpenChange={local.onOpenChange}
      placement={local.placement ?? "bottom"}
    >
      <KPopover.Trigger class={triggerClass()} aria-label={local.triggerLabel}>
        <Show when={iconMode()} fallback={local.triggerLabel}>
          <Icon name={local.triggerIcon!} />
        </Show>
      </KPopover.Trigger>
      <KPopover.Portal>
        <KPopover.Content class="kit-floating kit-popover">
          <KPopover.CloseButton class="kit-popover__close kit-focusable" aria-label="Close">
            <Icon name="x" size="body" />
          </KPopover.CloseButton>
          <Show when={local.title}>
            <KPopover.Title class="kit-popover__title">{local.title}</KPopover.Title>
          </Show>
          <KPopover.Description as="div">{local.children}</KPopover.Description>
        </KPopover.Content>
      </KPopover.Portal>
    </KPopover>
  );
}

export default Popover;
