/**
 * Tooltip — on Kobalte Tooltip. Shows on hover AND keyboard focus (Req 17.1),
 * dismisses on escape/blur. Content is supplementary only (never the sole
 * carrier of essential info).
 */
import { Tooltip as KTooltip } from "@kobalte/core/tooltip";
import { splitProps, type JSX } from "solid-js";
import "./kit.base.css";
import "./floating.css";

export interface TooltipProps {
  /** Trigger element (must be focusable for keyboard access). */
  children: JSX.Element;
  /** Tooltip text/content. */
  content: JSX.Element;
  open?: boolean;
  defaultOpen?: boolean;
  openDelay?: number;
  onOpenChange?: (open: boolean) => void;
  placement?: "top" | "bottom" | "left" | "right";
}

export function Tooltip(props: TooltipProps) {
  const [local] = splitProps(props, [
    "children",
    "content",
    "open",
    "defaultOpen",
    "openDelay",
    "onOpenChange",
    "placement",
  ]);

  return (
    <KTooltip
      open={local.open}
      defaultOpen={local.defaultOpen}
      openDelay={local.openDelay ?? 300}
      onOpenChange={local.onOpenChange}
      placement={local.placement ?? "top"}
    >
      <KTooltip.Trigger as="span" class="kit-tooltip-trigger">
        {local.children}
      </KTooltip.Trigger>
      <KTooltip.Portal>
        <KTooltip.Content class="kit-floating kit-tooltip">
          {local.content}
        </KTooltip.Content>
      </KTooltip.Portal>
    </KTooltip>
  );
}

export default Tooltip;
