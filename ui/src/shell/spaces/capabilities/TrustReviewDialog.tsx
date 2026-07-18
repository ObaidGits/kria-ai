/**
 * TrustReviewDialog — the trust-review step before installing a remote skill
 * (task 8.2, Req 7.4). A remote ClawHub skill is ALWAYS Community trust tier;
 * this dialog states that plainly (icon + text, never color alone — Req 17.3),
 * lists the capabilities the skill requests, and requires a DELIBERATE confirm
 * before `installSkill` is dispatched.
 *
 * ── ARCHITECTURE INVARIANT ──────────────────────────────────────────────────
 * This dialog performs NO install itself — on confirm it calls the injected
 * `onInstall` (wired to the `installSkill` dispatch bridge). The runtime forces
 * the real trust tier + verifies the bundle regardless of what is shown here.
 * Skill text is UNTRUSTED and rendered as escaped text.
 *
 * Requirements: 7.4, 17.3
 */
import { For, Show, createSignal } from "solid-js";
import { Badge, Button, Dialog } from "../../../kit";
import type { BadgeTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { RemoteSkillView } from "../../../stores";

/** Trust tier → badge tone + label (icon + text, never color alone). */
function trustPresentation(tier: string): { tone: BadgeTone; label: string; icon: string } {
  switch (tier.toLowerCase()) {
    case "verified":
      return { tone: "success", label: "Verified", icon: "shield" };
    case "community":
      return { tone: "warning", label: "Community", icon: "shield-alert" };
    case "local":
    default:
      return { tone: "neutral", label: "Local", icon: "shield" };
  }
}

export interface TrustReviewDialogProps {
  skill: RemoteSkillView;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /**
   * Install the skill with the reviewed + approved capabilities. Wired to the
   * `installSkill` dispatch bridge by the Space. Should resolve when done.
   */
  onInstall: (approvedCapabilities: string[]) => void | Promise<void>;
}

export function TrustReviewDialog(props: TrustReviewDialogProps) {
  const skill = () => props.skill;
  const trust = () => trustPresentation(skill().trustTier);
  const [installing, setInstalling] = createSignal(false);

  async function confirmInstall() {
    setInstalling(true);
    try {
      await props.onInstall(skill().capabilities);
      props.onOpenChange(false);
    } finally {
      setInstalling(false);
    }
  }

  return (
    <Dialog
      title="Review before installing"
      open={props.open}
      onOpenChange={props.onOpenChange}
      description={
        <span>
          Installing <strong>{skill().name}</strong> from ClawHub. Review its trust tier and the
          capabilities it requests before you allow it.
        </span>
      }
      footer={
        <>
          <Button variant="ghost" onClick={() => props.onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="primary" disabled={installing()} onClick={confirmInstall}>
            <Icon name="download" size={15} aria-hidden={true} />
            {installing() ? "Installing…" : "Install"}
          </Button>
        </>
      }
    >
      <div class="kria-trust-review" data-testid="trust-review">
        {/* Trust tier — icon + text (Req 17.3). */}
        <section class="kria-trust-review__section" aria-label="Trust tier">
          <span class="kria-trust-review__label">Trust tier</span>
          <div class="kria-trust-review__tier">
            <Badge tone={trust().tone}>
              <Icon name={trust().icon} size={12} aria-hidden={true} /> {trust().label}
            </Badge>
            <span class="kria-trust-review__note">
              Remote skills are never Verified — KRIA runs them as Community with capability
              enforcement.
            </span>
          </div>
        </section>

        {/* Requested capabilities. */}
        <section class="kria-trust-review__section" aria-label="Requested capabilities">
          <span class="kria-trust-review__label">Requested capabilities</span>
          <Show
            when={skill().capabilities.length > 0}
            fallback={<p class="kria-trust-review__empty">No special capabilities requested.</p>}
          >
            <ul class="kria-trust-review__caps">
              <For each={skill().capabilities}>
                {(cap) => (
                  <li>
                    <Icon name="lock" size={12} aria-hidden={true} /> {cap}
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>

        <Show when={skill().description}>
          <p class="kria-trust-review__desc">{skill().description}</p>
        </Show>
      </div>
    </Dialog>
  );
}

export default TrustReviewDialog;
