/**
 * Row — list item with leading/trailing slots. When `onSelect` is provided it
 * renders a semantic button (keyboard-operable, focus-visible, aria-selected).
 */
import { splitProps, Show, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import "./kit.base.css";
import "./Row.css";

export interface RowProps {
  title?: JSX.Element;
  subtitle?: JSX.Element;
  leading?: JSX.Element;
  trailing?: JSX.Element;
  selected?: boolean;
  disabled?: boolean;
  onSelect?: () => void;
  children?: JSX.Element;
  class?: string;
}

export function Row(props: RowProps) {
  const [local] = splitProps(props, [
    "title",
    "subtitle",
    "leading",
    "trailing",
    "selected",
    "disabled",
    "onSelect",
    "children",
    "class",
  ]);
  const interactive = () => !!local.onSelect;

  return (
    <Dynamic
      component={interactive() ? "button" : "div"}
      type={interactive() ? "button" : undefined}
      class={`kit-row ${interactive() ? "kit-focusable" : ""} ${local.selected ? "kit-row--selected" : ""} ${local.class ?? ""}`}
      aria-selected={interactive() ? (local.selected ? "true" : "false") : undefined}
      disabled={interactive() ? local.disabled : undefined}
      onClick={() => local.onSelect?.()}
    >
      <Show when={local.leading}>
        <span class="kit-row__leading">{local.leading}</span>
      </Show>
      <span class="kit-row__content">
        <Show when={local.title}>
          <span class="kit-row__title">{local.title}</span>
        </Show>
        <Show when={local.subtitle}>
          <span class="kit-row__subtitle">{local.subtitle}</span>
        </Show>
        {local.children}
      </span>
      <Show when={local.trailing}>
        <span class="kit-row__trailing">{local.trailing}</span>
      </Show>
    </Dynamic>
  );
}

export default Row;
