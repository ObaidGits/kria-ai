/**
 * EmptyState — calm empty/first-run placeholder. Uses a real heading for
 * correct document structure (Req 17.2).
 */
import { splitProps, Show, type JSX } from "solid-js";
import { Icon } from "../components/Icon";
import "./EmptyState.css";

export interface EmptyStateProps {
  icon?: string;
  title: string;
  description?: JSX.Element;
  action?: JSX.Element;
  class?: string;
}

export function EmptyState(props: EmptyStateProps) {
  const [local] = splitProps(props, ["icon", "title", "description", "action", "class"]);
  return (
    <div class={`kit-empty ${local.class ?? ""}`}>
      <Show when={local.icon}>
        <span class="kit-empty__icon" aria-hidden="true">
          <Icon name={local.icon!} />
        </span>
      </Show>
      <h2 class="kit-empty__title">{local.title}</h2>
      <Show when={local.description}>
        <p class="kit-empty__description">{local.description}</p>
      </Show>
      <Show when={local.action}>
        <div class="kit-empty__action">{local.action}</div>
      </Show>
    </div>
  );
}

export default EmptyState;
