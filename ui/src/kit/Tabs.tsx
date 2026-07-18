/**
 * Tabs — on Kobalte Tabs (design.md §1.5). Correct tablist/tab/tabpanel roles,
 * arrow-key navigation, and focus management for free.
 */
import { Tabs as KTabs } from "@kobalte/core/tabs";
import { createEffect, createMemo, createSignal, For, Show, splitProps, type JSX } from "solid-js";
import "./kit.base.css";
import "./Tabs.css";

export interface TabItem {
  value: string;
  label: string;
  /** Lazy panel renderer. Defers owner-sensitive computations until selected. */
  content: () => JSX.Element;
  disabled?: boolean;
}

export interface TabsProps {
  items: TabItem[];
  value?: string;
  defaultValue?: string;
  onChange?: (value: string) => void;
  class?: string;
}

export function Tabs(props: TabsProps) {
  const [local] = splitProps(props, ["items", "value", "defaultValue", "onChange", "class"]);
  const [uncontrolledValue, setUncontrolledValue] = createSignal(
    local.defaultValue ?? local.items[0]?.value,
  );
  const selectedValue = createMemo(() => local.value ?? uncontrolledValue());
  const selectedItem = createMemo(() =>
    local.items.find((item) => item.value === selectedValue()),
  );

  createEffect(() => {
    if (local.value === undefined && !selectedItem() && local.items[0]) {
      setUncontrolledValue(local.items[0].value);
    }
  });

  const handleChange = (value: string) => {
    if (local.value === undefined) setUncontrolledValue(value);
    local.onChange?.(value);
  };

  return (
    <KTabs
      class={`kit-tabs ${local.class ?? ""}`}
      value={selectedValue()}
      onChange={handleChange}
    >
      <KTabs.List class="kit-tabs__list">
        <For each={local.items}>
          {(item) => (
            <KTabs.Trigger
              class="kit-tabs__trigger kit-focusable"
              value={item.value}
              disabled={item.disabled}
            >
              {item.label}
            </KTabs.Trigger>
          )}
        </For>
        <KTabs.Indicator class="kit-tabs__indicator" />
      </KTabs.List>
      <Show when={selectedItem()} keyed>
        {(item) => (
          <KTabs.Content class="kit-tabs__content" value={item.value}>
            {item.content()}
          </KTabs.Content>
        )}
      </Show>
    </KTabs>
  );
}

export default Tabs;
