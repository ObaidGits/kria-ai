/**
 * Input — single-line text field on Kobalte TextField (design.md §1.5).
 * Provides an associated <label> and optional error message (Req 17.2/17.3).
 * States: default/hover/focus-visible/disabled/invalid.
 */
import { TextField } from "@kobalte/core/text-field";
import { splitProps, Show, type ComponentProps } from "solid-js";
import "./kit.base.css";
import "./field.css";

export interface InputProps {
  label?: string;
  /** Visually hide the label but keep it for assistive tech. */
  hideLabel?: boolean;
  value?: string;
  defaultValue?: string;
  placeholder?: string;
  type?: string;
  name?: string;
  disabled?: boolean;
  /** When set the field is marked invalid and the message is shown. */
  errorMessage?: string;
  onChange?: (value: string) => void;
  inputProps?: Omit<ComponentProps<typeof TextField.Input>, "class" | "type" | "placeholder">;
  class?: string;
}

export function Input(props: InputProps) {
  const [local] = splitProps(props, [
    "label",
    "hideLabel",
    "value",
    "defaultValue",
    "placeholder",
    "type",
    "name",
    "disabled",
    "errorMessage",
    "onChange",
    "inputProps",
    "class",
  ]);

  return (
    <TextField
      class={`kit-field ${local.class ?? ""}`}
      value={local.value}
      defaultValue={local.defaultValue}
      name={local.name}
      disabled={local.disabled}
      validationState={local.errorMessage ? "invalid" : "valid"}
      onChange={local.onChange}
    >
      <Show when={local.label}>
        <TextField.Label
          class={local.hideLabel ? "kit-visually-hidden" : "kit-field__label"}
        >
          {local.label}
        </TextField.Label>
      </Show>
      <TextField.Input
        class="kit-field__control"
        type={local.type ?? "text"}
        placeholder={local.placeholder}
        {...local.inputProps}
      />
      <Show when={local.errorMessage}>
        <TextField.ErrorMessage class="kit-field__error">
          {local.errorMessage}
        </TextField.ErrorMessage>
      </Show>
    </TextField>
  );
}

export default Input;
