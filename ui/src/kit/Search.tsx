/**
 * Search — a search input built on Kobalte TextField with a leading icon and
 * type="search" semantics (role "searchbox"). Label is required for a11y but
 * defaults to visually hidden (Req 17.2).
 */
import { TextField } from "@kobalte/core/text-field";
import { splitProps, Show } from "solid-js";
import { Icon } from "../components/Icon";
import "./kit.base.css";
import "./field.css";

export interface SearchProps {
  label?: string;
  showLabel?: boolean;
  value?: string;
  defaultValue?: string;
  placeholder?: string;
  name?: string;
  disabled?: boolean;
  onChange?: (value: string) => void;
  class?: string;
}

export function Search(props: SearchProps) {
  const [local] = splitProps(props, [
    "label",
    "showLabel",
    "value",
    "defaultValue",
    "placeholder",
    "name",
    "disabled",
    "onChange",
    "class",
  ]);
  const label = () => local.label ?? "Search";

  return (
    <TextField
      class={`kit-field ${local.class ?? ""}`}
      value={local.value}
      defaultValue={local.defaultValue}
      name={local.name}
      disabled={local.disabled}
      onChange={local.onChange}
    >
      <TextField.Label
        class={local.showLabel ? "kit-field__label" : "kit-visually-hidden"}
      >
        {label()}
      </TextField.Label>
      <div class="kit-search">
        <span class="kit-search__icon">
          <Icon name="search" size={16} />
        </span>
        <TextField.Input
          class="kit-field__control"
          type="search"
          placeholder={local.placeholder ?? label()}
        />
      </div>
    </TextField>
  );
}

export default Search;
