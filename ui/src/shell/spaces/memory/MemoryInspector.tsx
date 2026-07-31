/**
 * MemoryInspector — the shared Inspector's body for `type: "memory"` targets
 * (task 6.2, Req 5.2 / 5.3). Registered via `registerInspectorRenderer("memory",
 * …)` so selecting a MemoryCard opens THIS body in the one shared Inspector
 * (Req 1.6). It fetches the full detail through the existing `memory_explain`
 * command and discloses every Req-5.2 field, then offers the Req-5.3 actions.
 *
 * Req 5.2 disclosures: content · confidence · worth · verification/truth state ·
 * staleness · source · conflicts/contradictions · lineage (derived-from /
 * superseded-by) · version lineage · an AI explanation.
 *
 * Req 5.3 actions (each routes through an EXISTING memory_* command):
 *   • verify  → memory_verify
 *   • correct → memory_correct  (inline editor, stable backend identity)
 *   • reinforce/penalize → memory_record_feedback(thumbs_up/thumbs_down)
 *   • forget  → memory_forget  (REVERSIBLE → memory_restore_forgotten)
 *   • hard-delete → memory_hard_delete  (IRREVERSIBLE → deliberate Confirm)
 *
 * HONEST STATES (fixes the old silent-failure explain/action bug): loading,
 * error-with-retry, and "no longer exists" are all shown; EVERY action reports
 * success/failure through the Notification Center — never a silent no-op.
 *
 * SECURITY: detail content is UNTRUSTED. Plain fields render as text (Solid
 * escapes). The AI explanation is composed as markdown and passed through
 * `renderMarkdown` (DOMPurify) before it reaches `innerHTML` — no un-sanitized
 * model/tool HTML is ever rendered (design.md §1.17).
 *
 * ARCHITECTURE: presentation + memory-command dispatch only. Mutations go
 * through the runtime-authoritative memory_* commands via `memoryStore`; the UI
 * never bypasses, orchestrates, or takes a prompt→tool shortcut.
 *
 * Requirements: 5.2, 5.3, 17.2, 17.3
 */
import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import { memoryStore, shellStore, notificationStore } from "../../../stores";
import type { InspectorTarget } from "../../../stores/shellStore";
import type { MemoryActionResult, MemoryDetail } from "../../../stores";
import { Badge, Button, Confirm, ProvenanceCue } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { renderMarkdown } from "../../../lib/markdown";
import {
  confidenceCue,
  sampledWorthCue,
  stalenessClassCue,
  stateCue,
  type MemoryCue,
} from "./memoryCues";
import "./MemoryInspector.css";

// ─── Notification helpers (honest success/failure — Req 5.3) ─────────────────

function notifyOk(message: string): void {
  notificationStore.push({
    id: `mem-ok-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
    level: "success",
    message,
    source: "memory",
  });
}

function notifyFail(message: string): void {
  notificationStore.push({
    id: `mem-err-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
    level: "error",
    message,
    source: "memory",
  });
}

// ─── Small presentation atoms ────────────────────────────────────────────────

function CueBadge(props: { cue: MemoryCue }) {
  return (
    <Badge tone={props.cue.tone} class="kria-mem-insp__cue">
      <Icon name={props.cue.icon} size={13} />
      <span>{props.cue.label}</span>
    </Badge>
  );
}

function Field(props: { label: string; children: JSX.Element }) {
  return (
    <div class="kria-mem-insp__field">
      <dt class="kria-mem-insp__field-label">{props.label}</dt>
      <dd class="kria-mem-insp__field-value">{props.children}</dd>
    </div>
  );
}

