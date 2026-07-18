/**
 * EnrollWizard — the device enrollment wizard (task 9.1, Req 8.1). A guided
 * multi-step flow (Identity → Connection → Review) on the accessible kit Dialog
 * (Kobalte → focus trap + labelled title + Escape close, Req 17.6). The final
 * step dispatches enrollment through `onEnroll`, which the Space wires to the
 * EXISTING `register_new_target` command (dispatch-only — the runtime performs
 * the SSH bootstrap + host-key verification; this UI never touches the
 * substrate directly).
 *
 * HONEST STATES: validation, in-flight ("Enrolling…"), typed error, and success
 * are all shown (Req 20.4) — never a silent failure.
 *
 * SECURITY: enrollment never disables strict host-key checking; an optional
 * expected host-key pin is offered. All echoed error detail is escaped text.
 *
 * Requirements: 8.1, 17.6, 20.4
 */
import { Show, createMemo, createSignal } from "solid-js";
import { Dialog, Button, Input } from "../../../kit";
import { Icon } from "../../../components/Icon";
import "./machines.css";

export interface EnrollRequest {
  displayName: string;
  host: string;
  port: number;
  username: string;
  sshPrivateKeyPath?: string;
  expectedHostkeySha256?: string;
}

export type EnrollResult = { ok: true } | { ok: false; title: string; message: string; detail?: string };

export interface EnrollWizardProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Dispatch enrollment to the runtime (existing `register_new_target`). */
  onEnroll: (request: EnrollRequest) => Promise<EnrollResult>;
}

const STEPS = ["Identity", "Connection", "Review"] as const;

export function EnrollWizard(props: EnrollWizardProps) {
  return (
    <Dialog
      title="Enroll a device"
      open={props.open}
      onOpenChange={props.onOpenChange}
      description="KRIA verifies host identity and bootstraps SSH access. Strict host-key checking is never disabled."
    >
      {/* Body is remounted fresh each time the dialog opens (Kobalte only
          renders portal content while open) → no stale wizard state. Kept as a
          separate exported component so it is unit-testable without the modal. */}
      <EnrollWizardBody onEnroll={props.onEnroll} onClose={() => props.onOpenChange(false)} />
    </Dialog>
  );
}

export interface EnrollWizardBodyProps {
  onEnroll: (request: EnrollRequest) => Promise<EnrollResult>;
  onClose: () => void;
}

