/**
 * SkillCard — a ClawHub/OpenClaw skill in the Skills segment (task 8.1,
 * Req 7.1). Shows the skill's name, description, category, and trust tier
 * (icon + text, never color alone — Req 17.3) plus its installed/enabled state.
 *
 * SCOPE (task 8.2, Req 7.4): renders honest state AND exposes enable/disable +
 * uninstall for an installed skill, each a dispatch-only call to an EXISTING
 * backend command (`clawhub_toggle_skill` / `clawhub_uninstall_skill`) via the
 * injected handlers (defaulting to the capabilityActions bridge). The remote
 * install-with-trust-review flow lives in the Space (TrustReviewDialog). No
 * control silently does nothing (Req 10.6). Skill text is UNTRUSTED and
 * rendered as escaped text.
 *
 * Requirements: 7.1, 7.4, 17.3
 */
import { createMemo, createSignal, Show } from "solid-js";
import { Badge, Button, Card, StatusDot } from "../../../kit";
import type { BadgeTone, StatusTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { SkillView, CapabilityActionResult } from "../../../stores";
import {
  toggleSkill as toggleSkillAction,
  uninstallSkill as uninstallSkillAction,
} from "../../../bridge/capabilityActions";

/** Trust tier → badge tone + label (Req 7.2 trust surface, icon + text). */
function trustPresentation(tier: string): { tone: BadgeTone; label: string } {
  switch (tier.toLowerCase()) {
    case "verified":
      return { tone: "success", label: "Verified" };
    case "community":
      return { tone: "warning", label: "Community" };
    case "local":
    default:
      return { tone: "neutral", label: "Local" };
  }
}

function enabledPresentation(skill: SkillView): { tone: StatusTone; label: string } {
  if (!skill.installed) return { tone: "offline", label: "Not installed" };
  return skill.enabled
    ? { tone: "online", label: "Enabled" }
    : { tone: "info", label: "Disabled" };
}

export interface SkillCardProps {
  skill: SkillView;
  /** Enable/disable handler (defaults to the `clawhub_toggle_skill` bridge). */
  onToggle?: (slug: string, enabled: boolean) => Promise<CapabilityActionResult>;
  /** Uninstall handler (defaults to the `clawhub_uninstall_skill` bridge). */
  onUninstall?: (slug: string) => Promise<CapabilityActionResult>;
}

export function SkillCard(props: SkillCardProps) {
  const skill = () => props.skill;
  const trust = createMemo(() => trustPresentation(skill().trustTier));
  const state = createMemo(() => enabledPresentation(skill()));
  const [busy, setBusy] = createSignal(false);

  async function toggle() {
    setBusy(true);
    try {
      const handler = props.onToggle ?? toggleSkillAction;
      await handler(skill().slug, !skill().enabled);
    } finally {
      setBusy(false);
    }
  }

  async function uninstall() {
    setBusy(true);
    try {
      const handler = props.onUninstall ?? uninstallSkillAction;
      await handler(skill().slug);
    } finally {
      setBusy(false);
    }
  }

  return (
    <li>
      <Card class="kria-capcard" aria-label={skill().name}>
        <div class="kria-capcard__head">
          <span class="kria-capcard__name">
            <Icon name="sparkles" size={14} aria-hidden /> {skill().name}
          </span>
          <Badge tone={trust().tone}>{trust().label}</Badge>
        </div>

        <Show when={skill().description}>
          <p class="kria-capcard__desc">{skill().description}</p>
        </Show>

        <div class="kria-capcard__meta">
          <Badge tone="neutral">{skill().category}</Badge>
          <StatusDot tone={state().tone} label={state().label} />
          <span class="kria-capcard__status-label">{state().label}</span>
        </div>

        {/* Actions (Req 7.4) — only for installed skills; dispatch-only. */}
        <Show when={skill().installed}>
          <div class="kria-capcard__actions">
            <Button variant="secondary" size="sm" disabled={busy()} onClick={toggle}>
              <Icon name={skill().enabled ? "pause" : "play"} size={14} aria-hidden />
              {skill().enabled ? "Disable" : "Enable"}
            </Button>
            <Button variant="ghost" size="sm" disabled={busy()} onClick={uninstall}>
              <Icon name="trash-2" size={14} aria-hidden />
              Uninstall
            </Button>
          </div>
        </Show>
      </Card>
    </li>
  );
}

export default SkillCard;
