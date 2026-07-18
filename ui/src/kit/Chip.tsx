/**
 * Chip — compact pill. Static by default; when `onToggle` is provided it is a
 * toggle button exposing aria-pressed; when `onRemove` is provided it renders a
 * labeled remove control (Req 17.2).
 */
import { splitProps, Show, type ParentProps } from "solid-js";
import { Icon } from "../components/Icon";
import "./kit.base.css";
import "./Chip.css";

export interface ChipProps extends ParentProps {
  selected?: boolean;
  disabled?: boolean;
  onToggle?: () => void;
  onRemove?: () => void;
  /** Accessible label for the remove control. */
  removeLabel?: string;
  class?: string;
}

export function Chip(props: ChipProps) {
  const [local] = splitProps(props, [
    "selected",
    "disabled",
    "onToggle",
    "onRemove",
    "removeLabel",
    "class",
    "children",
  ]);

  const body = (
    <>
      {local.children}
      <Show when={local.onRemove}>
        <span
          role="button"
          tabindex="0"
          class="kit-chip__remove kit-focusable"
          aria-label={local.removeLabel ?? "Remove"}
          onClick={(e) => {
            e.stopPropagation();
            local.onRemove?.();
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              local.onRemove?.();
            }
          }}
        >
          <Icon name="x" size={14} />
        </span>
      </Show>
    </>
  );

  return (
    <Show
      when={local.onToggle}
      fallback={
        <span class={`kit-chip ${local.selected ? "kit-chip--selected" : ""} ${local.class ?? ""}`}>
          {body}
        </span>
      }
    >
      <button
        type="button"
        class={`kit-chip kit-focusable ${local.class ?? ""}`}
        aria-pressed={local.selected ? "true" : "false"}
        disabled={local.disabled}
        onClick={() => local.onToggle?.()}
      >
        {body}
      </button>
    </Show>
  );
}

export default Chip;
