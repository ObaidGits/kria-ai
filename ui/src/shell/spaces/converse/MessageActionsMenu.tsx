/**
 * Message action menus (Req 4.8) — two entry points sharing one action set:
 *
 *  • `MessageContextMenu` wraps the bubble so a RIGHT-CLICK opens the actions
 *    (Kobalte ContextMenu — also opens via the keyboard Menu key / Shift+F10).
 *  • `MessageActionsMenu` is one explicit, persistent, always-keyboard-reachable
 *    trigger button (Kobalte DropdownMenu) that is visible at rest and merely
 *    promoted in emphasis on selection/focus/hover (Req 12.2). It is a single
 *    tab stop per message and opens the menu holding all six actions.
 *
 * Both are Kobalte-backed → correct menu/menuitem roles, arrow-key nav,
 * typeahead, focus return, and a focus-visible trigger (Req 17.1). Feedback is
 * a submenu so the top level stays at the six named actions.
 */
import { ContextMenu } from "@kobalte/core/context-menu";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { For, Show, type JSX } from "solid-js";
import { Icon } from "../../../components/Icon";
import "../../../kit/floating.css";
import type { MessageAction } from "./messageActions";

// ─── Right-click context menu ────────────────────────────────────────────────

function ContextItems(props: { actions: MessageAction[] }) {
  return (
    <For each={props.actions}>
      {(action) => (
        <Show
          when={action.children}
          fallback={
            <ContextMenu.Item class="kit-menu-item" onSelect={() => action.run?.()}>
              <Icon name={action.icon} size={16} />
              <ContextMenu.ItemLabel>{action.label}</ContextMenu.ItemLabel>
            </ContextMenu.Item>
          }
        >
          <ContextMenu.Sub>
            <ContextMenu.SubTrigger class="kit-menu-item">
              <Icon name={action.icon} size={16} />
              <span class="kit-menu-item__label">{action.label}</span>
              <Icon name="chevron-right" size={16} />
            </ContextMenu.SubTrigger>
            <ContextMenu.Portal>
              <ContextMenu.SubContent class="kit-floating kit-floating--menu">
                <For each={action.children}>
                  {(child) => (
                    <ContextMenu.Item class="kit-menu-item" onSelect={() => child.run?.()}>
                      <Icon name={child.icon} size={16} />
                      <ContextMenu.ItemLabel>{child.label}</ContextMenu.ItemLabel>
                    </ContextMenu.Item>
                  )}
                </For>
              </ContextMenu.SubContent>
            </ContextMenu.Portal>
          </ContextMenu.Sub>
        </Show>
      )}
    </For>
  );
}

export interface MessageContextMenuProps {
  actions: MessageAction[];
  children: JSX.Element;
}

export function MessageContextMenu(props: MessageContextMenuProps) {
  return (
    <ContextMenu>
      {/* Wrapper trigger — the inner <article> carries the accessible name, so
          the wrapper stays unlabeled to avoid a duplicate name. */}
      <ContextMenu.Trigger as="div" class="kria-msg__context-trigger">
        {props.children}
      </ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content class="kit-floating kit-floating--menu" aria-label="Message actions">
          <ContextItems actions={props.actions} />
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu>
  );
}

// ─── Keyboard-reachable actions button ───────────────────────────────────────

function DropdownItems(props: { actions: MessageAction[] }) {
  return (
    <For each={props.actions}>
      {(action) => (
        <Show
          when={action.children}
          fallback={
            <DropdownMenu.Item class="kit-menu-item" onSelect={() => action.run?.()}>
              <Icon name={action.icon} size={16} />
              <DropdownMenu.ItemLabel>{action.label}</DropdownMenu.ItemLabel>
            </DropdownMenu.Item>
          }
        >
          <DropdownMenu.Sub>
            <DropdownMenu.SubTrigger class="kit-menu-item">
              <Icon name={action.icon} size={16} />
              <span class="kit-menu-item__label">{action.label}</span>
              <Icon name="chevron-right" size={16} />
            </DropdownMenu.SubTrigger>
            <DropdownMenu.Portal>
              <DropdownMenu.SubContent class="kit-floating kit-floating--menu">
                <For each={action.children}>
                  {(child) => (
                    <DropdownMenu.Item class="kit-menu-item" onSelect={() => child.run?.()}>
                      <Icon name={child.icon} size={16} />
                      <DropdownMenu.ItemLabel>{child.label}</DropdownMenu.ItemLabel>
                    </DropdownMenu.Item>
                  )}
                </For>
              </DropdownMenu.SubContent>
            </DropdownMenu.Portal>
          </DropdownMenu.Sub>
        </Show>
      )}
    </For>
  );
}

export interface MessageActionsMenuProps {
  actions: MessageAction[];
}

export function MessageActionsMenu(props: MessageActionsMenuProps) {
  return (
    <DropdownMenu>
      <DropdownMenu.Trigger
        class="kit-icon-button kit-icon-button--ghost kit-icon-button--sm kit-focusable kit-transition"
        aria-label="Message actions"
      >
        <Icon name="ellipsis" />
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content class="kit-floating kit-floating--menu" aria-label="Message actions">
          <DropdownItems actions={props.actions} />
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu>
  );
}
