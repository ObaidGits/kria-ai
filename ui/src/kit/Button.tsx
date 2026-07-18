/**
 * Button — built on Kobalte's accessible Button primitive (design.md §1.5/§4.2).
 * One component per concept (Req 14.4). Variants + sizes + full interaction
 * states with a visible focus ring (Req 14.5 / 17.1). Semantic <button> with
 * correct disabled semantics for free from Kobalte (Req 17.2).
 */
import { Button as KButton } from "@kobalte/core/button";
import { splitProps, type JSX, type ParentProps } from "solid-js";
import "./kit.base.css";
import "./Button.css";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps
  extends ParentProps,
    JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

export function Button(props: ButtonProps) {
  const [local, rest] = splitProps(props, ["variant", "size", "class", "children"]);
  const variant = () => local.variant ?? "primary";
  const size = () => local.size ?? "md";

  return (
    <KButton
      class={`kit-button kit-focusable kit-transition kit-button--${variant()} kit-button--${size()} ${local.class ?? ""}`}
      {...rest}
    >
      {local.children}
    </KButton>
  );
}

export default Button;
