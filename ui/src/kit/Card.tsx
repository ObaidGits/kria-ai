/**
 * Card — content container. Interactive cards render a semantic button;
 * static cards render a div (Req 14.5 / 17.1).
 */
import { Show, splitProps, type JSX, type ParentProps } from "solid-js";
import "./kit.base.css";
import "./Card.css";

export type CardProps = ParentProps<{
  variant?: "default" | "elevated";
  interactive?: boolean;
  class?: string;
  onClick?: (event: MouseEvent) => void;
}> & Omit<JSX.HTMLAttributes<HTMLDivElement>, "children" | "class" | "onClick">;

export function Card(props: CardProps) {
  const [local, rest] = splitProps(props, ["variant", "interactive", "class", "children", "onClick"]);
  const isInteractive = () => local.interactive || !!local.onClick;
  const className = () =>
    `kit-card kit-card--${local.variant ?? "default"} ${isInteractive() ? "kit-card--interactive kit-focusable" : ""} ${local.class ?? ""}`;

  return (
    <Show
      when={isInteractive()}
      fallback={<div class={className()} onClick={(event) => local.onClick?.(event)} {...rest}>{local.children}</div>}
    >
      <button
        type="button"
        class={className()}
        onClick={(event) => local.onClick?.(event)}
        {...(rest as JSX.ButtonHTMLAttributes<HTMLButtonElement>)}
      >
        {local.children}
      </button>
    </Show>
  );
}

export default Card;
