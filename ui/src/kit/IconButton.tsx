/**
 * IconButton — icon-only action built on Kobalte Button. Requires a `label`
 * that becomes the accessible name (Req 17.2: labeled controls; never rely on
 * the icon alone). Renders the bundled Lucide sprite via <Icon>.
 */
import { Button as KButton } from "@kobalte/core/button";
import { splitProps, type JSX } from "solid-js";
import { Icon } from "../components/Icon";
import "./kit.base.css";
import "./IconButton.css";

export type IconButtonVariant = "ghost" | "solid" | "danger";
export type IconButtonSize = "sm" | "md" | "lg";

export interface IconButtonProps
  extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  /** Lucide icon id in the sprite. */
  icon: string;
  /** Accessible name — required (aria-label). */
  label: string;
  variant?: IconButtonVariant;
  size?: IconButtonSize;
}

export function IconButton(props: IconButtonProps) {
  const [local, rest] = splitProps(props, [
    "icon",
    "label",
    "variant",
    "size",
    "class",
  ]);
  const variant = () => local.variant ?? "ghost";
  const size = () => local.size ?? "md";

  return (
    <KButton
      class={`kit-icon-button kit-focusable kit-transition kit-icon-button--${variant()} kit-icon-button--${size()} ${local.class ?? ""}`}
      aria-label={local.label}
      {...rest}
    >
      <Icon name={local.icon} />
    </KButton>
  );
}

export default IconButton;
