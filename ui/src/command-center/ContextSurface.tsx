/**
 * ContextSurface — the single Adaptive Context Surface beneath the Composer.
 *
 * Shows the ONE most-relevant thing KRIA wants surfaced right now. When
 * `CONTEXT_SUBJECT` is `null` the surface dissolves entirely — no placeholder,
 * no empty card — so the homepage stays calm. Static demo content.
 */
import { Show } from "solid-js";
import { CcIcon } from "./CcIcon";
import { CONTEXT_SUBJECT } from "./data";

export function ContextSurface() {
  return (
    <Show when={CONTEXT_SUBJECT} keyed>
      {(subject) => (
        <div class="cc-context" role="status">
          <span class="cc-context__icon"><CcIcon name={subject.icon} size={20} /></span>
          <div class="cc-context__text">
            <span class="cc-context__title">{subject.title}</span>
            <span class="cc-context__line">{subject.line}</span>
            <span class="cc-context__meta">
              <span class="cc-context__time"><CcIcon name="clock" size={13} /> {subject.time}</span>
              <span class="cc-context__priority"><span class="cc-dot cc-dot--warn" /> {subject.priority}</span>
            </span>
          </div>
          <button type="button" class="cc-context__action">{subject.action} <CcIcon name="chevron" size={13} /></button>
        </div>
      )}
    </Show>
  );
}

export default ContextSurface;
