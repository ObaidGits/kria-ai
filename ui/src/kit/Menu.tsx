/**
 * Menu — dropdown action menu on Kobalte DropdownMenu (design.md §1.5).
 * Correct menu/menuitem roles, arrow-key navigation, typeahead, focus return.
 * Items are declarative; separators and disabled items supported.
 */
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { For, Show, splitProps } from "solid-js";
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
  separator?: boolean;
}

export interface MenuProps {
  /** Accessible name for the trigger button (required). */
  triggerLabel: string;
  /** Icon id → renders an icon-only trigger; otherwise the label is shown. */
  triggerIcon?: string;
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
    "items",
    "label",
    "open",
    "defaultOpen",
    "onOpenChange",
  ]);
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
      <DropdownMenu.Trigger class={triggerClass()} aria-label={local.triggerLabel}>
        <Show when={iconMode()} fallback={local.triggerLabel}>
          <Icon name={local.triggerIcon!} />
        </Show>
      </DropdownMenu.Trigger>
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
                    <Icon name={item.icon!} size={16} />
                  </Show>
                  <DropdownMenu.ItemLabel>{item.label}</DropdownMenu.ItemLabel>
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
