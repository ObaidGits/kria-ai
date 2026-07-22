/**
 * ActionChips — low-weight suggestions that naturally follow the Composer.
 *
 * These are suggestions, not navigation: quiet transparent pills that recede
 * beneath the Composer. Static demo content from `./data`.
 */
import { For } from "solid-js";
import { CcIcon } from "./CcIcon";
import { ACTION_CHIPS } from "./data";

export function ActionChips() {
  return (
    <div class="cc-chips" role="list" aria-label="Suggested actions">
      <For each={ACTION_CHIPS}>
        {(chip) => (
          <button type="button" class="cc-chip" role="listitem">
            <CcIcon name={chip.icon} size={14} />
            {chip.label}
          </button>
        )}
      </For>
    </div>
  );
}

export default ActionChips;