/** Compose a short, plain-language AI explanation, sanitized before render. */
function explanationHtml(d: MemoryDetail): string {
  const conf = Math.round(d.confidence * 100);
  const parts: string[] = [];
  parts.push(`This memory is currently **${d.state}**, held with **${conf}% confidence**.`);
  if (d.sourceEventTag) parts.push(`It was learned from \`${d.sourceEventTag}\`.`);
  if (d.worthSamples > 0) {
    parts.push(
      `Its worth has been sampled ${d.worthSamples} time(s) — ${d.worthSuccess} useful, ${d.worthFailure} not.`,
    );
  } else {
    parts.push("Its usefulness has not been sampled yet.");
  }
  if (d.contradicts.length > 0) {
    parts.push(`It conflicts with ${d.contradicts.length} other memory(ies).`);
  }
  if (d.supersededBy) parts.push("A newer memory has superseded it.");
  else if (d.derivedFrom.length > 0) parts.push(`It was derived from ${d.derivedFrom.length} earlier memory(ies).`);
  return renderMarkdown(parts.join(" "));
}

// ─── Body ────────────────────────────────────────────────────────────────────

export interface MemoryInspectorProps {
  target: InspectorTarget;
}

export function MemoryInspector(props: MemoryInspectorProps) {
  const memoryId = () => props.target.id;

  // Honest fetch of the full detail (Req 5.2). createResource gives us loading
  // and re-fetch for free; the promise resolves to a MemoryActionResult so a
  // command failure is a first-class, surfaced state (not a silent empty body).
  const [detail, { refetch }] = createResource(
    memoryId,
    (id): Promise<MemoryActionResult<MemoryDetail | null>> => memoryStore.fetchDetail(id),
  );

  const [busy, setBusy] = createSignal<string | null>(null);
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal("");
  const [forgotten, setForgotten] = createSignal(false);

  const result = () => detail();
  const data = (): MemoryDetail | null => {
    const r = result();
    return r && r.ok ? r.data : null;
  };

  /** Run an action, surface the outcome honestly, and refresh detail on success. */
  async function run(
    label: string,
    fn: () => Promise<MemoryActionResult<unknown>>,
    okMessage: string,
    opts: { refetch?: boolean } = { refetch: true },
  ): Promise<boolean> {
    setBusy(label);
    const res = await fn();
    setBusy(null);
    if (res.ok) {
      notifyOk(okMessage);
      if (opts.refetch) void refetch();
      return true;
    }
    notifyFail(res.message);
    return false;
  }

  function startCorrect(): void {
    setDraft(data()?.content ?? "");
    setEditing(true);
  }

  async function saveCorrect(): Promise<void> {
    const ok = await run(
      "correct",
      () => memoryStore.correct(memoryId(), draft()),
      "Correction recorded",
    );
    if (ok) setEditing(false);
  }

  async function doForget(): Promise<void> {
    const ok = await run(
      "forget",
      () => memoryStore.forget(memoryId()),
      "Memory forgotten — you can undo this",
      { refetch: false },
    );
    if (ok) setForgotten(true);
  }

  async function doUndo(): Promise<void> {
    const ok = await run(
      "undo",
      () => memoryStore.undoForget(),
      "Memory restored",
      { refetch: false },
    );
    if (ok) {
      setForgotten(false);
      void refetch();
    }
  }

  async function doHardDelete(): Promise<void> {
    const ok = await run(
      "hard-delete",
      () => memoryStore.hardDelete(memoryId()),
      "Memory permanently deleted",
      { refetch: false },
    );
    if (ok) shellStore.closeInspector();
  }

  return (
    <div class="kria-mem-insp">
      {/* Loading — honest, not a silent blank body. */}
      <Show when={detail.loading}>
        <p class="kria-mem-insp__status" role="status" aria-live="polite">
          Loading memory detail…
        </p>
      </Show>

      <Show when={!detail.loading && result() && !result()!.ok}>
        <div class="kria-mem-insp__error" role="alert">
          <p>Couldn’t load this memory.</p>
          <p class="kria-mem-insp__error-detail">
            {result() && !result()!.ok ? (result() as { message: string }).message : ""}
          </p>
          <Button variant="secondary" size="sm" onClick={() => void refetch()}>
            <Icon name="refresh-cw" size={14} /> Retry
          </Button>
        </div>
      </Show>

      <Show when={!detail.loading && result()?.ok && data() === null && !forgotten()}>
        <p class="kria-mem-insp__status" role="status">
          This memory no longer exists.
        </p>
      </Show>

      {/* Forgotten (reversible) — inline Undo (Req 5.3). */}
      <Show when={forgotten()}>
        <div class="kria-mem-insp__forgotten" role="status" aria-live="polite">
          <p>
            <Icon name="eye-off" size={14} aria-hidden /> This memory was forgotten. This is
            reversible.
          </p>
          <Button variant="secondary" size="sm" disabled={busy() === "undo"} onClick={() => void doUndo()}>
            <Icon name="rotate-ccw" size={14} /> Undo
          </Button>
        </div>
      </Show>

      <Show when={!forgotten() && data()}>
        {(d) => (
          <>
            {/* Content (untrusted → plain text, auto-escaped). */}
            <section class="kria-mem-insp__content" aria-label="Content">
              <Show when={!editing()} fallback={<CorrectEditor draft={draft()} setDraft={setDraft} busy={busy() === "correct"} onSave={() => void saveCorrect()} onCancel={() => setEditing(false)} />}>
                <p class="kria-mem-insp__content-text">{d().content}</p>
              </Show>
            </section>

            {/* Cues — icon+text, never color-only (Req 17.3). */}
            <div class="kria-mem-insp__cues" aria-label="Signals">
              <CueBadge cue={confidenceCue(d().confidence)} />
              <CueBadge cue={stateCue(d().state)} />
              <CueBadge cue={sampledWorthCue(d().worthSuccess, d().worthFailure, d().worthSamples)} />
              <CueBadge cue={stalenessClassCue(d().stalenessClass)} />
            </div>

            {/* Detail fields (Req 5.2). */}
            <dl class="kria-mem-insp__fields">
              <Field label="Type">{d().memoryType}</Field>
              <Field label="Verification / truth state">{d().state}</Field>
              <Field label="Confidence">{Math.round(d().confidence * 100)}%</Field>
              <Field label="Importance">{Math.round(d().importance * 100)}%</Field>
              <Field label="Worth">
                {d().worthSamples > 0
                  ? `${d().worthSuccess} useful / ${d().worthFailure} not (${d().worthSamples} samples)`
                  : "untested"}
              </Field>
              <Field label="Staleness">{d().stalenessClass}</Field>
              <Field label="Source">{d().sourceEventTag ?? "unknown"}</Field>
              <Field label="Times accessed">{d().accessCount}</Field>
            </dl>

            {/* Conflicts / contradictions (Req 5.2). */}
            <section class="kria-mem-insp__section" aria-label="Conflicts">
              <h3 class="kria-mem-insp__section-title">
                <Icon name="alert-triangle" size={14} aria-hidden /> Conflicts
              </h3>
              <Show
                when={d().contradicts.length > 0}
                fallback={<p class="kria-mem-insp__muted">No known contradictions.</p>}
              >
                <ul class="kria-mem-insp__idlist">
                  <For each={d().contradicts}>{(cid) => <li>{cid}</li>}</For>
                </ul>
              </Show>
            </section>

            {/* Lineage / version history (Req 5.2). */}
            <section class="kria-mem-insp__section" aria-label="Lineage and version history">
              <h3 class="kria-mem-insp__section-title">
                <Icon name="git-branch" size={14} aria-hidden /> Lineage &amp; versions
              </h3>
              <Field label="Derived from">
                <Show when={d().derivedFrom.length > 0} fallback={<span class="kria-mem-insp__muted">—</span>}>
                  <ul class="kria-mem-insp__idlist">
                    <For each={d().derivedFrom}>{(pid) => <li>{pid}</li>}</For>
                  </ul>
                </Show>
              </Field>
              <Field label="Superseded by">
                <Show when={d().supersededBy} fallback={<span class="kria-mem-insp__muted">— (current version)</span>}>
                  {d().supersededBy}
                </Show>
              </Field>
            </section>

            {/* AI explanation — composed markdown, sanitized before render. */}
            <section
              class="kria-mem-insp__section"
              aria-label="AI explanation"
              data-provenance="kria"
            >
              <h3 class="kria-mem-insp__section-title">
                <Icon name="sparkles" size={14} aria-hidden /> AI explanation
              </h3>
              <ProvenanceCue source="kria" label="Explained by KRIA" />
              {/* eslint-disable-next-line solid/no-innerhtml */}
              <div class="kria-mem-insp__explanation" innerHTML={explanationHtml(d())} />
            </section>

            {/* Actions (Req 5.3). Each button pairs an icon WITH a text label. */}
            <section class="kria-mem-insp__actions" aria-label="Actions">
              <Button variant="secondary" size="sm" disabled={busy() === "verify"} onClick={() => void run("verify", () => memoryStore.verify(memoryId()), "Verification requested")}>
                <Icon name="check-circle" size={14} /> Verify
              </Button>
              <Button variant="secondary" size="sm" disabled={editing()} onClick={startCorrect}>
                <Icon name="pencil" size={14} /> Correct
              </Button>
              <Button variant="secondary" size="sm" disabled={busy() === "reinforce"} onClick={() => void run("reinforce", () => memoryStore.reinforce(memoryId()), "Memory reinforced")}>
                <Icon name="star" size={14} /> Reinforce
              </Button>
              <Button variant="secondary" size="sm" disabled={busy() === "penalize"} onClick={() => void run("penalize", () => memoryStore.penalize(memoryId()), "Memory penalized")}>
                <Icon name="alert-triangle" size={14} /> Penalize
              </Button>
              <Button variant="ghost" size="sm" disabled={busy() === "forget"} onClick={() => void doForget()}>
                <Icon name="eye-off" size={14} /> Forget
              </Button>
              {/* Deliberate confirmation for the irreversible hard-delete
                  (Req 5.3). The Confirm's own trigger opens a focus-trapped
                  dialog; danger risk is shown with icon+text, never color-only
                  (Req 17.3). The danger-styled trigger keeps the destructive
                  affordance visible in the actions row. */}
              <Confirm
                triggerLabel="Hard delete"
                title="Permanently delete this memory?"
                message="This permanently deletes the memory (Hard Delete — content marked for removal). No cryptographic erasure is available yet; data remains on disk until OS-level disk encryption or physical media destruction. Use Forget instead if you might want it back."
                confirmLabel="Delete permanently"
                cancelLabel="Keep memory"
                risk="danger"
                onConfirm={() => void doHardDelete()}
              />
            </section>
          </>
        )}
      </Show>
    </div>
  );
}

// ─── Inline correction editor ────────────────────────────────────────────────

function CorrectEditor(props: {
  draft: string;
  setDraft: (v: string) => void;
  busy: boolean;
  onSave: () => void;
  onCancel: () => void;
}) {
  return (
    <div class="kria-mem-insp__editor">
      <label class="kit-visually-hidden" for="kria-mem-correct">
        Corrected content
      </label>
      <textarea
        id="kria-mem-correct"
        class="kria-mem-insp__textarea"
        value={props.draft}
        onInput={(e) => props.setDraft(e.currentTarget.value)}
      />
      <div class="kria-mem-insp__editor-actions">
        <Button variant="ghost" size="sm" onClick={() => props.onCancel()}>
          Cancel
        </Button>
        <Button variant="primary" size="sm" disabled={props.busy} onClick={() => props.onSave()}>
          <Icon name="check" size={14} /> Save correction
        </Button>
      </div>
    </div>
  );
}

export default MemoryInspector;
