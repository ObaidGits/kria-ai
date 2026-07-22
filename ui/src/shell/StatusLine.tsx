/**
 * StatusLine — the ONE status line of the shell (Req 1.1). It is the single
 * persistent HOME for the Core textual state and its concise narration —
 * including error/recovery state, which is the reason StatusLine survives
 * consolidation (design.md UIE-M-012, §20 status priority). The Core state sits
 * in a polite live region so assistive tech announces changes (Req 17.2 live
 * regions) without stealing focus.
 *
 * ONE-FACT-ONE-HOME (design.md §8.6, Req 9.4/9.5, UIE-M-012, task 5.5): the
 * StatusLine deliberately does NOT re-state facts that already have a persistent
 * owner elsewhere:
 *   • Active Space is owned by the Dock (aria-current="page", one-click switch).
 *     The old non-actionable Space label here was a pure duplicate → removed.
 *   • Pending approvals are owned by the PresenceBar shield (badged, GLOW
 *     attention, opens the Approval Center — the safety-critical action). The
 *     old StatusLine approval COUNT was non-actionable text with no distinct
 *     safety action, so §8.6 does not justify a second placement → removed.
 * No fact is lost: both facts remain visible/actionable at their owners.
 *
 * Per Req 9.2 the Observatory + Core + this status line are the only places
 * telemetry surfaces — this line stays intentionally minimal.
 *
 * UNIFORM FOOTPRINT (supersedes the former UIE-L-001 idle minimization): the
 * StatusLine now ALWAYS carries a unique persistent fact — the Brain (LLM)
 * status (connected / disconnected / model). The old minimization collapsed the
 * line when the Core was idle, which — with a persistent fact present — made the
 * footer visibly appear/disappear as the Core toggled idle↔active. To keep the
 * chrome stable, the line holds ONE uniform footprint at all times: the Core dot
 * + label, optional narration, and the Brain status. It is never collapsed or
 * removed. Scoped Stop / approval controls live at their own owners (WorkBlock,
 * PresenceBar shield) and are unaffected.
 *
 * Requirements: 1.1, 9.4, 9.5, 17.2
 */
import { Show, createMemo } from "solid-js";
import { capabilityStore, coreStore } from "../stores";
import { coreNarration } from "../stores/coreNarration";
import { StatusDot } from "../kit";
import type { StatusTone } from "../kit";
import "./AppShell.css";

/** Map a Core state to a status-dot tone (label always accompanies — Req 17.3). */
function coreTone(): StatusTone {
  if (coreStore.state() === "error") return "error";
  if (coreStore.needsAttention()) return "busy";
  if (coreStore.isActive()) return "info";
  return "online";
}

interface BrainStatus {
  label: string;
  tone: StatusTone;
  pulse: boolean;
}

function brainStatus(): BrainStatus {
  const runtime = capabilityStore.activeLlmRuntime();
  const apply = capabilityStore.runtimeApplyStatus();
  const phase = capabilityStore.orchestratorPhase();

  const identity = runtime
    ? [runtime.displayName, runtime.activeModel]
        .map((value) => value.trim())
        .filter((value, index, values) => value.length > 0 && values.indexOf(value) === index)
        .join(" · ")
    : "";
  const withIdentity = (label: string) => (identity ? `${label} · ${identity}` : label);

  // 1) A provider/model apply in flight, or the local runtime (re)starting →
  //    show the honest initializing/starting state (highest priority).
  if (apply?.state === "switching" || phase === "starting") {
    return { label: withIdentity("Kria Brain: Initializing"), tone: "busy", pulse: true };
  }

  // 2) Explicit failure from an apply/rollback or a failed local-runtime swap.
  if (apply?.state === "failed" || apply?.state === "rollback_required" || phase === "failed") {
    return { label: withIdentity("Kria Brain: Failed"), tone: "error", pulse: false };
  }

  // 3) Still resolving the very first runtime read at boot → Starting.
  if (capabilityStore.llmRuntimeStatusLoading() && !runtime) {
    return { label: "Kria Brain: Starting", tone: "info", pulse: true };
  }

  // 4) No configured runtime at all.
  if (!runtime) {
    return { label: "Kria Brain: Disconnected", tone: "error", pulse: false };
  }

  // 5) Steady state — reflect health honestly.
  const connected = runtime.enabled && runtime.configured && runtime.routerHealthy;
  return {
    label: withIdentity(`Kria Brain: ${connected ? "Connected" : "Disconnected"}`),
    tone: connected ? "online" : "error",
    pulse: false,
  };
}

export function StatusLine() {
  // Concise situational text paired with the Core state (UIE-H-013, Req 8.5).
  // Read-only projection of authoritative signals; omitted (null) for idle and
  // any unmapped state so nothing is fabricated. This is additive text — it does
  // NOT change CorePresence visuals/motion (Req 8.6).
  const narration = coreNarration;
  const brain = createMemo(brainStatus);

  // The StatusLine now ALWAYS carries a unique persistent fact — the Brain (LLM)
  // status — so the old idle-minimization (UIE-L-001) no longer applies: it was
  // predicated on the resting line having nothing but the redundant Core label.
  // Collapsing now would make the footer visibly appear/disappear as the Core
  // toggles idle↔active (the non-uniformity users see). Keep one stable
  // footprint; the Core label is always shown alongside the Brain status.
  const minimized = () => false;

  return (
    <footer
      class="kria-statusline"
      role="contentinfo"
      data-state={minimized() ? "idle" : "active"}
      data-minimized={minimized() ? "true" : "false"}
    >
      <span class="kria-statusline__group" aria-live="polite">
        <StatusDot
          tone={coreTone()}
          label={coreStore.state()}
          pulse={coreStore.isActive()}
          hideLabel={minimized()}
        />
        <Show when={narration()}>
          {(n) => (
            <span
              class="kria-statusline__narration"
              data-region="core-narration"
              data-actionable={n().actionable ? "true" : "false"}
            >
              {n().text}
            </span>
          )}
        </Show>
      </span>
      {/* Ambient Brain (LLM) status. NOT a polite live region: the StatusLine
          keeps exactly ONE polite live region (the Core narration group) so
          announcements are not duplicated (Req 9.4/9.5/17.2). The StatusDot
          still carries an accessible label for on-demand SR inspection. */}
      <span
        class="kria-statusline__brain"
        data-region="llm-runtime-status"
        title={capabilityStore.llmRuntimeStatusError() ?? brain().label}
      >
        <StatusDot tone={brain().tone} label={brain().label} pulse={brain().pulse} />
      </span>
    </footer>
  );
}

export default StatusLine;
