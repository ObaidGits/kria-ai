/**
 * EvidenceViewer — shows a workflow run's evidence/output (task 7.2, Req 6.5).
 *
 * Reused concept from the Converse WorkBlock evidence pattern (task 3.3): a
 * labelled list of the sources/artifacts a run used or produced. Each item's
 * `detail` is UNTRUSTED (model/tool/n8n-authored) and is ALWAYS run through the
 * shared sanitizer (`sanitizeHtml`) before it reaches the DOM — no run output is
 * ever rendered un-sanitized. `href` links open in a new, `noopener` tab.
 *
 * Presentation only — no command dispatch, no orchestration.
 *
 * Requirements: 6.5
 */
import { For, Show } from "solid-js";
import { sanitizeHtml } from "../../../lib/markdown";
import { EmptyState } from "../../../kit";
import type { RunEvidenceItem } from "../../../stores";
import "./run.css";

export interface EvidenceViewerProps {
  evidence: readonly RunEvidenceItem[];
  /** Section heading; defaults to "Evidence". */
  title?: string;
  /**
   * When true, an honest empty state renders instead of nothing when there is
   * no evidence yet. Defaults to false (the caller decides whether to mount).
   */
  showEmpty?: boolean;
}

export function EvidenceViewer(props: EvidenceViewerProps) {
  const items = () => props.evidence ?? [];
  const title = () => props.title ?? "Evidence";

  return (
    <Show
      when={items().length > 0}
      fallback={
        <Show when={props.showEmpty}>
          <EmptyState
            icon="search"
            title="No evidence yet"
            description="When this run produces output or uses a source, it will appear here."
          />
        </Show>
      }
    >
      <section class="kria-evidence" data-region="run-evidence" aria-label={title()}>
        <h3 class="kria-evidence__title">{title()}</h3>
        <ul class="kria-evidence__list">
          <For each={items()}>
            {(item) => (
              <li class="kria-evidence__item">
                <Show
                  when={item.href}
                  fallback={<span class="kria-evidence__label">{item.label}</span>}
                >
                  <a
                    class="kria-evidence__label"
                    href={item.href}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    {item.label}
                  </a>
                </Show>
                <Show when={item.detail}>
                  {/* Untrusted run output → sanitized before display. */}
                  <div class="kria-evidence__detail" innerHTML={sanitizeHtml(item.detail!)} />
                </Show>
              </li>
            )}
          </For>
        </ul>
      </section>
    </Show>
  );
}

export default EvidenceViewer;
