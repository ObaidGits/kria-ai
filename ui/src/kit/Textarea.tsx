/**
 * Textarea — multi-line text field on Kobalte TextField. Same field grammar as
 * Input (label + error + states). Optional grow-to-fit via autoResize.
 */
import { TextField } from "@kobalte/core/text-field";
import { splitProps, Show, type ComponentProps } from "solid-js";
import "./kit.base.css";
import "./field.css";

export interface TextareaProps {
  label?: string;
  hideLabel?: boolean;
  value?: string;
  defaultValue?: string;
  placeholder?: string;
  name?: string;
  disabled?: boolean;
  rows?: number;
  autoResize?: boolean;
  errorMessage?: string;
  onChange?: (value: string) => void;
  textareaProps?: Omit<ComponentProps<typeof TextField.TextArea>, "class" | "placeholder" | "rows" | "autoResize">;
  class?: string;
}

export function Textarea(props: TextareaProps) {
  const [local] = splitProps(props, [
    "label",
    "hideLabel",
    "value",
    "defaultValue",
    "placeholder",
    "name",
    "disabled",
    "rows",
    "autoResize",
    "errorMessage",
    "onChange",
    "textareaProps",
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
      <TextField.TextArea
        class="kit-field__control kit-field__textarea"
        placeholder={local.placeholder}
        rows={local.rows}
        autoResize={local.autoResize}
        {...local.textareaProps}
      />
      <Show when={local.errorMessage}>
        <TextField.ErrorMessage class="kit-field__error">
          {local.errorMessage}
        </TextField.ErrorMessage>
      </Show>
    </TextField>
  );
}

export default Textarea;
