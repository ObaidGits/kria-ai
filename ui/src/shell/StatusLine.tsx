/**
 * StatusLine — the ONE status line of the shell (Req 1.1). Surfaces the Core
 * state, the active Space, and a pending-approval count. The Core state sits in
 * a polite live region so assistive tech announces changes (Req 17.2 live
 * regions) without stealing focus.
 *
 * Per Req 9.2 the Observatory + Core + this status line are the only places
 * telemetry surfaces — this line stays intentionally minimal.
 *
 * Requirements: 1.1, 17.2
 */
import { Show } from "solid-js";
import { coreStore, shellStore, approvalStore } from "../stores";
import { StatusDot } from "../kit";
import type { StatusTone } from "../kit";
import { SPACE_META } from "./spaces";
import "./AppShell.css";

/** Map a Core state to a status-dot tone (label always accompanies — Req 17.3). */
function coreTone(): StatusTone {
  if (coreStore.state() === "error") return "error";
  if (coreStore.needsAttention()) return "busy";
  if (coreStore.isActive()) return "info";
  return "online";
}

export function StatusLine() {
  return (
    <footer class="kria-statusline" role="contentinfo">
      <span class="kria-statusline__group" aria-live="polite">
        <StatusDot tone={coreTone()} label={coreStore.state()} pulse={coreStore.isActive()} />
      </span>
      <span class="kria-statusline__group">
        <span class="kria-statusline__space">
          {SPACE_META[shellStore.activeSpace()].label}
        </span>
      </span>
      <Show when={approvalStore.hasPending()}>
        <span class="kria-statusline__group kria-statusline__approvals">
          {approvalStore.pendingCount()} pending approval
          {approvalStore.pendingCount() === 1 ? "" : "s"}
        </span>
      </Show>
    </footer>
  );
}

export default StatusLine;
