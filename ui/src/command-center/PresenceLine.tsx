/**
 * PresenceLine — the single living sentence directly beneath the Core.
 *
 * KRIA's current awareness expressed as ONE calm message at a time (not a feed).
 * Phase 6: the message is context-aware — it reflects `currentContext()` from
 * the Context Engine. `aria-live="polite"` announces context changes without
 * stealing focus.
 */
import { Show } from "solid-js";
import { currentContext } from "./context";

export function PresenceLine() {
  return (
    <p class="cc-pline" role="status" aria-live="polite">
      {currentContext().presence}
      <Show when={currentContext().accent}>
        <span class="cc-pline__accent">{currentContext().accent}</span>
      </Show>
    </p>
  );
}

export default PresenceLine;