/** The wizard's steps/validation/submit UI, independent of the Dialog shell. */
export function EnrollWizardBody(props: EnrollWizardBodyProps) {
  const [step, setStep] = createSignal(0);
  const [displayName, setDisplayName] = createSignal("");
  const [host, setHost] = createSignal("");
  const [port, setPort] = createSignal("22");
  const [username, setUsername] = createSignal("");
  const [sshKeyPath, setSshKeyPath] = createSignal("~/.ssh/kria_id");
  const [expectedHostkey, setExpectedHostkey] = createSignal("");

  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal<{ title: string; message: string; detail?: string } | null>(null);
  const [done, setDone] = createSignal(false);

  const parsedPort = createMemo<number | null>(() => {
    const n = Number.parseInt(port().trim(), 10);
    if (!Number.isFinite(n) || n <= 0 || n > 65535) return null;
    return n;
  });

  const connectionValid = createMemo(
    () => host().trim().length > 0 && username().trim().length > 0 && parsedPort() !== null,
  );

  function close() {
    if (submitting()) return;
    props.onClose();
  }

  async function submit() {
    if (!connectionValid()) return;
    setSubmitting(true);
    setError(null);
    const result = await props.onEnroll({
      displayName: displayName().trim() || host().trim(),
      host: host().trim(),
      port: parsedPort() ?? 22,
      username: username().trim(),
      sshPrivateKeyPath: sshKeyPath().trim() || undefined,
      expectedHostkeySha256: expectedHostkey().trim() || undefined,
    });
    setSubmitting(false);
    if (result.ok) {
      setDone(true);
    } else {
      setError({ title: result.title, message: result.message, detail: result.detail });
    }
  }

  return (
      <div class="kria-enroll">
        {/* Step indicator (Req 17.2 — legible progress). */}
        <ol class="kria-enroll__steps" aria-label="Enrollment steps">
          {STEPS.map((label, index) => (
            <li
              class="kria-enroll__step"
              aria-current={step() === index ? "step" : undefined}
            >
              <Icon
                name={done() || step() > index ? "check-circle" : "circle"}
                size={13}
                aria-hidden
              />
              {label}
            </li>
          ))}
        </ol>

        <Show when={done()}>
          <p class="kria-enroll__ok" role="status">
            <Icon name="check-circle" size={14} aria-hidden /> Device enrolled. It will appear in the
            fleet matrix shortly.
          </p>
        </Show>

        <Show when={!done()}>
          {/* Step 0 — Identity */}
          <Show when={step() === 0}>
            <div class="kria-enroll__fields">
              <Input
                label="Display name"
                value={displayName()}
                onChange={setDisplayName}
                placeholder="e.g. Office Ubuntu VM"
              />
              <p class="kria-enroll__note">
                <Icon name="info" size={14} aria-hidden />
                A friendly name for this device. Defaults to the host if left blank.
              </p>
            </div>
          </Show>

          {/* Step 1 — Connection */}
          <Show when={step() === 1}>
            <div class="kria-enroll__fields">
              <Input
                label="Host"
                value={host()}
                onChange={setHost}
                placeholder="192.168.1.25 or vm.example.com"
                errorMessage={host().trim().length === 0 ? "Host is required." : undefined}
              />
              <Input
                label="Port"
                type="number"
                value={port()}
                onChange={setPort}
                placeholder="22"
                errorMessage={parsedPort() === null ? "Port must be 1–65535." : undefined}
              />
              <Input
                label="Username"
                value={username()}
                onChange={setUsername}
                placeholder="root"
                errorMessage={username().trim().length === 0 ? "Username is required." : undefined}
              />
              <Input
                label="SSH private key path"
                value={sshKeyPath()}
                onChange={setSshKeyPath}
                placeholder="~/.ssh/kria_id"
              />
              <Input
                label="Expected host key (SHA256, optional)"
                value={expectedHostkey()}
                onChange={setExpectedHostkey}
                placeholder="Optional safety pin for first enrollment"
              />
            </div>
          </Show>

          {/* Step 2 — Review */}
          <Show when={step() === 2}>
            <dl class="kria-enroll__review">
              <dt>Display name</dt>
              <dd>{displayName().trim() || host().trim() || "—"}</dd>
              <dt>Host</dt>
              <dd>{host().trim() || "—"}</dd>
              <dt>Port</dt>
              <dd>{parsedPort() ?? "—"}</dd>
              <dt>Username</dt>
              <dd>{username().trim() || "—"}</dd>
              <dt>SSH key</dt>
              <dd>{sshKeyPath().trim() || "auto-generated"}</dd>
              <dt>Host-key pin</dt>
              <dd>{expectedHostkey().trim() || "none (scanned on connect)"}</dd>
            </dl>
            <p class="kria-enroll__note">
              <Icon name="shield" size={14} aria-hidden />
              KRIA scans and fingerprints the SSH host key before bootstrap and fails closed on a
              host-key mismatch.
            </p>
          </Show>

          <Show when={error()}>
            {(err) => (
              <div class="kria-enroll__error" role="alert">
                <strong>
                  <Icon name="alert-triangle" size={13} aria-hidden /> {err().title}
                </strong>
                <span>{err().message}</span>
                <Show when={err().detail}>
                  <pre>{err().detail}</pre>
                </Show>
              </div>
            )}
          </Show>

          {/* Navigation */}
          <div class="kria-enroll__nav">
            <Button variant="ghost" onClick={close} disabled={submitting()}>
              Cancel
            </Button>
            <span class="kria-enroll__nav-spacer" />
            <Show when={step() > 0}>
              <Button
                variant="secondary"
                onClick={() => setStep((s) => Math.max(0, s - 1))}
                disabled={submitting()}
              >
                Back
              </Button>
            </Show>
            <Show when={step() < STEPS.length - 1}>
              <Button
                variant="primary"
                onClick={() => setStep((s) => Math.min(STEPS.length - 1, s + 1))}
                disabled={step() === 1 && !connectionValid()}
              >
                Next
              </Button>
            </Show>
            <Show when={step() === STEPS.length - 1}>
              <Button variant="primary" onClick={submit} disabled={submitting() || !connectionValid()}>
                {submitting() ? "Enrolling…" : "Enroll device"}
              </Button>
            </Show>
          </div>
        </Show>

        <Show when={done()}>
          <div class="kria-enroll__nav">
            <span class="kria-enroll__nav-spacer" />
            <Button variant="primary" onClick={close}>
              Done
            </Button>
          </div>
        </Show>
      </div>
  );
}

export default EnrollWizard;
