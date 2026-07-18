/**
 * Select — single-select dropdown on Kobalte Select (design.md §1.5). Correct
 * listbox ARIA, keyboard nav, typeahead, and focus management for free.
 * Options are {value,label}; label + optional error supported (Req 17.2/17.3).
 */
import { Select as KSelect } from "@kobalte/core/select";
import { splitProps, Show } from "solid-js";
import { Icon } from "../components/Icon";
import "./kit.base.css";
import "./floating.css";
import "./Select.css";

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SelectProps {
  label?: string;
  hideLabel?: boolean;
  options: SelectOption[];
  value?: string;
  defaultValue?: string;
  placeholder?: string;
  disabled?: boolean;
  errorMessage?: string;
  onChange?: (value: string | undefined) => void;
  class?: string;
}

export function Select(props: SelectProps) {
  const [local] = splitProps(props, [
    "label",
    "hideLabel",
    "options",
    "value",
    "defaultValue",
    "placeholder",
    "disabled",
    "errorMessage",
    "onChange",
    "class",
  ]);

  const selectedOption = (value: string | undefined) =>
    local.options.find((o) => o.value === value);

  return (
    <KSelect<SelectOption>
      class={`kit-select ${local.class ?? ""}`}
      options={local.options}
      optionValue="value"
      optionTextValue="label"
      optionDisabled="disabled"
      value={selectedOption(local.value)}
      defaultValue={selectedOption(local.defaultValue)}
      disabled={local.disabled}
      validationState={local.errorMessage ? "invalid" : "valid"}
      placeholder={local.placeholder ?? "Select…"}
      onChange={(opt) => local.onChange?.(opt?.value)}
      itemComponent={(itemProps) => (
        <KSelect.Item item={itemProps.item} class="kit-menu-item">
          <KSelect.ItemLabel>{itemProps.item.rawValue.label}</KSelect.ItemLabel>
          <KSelect.ItemIndicator class="kit-menu-item__indicator">
            <Icon name="check" size={16} />
          </KSelect.ItemIndicator>
        </KSelect.Item>
      )}
    >
      <Show when={local.label}>
        <KSelect.Label
          class={local.hideLabel ? "kit-visually-hidden" : "kit-select__label"}
        >
          {local.label}
        </KSelect.Label>
      </Show>
      <KSelect.Trigger class="kit-select__trigger kit-focusable">
        <KSelect.Value<SelectOption> class="kit-select__value">
          {(state) => state.selectedOption().label}
        </KSelect.Value>
        <KSelect.Icon class="kit-select__icon">
          <Icon name="chevron-down" size={16} />
        </KSelect.Icon>
      </KSelect.Trigger>
      <Show when={local.errorMessage}>
        <KSelect.ErrorMessage class="kit-select__error">
          {local.errorMessage}
        </KSelect.ErrorMessage>
      </Show>
      <KSelect.Portal>
        <KSelect.Content class="kit-floating">
          <KSelect.Listbox class="kit-select__listbox" />
        </KSelect.Content>
      </KSelect.Portal>
    </KSelect>
  );
}

export default Select;
