/**
 * CapabilityRow — a single tool/capability in the Tools segment (task 8.1,
 * Req 7.1 / 7.2). Selecting the row opens the capability's descriptor in the
 * single shared Inspector (Req 1.6 / 7.2) via `shellStore.openInspector`.
 *
 * Status/risk is shown as icon + text (never color alone — Req 17.3). The row
 * is a semantic button (kit `Row` with `onSelect`) → keyboard-operable +
 * focus-visible (Req 17.1).
 *
 * ── ARCHITECTURE INVARIANT ──────────────────────────────────────────────────
 * Presentation only. The row NEVER runs the capability (run→permission-gate is
 * task 8.2) — it only asks the shared Inspector to disclose the descriptor.
 * Capability name/description is UNTRUSTED and rendered as escaped text.
 *
 * Requirements: 7.1, 7.2, 17.1, 17.3
 */
import { createMemo, Show } from "solid-js";
import { Badge, Row } from "../../../kit";
import type { BadgeTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { shellStore } from "../../../stores";
import type { Capability, RiskLevel } from "../../../stores";

/** Risk → badge tone + label (icon + text, never color alone — Req 17.3). */
function riskPresentation(risk: RiskLevel): { tone: BadgeTone; label: string } {
  switch (risk) {
    case "green":
      return { tone: "success", label: "Low risk" };
    case "yellow":
      return { tone: "warning", label: "Elevated" };
    case "red":
      return { tone: "danger", label: "High risk" };
    case "black":
      return { tone: "danger", label: "Critical" };
    default:
      return { tone: "neutral", label: "Unknown" };
  }
}

export interface CapabilityRowProps {
  capability: Capability;
  /** Open the descriptor Inspector — defaults to `shellStore.openInspector`. */
  onInspect?: (capability: Capability) => void;
}

export function CapabilityRow(props: CapabilityRowProps) {
  const cap = () => props.capability;
  const risk = createMemo(() => riskPresentation(cap().riskLevel));

  function inspect() {
    if (props.onInspect) {
      props.onInspect(cap());
      return;
    }
    // Address the backing descriptor so the shared Inspector can fetch it.
    shellStore.openInspector("capability", cap().id, {
      providerId: cap().providerId ?? "",
      capabilityId: cap().capabilityId ?? "",
      name: cap().name,
    });
  }

  return (
    <li class="kria-capabilities__list-item">
      <Row
        onSelect={inspect}
        leading={<Icon name="zap" size={16} aria-hidden />}
        title={<span class="kria-caprow__name">{cap().name}</span>}
        subtitle={
          <Show when={cap().description}>
            <span class="kria-caprow__desc">{cap().description}</span>
          </Show>
        }
        trailing={
          <span class="kria-caprow__meta">
            <Show when={cap().elevated}>
              <Icon name="shield-alert" size={13} aria-hidden />
            </Show>
            <Badge tone={risk().tone}>{risk().label}</Badge>
            <Icon name="chevron-right" size={14} aria-hidden />
          </span>
        }
      />
    </li>
  );
}

export default CapabilityRow;
