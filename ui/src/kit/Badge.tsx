/**
 * Badge — inline semantic label (counts, statuses, tags).
 */
import { splitProps, type ParentProps } from "solid-js";
import "./Badge.css";

export type BadgeTone =
  | "neutral"
  | "accent"
  | "success"
  | "warning"
  | "danger"
  | "info";

export interface BadgeProps extends ParentProps {
  tone?: BadgeTone;
  class?: string;
}

export function Badge(props: BadgeProps) {
  const [local] = splitProps(props, ["tone", "class", "children"]);
  const tone = () => local.tone ?? "neutral";
  return (
    <span class={`kit-badge kit-badge--${tone()} ${local.class ?? ""}`}>
      {local.children}
    </span>
  );
}

export default Badge;
