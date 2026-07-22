/**
 * Menu — dropdown action menu on Kobalte DropdownMenu (design.md §1.5).
 * Correct menu/menuitem roles, arrow-key navigation, typeahead, focus return.
 * Items are declarative; separators and disabled items supported.
 */
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { createUniqueId, For, Show, splitProps } from "solid-js";
import { Icon } from "../components/Icon";
import "./kit.base.css";
import "./floating.css";

export interface MenuItem {
  /** Unique id (used as key). */
  id: string;
  label?: string;
  icon?: string;
  onSelect?: () => void;
  disabled?: boolean;
  /**
   * Optional accessible description for the item (e.g. why it is disabled and
   * what would re-enable it). Rendered via Kobalte `ItemDescription`, which
   * wires the item's `aria-describedby`, so the reason is programmatically
   * associated with the item — not hover-only content.
   */
  description?: string;
  separator?: boolean;
}

export interface MenuProps {
  /** Accessible name for the trigger button (required). */
  triggerLabel: string;
  /** Icon id → renders an icon-only trigger; otherwise the label is shown. */
  triggerIcon?: string;
  /**
   * Optional accessible description for the trigger (e.g. why the menu's
   * actions are currently unavailable and what enables them). Rendered as
   * visible, low-emphasis helper text and wired to the trigger via
   * `aria-describedby` — reachable by AT on focus, never hover-only, and the
   * trigger control itself is never hidden.
   */
  triggerDescription?: string;
  items: MenuItem[];
  /** Optional group label shown at the top of the menu. */
  label?: string;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
}

export function Menu(props: MenuProps) {
  const [local] = splitProps(props, [
    "triggerLabel",
    "triggerIcon",
    "triggerDescription",
    "items",
    "label",
    "open",
    "defaultOpen",
    "onOpenChange",
  ]);
  const descriptionId = createUniqueId();
  const hasTriggerDescription = () => !!local.triggerDescription;
  const iconMode = () => !!local.triggerIcon;
  const triggerClass = () =>
    iconMode()
      ? "kit-icon-button kit-icon-button--ghost kit-icon-button--md kit-focusable kit-transition"
      : "kit-button kit-button--secondary kit-button--md kit-focusable kit-transition";

  return (
    <DropdownMenu
      open={local.open}
      defaultOpen={local.defaultOpen}
      onOpenChange={local.onOpenChange}
    >
      <DropdownMenu.Trigger
        class={triggerClass()}
        aria-label={local.triggerLabel}
        aria-describedby={hasTriggerDescription() ? descriptionId : undefined}
      >
        <Show when={iconMode()} fallback={local.triggerLabel}>
          <Icon name={local.triggerIcon!} />
        </Show>
      </DropdownMenu.Trigger>
      <Show when={hasTriggerDescription()}>
        <span id={descriptionId} class="kit-menu-trigger-description" role="note">
          {local.triggerDescription}
        </span>
      </Show>
      <DropdownMenu.Portal>
        <DropdownMenu.Content class="kit-floating kit-floating--menu">
          <Show when={local.label}>
            <DropdownMenu.GroupLabel class="kit-menu-label">
              {local.label}
            </DropdownMenu.GroupLabel>
          </Show>
          <For each={local.items}>
            {(item) => (
              <Show
                when={!item.separator}
                fallback={<DropdownMenu.Separator class="kit-menu-separator" />}
              >
                <DropdownMenu.Item
                  class="kit-menu-item"
                  disabled={item.disabled}
                  onSelect={() => item.onSelect?.()}
                >
                  <Show when={item.icon}>
                    <Icon name={item.icon!} size="body" />
                  </Show>
                  <DropdownMenu.ItemLabel>{item.label}</DropdownMenu.ItemLabel>
                  <Show when={item.description}>
                    <DropdownMenu.ItemDescription class="kit-menu-item-description">
                      {item.description}
                    </DropdownMenu.ItemDescription>
                  </Show>
                </DropdownMenu.Item>
              </Show>
            )}
          </For>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu>
  );
}

export default Menu;
