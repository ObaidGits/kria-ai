/**
 * OverflowControl — the one labelled, badged responsive overflow surface
 * (design.md §20.3 "Responsive overflow/disclosure" row; UIE-H-007).
 *
 * Built on the shared kit `Menu` (Kobalte DropdownMenu) + kit `Badge` — no
 * parallel dropdown. It inherits Menu's correct menu roles, arrow-key
 * navigation, typeahead, Escape/outside close, and focus return to its trigger.
 *
 * §20.3 contract satisfied here:
 *  - Labelled trigger: `triggerLabel` (accessible name) always names the
 *    control AND folds in waiting/error counts + state, so urgency is never
 *    hidden behind the disclosure (UIE-H-007 badge rule).
 *  - A visible {@link Badge} mirrors the count/state for sighted users. It is
 *    marked decorative (`aria-hidden`) so the count is announced exactly once,
 *    via the trigger's accessible name.
 *  - Keyboard reachable + Escape/outside close: inherited from Menu.
 *  - Closing invokes no action: only `MenuItem.onSelect` runs an action; a
 *    dismiss (Escape/outside) flips open state only (inherited from Menu).
 *  - Must not cover/outrank a pending approval: this is a non-blocking
 *    `floating`-priority surface. When a pending approval is up the shell's
 *    inertness controller ({@link ./overlayLayers}) inerts lower surfaces;
 *    Kobalte manages its own portal + focus return, so we do not fight it.
 *
 * Requirements: 11.1, 11.9, 16.3–16.5
 */
import { Show, splitProps } from "solid-js";
import { Badge, type BadgeTone } from "../kit/Badge";
import { Menu, type MenuItem } from "../kit/Menu";
import "./OverflowControl.css";

export interface OverflowControlProps {
  /** Overflowed actions, rendered as menu items (reuses Menu's items API). */
  items: MenuItem[];
  /** Base accessible label for the trigger. Default: "More actions". */
  label?: string;
  /** Number of items waiting on a decision (e.g. pending approvals). */
  waitingCount?: number;
  /** Number of items in an error state. */
  errorCount?: number;
  /** Optional extra state phrase folded into the accessible name. */
  state?: string;
  /** Icon id for the trigger. Default: "ellipsis". */
  triggerIcon?: string;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
}

export function OverflowControl(props: OverflowControlProps) {
  const [local] = splitProps(props, [
    "items",
    "label",
    "waitingCount",
    "errorCount",
    "state",
    "triggerIcon",
    "open",
    "defaultOpen",
    "onOpenChange",
  ]);

  const baseLabel = () => local.label ?? "More actions";
  const waiting = () => local.waitingCount ?? 0;
  const errors = () => local.errorCount ?? 0;
  const total = () => waiting() + errors();
  const hasError = () => errors() > 0;
  const showBadge = () => total() > 0 || !!local.state;

  /** Trigger accessible name — folds in urgency so overflow never hides it. */
  const accessibleName = () => {
    const parts = [baseLabel()];
    if (waiting() > 0) parts.push(`${waiting()} waiting`);
    if (errors() > 0) parts.push(`${errors()} ${errors() === 1 ? "error" : "errors"}`);
    if (local.state) parts.push(local.state);
    return parts.join(", ");
  };

  const badgeTone = (): BadgeTone => (hasError() ? "danger" : "warning");
  const badgeText = () => (total() > 0 ? String(total()) : "!");

  return (
    <span class="kria-overflow-control">
      <Menu
        triggerLabel={accessibleName()}
        triggerIcon={local.triggerIcon ?? "ellipsis"}
        items={local.items}
        open={local.open}
        defaultOpen={local.defaultOpen}
        onOpenChange={local.onOpenChange}
      />
      <Show when={showBadge()}>
        {/* Decorative mirror of the count/state; announced once via triggerLabel. */}
        <span class="kria-overflow-control__badge" aria-hidden="true">
          <Badge tone={badgeTone()}>{badgeText()}</Badge>
        </span>
      </Show>
    </span>
  );
}

export default OverflowControl;
