/**
 * SegmentBar — segmented single-select on Kobalte ToggleGroup (roving focus,
 * arrow-key navigation for free). Requires an accessible group label (Req 17.2).
 */
import { ToggleGroup } from "@kobalte/core/toggle-group";
import { For, splitProps } from "solid-js";
import "./kit.base.css";
import "./SegmentBar.css";

export interface SegmentOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SegmentBarProps {
  /** Accessible name for the group (aria-label). */
  label: string;
  options: SegmentOption[];
  value?: string;
  defaultValue?: string;
  disabled?: boolean;
  onChange?: (value: string) => void;
  class?: string;
}

export function SegmentBar(props: SegmentBarProps) {
  const [local] = splitProps(props, [
    "label",
    "options",
    "value",
    "defaultValue",
    "disabled",
    "onChange",
    "class",
  ]);

  return (
    <ToggleGroup
      class={`kit-segment ${local.class ?? ""}`}
      aria-label={local.label}
      value={local.value}
      defaultValue={local.defaultValue}
      disabled={local.disabled}
      onChange={(v) => {
        // single-selection mode yields a string | null; ignore deselection.
        if (typeof v === "string" && v) local.onChange?.(v);
      }}
    >
      <For each={local.options}>
        {(opt) => (
          <ToggleGroup.Item
            class="kit-segment__item kit-focusable"
            value={opt.value}
            disabled={opt.disabled}
          >
            {opt.label}
          </ToggleGroup.Item>
        )}
      </For>
    </ToggleGroup>
  );
}

export default SegmentBar;
