/**
 * HomeComposer — the primary interaction point, aligned on the Core's axis.
 *
 * Presented as "speaking with KRIA" rather than a chat app's input box: a calm
 * centered field with a voice affordance and a ⌘K hint. Frontend-only demo —
 * no submit wiring, no stores.
 */
import { CcIcon } from "./CcIcon";
import { currentContext } from "./context";

export function HomeComposer() {
  return (
    <div class="cc-composer">
      <input
        type="text"
        class="cc-composer__field"
        placeholder={currentContext().placeholder}
        aria-label="Talk to KRIA, or type a message"
      />
      <kbd class="cc-composer__kbd">⌘K</kbd>
      <button type="button" class="cc-composer__mic" aria-label="Speak to KRIA">
        <CcIcon name="mic" size={18} />
      </button>
    </div>
  );
}

export default HomeComposer;
