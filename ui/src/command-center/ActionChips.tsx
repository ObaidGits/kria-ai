/** Four prioritized contextual actions for the primary command surface. */
import { For } from "solid-js";
import { CcIcon } from "./CcIcon";
import { currentCognition } from "./context";

export function ActionChips(props: { onSelect: (label: string) => void }) {
  const actions = () => [
    { id: "continue", label: currentCognition().nextAction, icon: "play", priority: "primary" },
    { id: "review", label: "Review changes", icon: "git", priority: "secondary" },
    { id: "memory", label: "Search memory", icon: "search", priority: "secondary" },
    { id: "more", label: "More", icon: "plus", priority: "quiet" },
  ] as const;

  return (
    <div class="cc-chips" role="group" aria-label="Suggested requests">
      <For each={actions()}>
        {(action) => (
          <button
            type="button"
            class="cc-chip"
            data-priority={action.priority}
            onClick={() => props.onSelect(action.id === "more" ? "Show more useful actions" : action.label)}
          >
            <CcIcon name={action.icon} size={14} />
            <span>{action.label}</span>
          </button>
        )}
      </For>
    </div>
  );
}

export default ActionChips;
