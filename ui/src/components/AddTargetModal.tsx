import { Component, Show, createMemo, createSignal } from "solid-js";
import {
  appStore,
  RegisterNewTargetErrorCode,
  RegisterNewTargetErrorPayload,
  RegisterNewTargetResponse,
} from "../stores/app";

interface AddTargetModalProps {
  onClose: () => void;
  onRegistered?: (response: RegisterNewTargetResponse) => void;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function asErrorPayload(value: unknown): RegisterNewTargetErrorPayload | null {
  if (!isRecord(value)) {
    return null;
  }

  const codeRaw = value.code;
  const messageRaw = value.message;
  const detailRaw = value.detail;

  if (typeof codeRaw !== "string" || typeof messageRaw !== "string") {
    return null;
  }

  const knownCodes: RegisterNewTargetErrorCode[] = [
    "validation_failed",
    "connection_refused",
    "authentication_failed",
    "host_key_changed",
    "dependency_missing",
    "bootstrap_failed",
    "persistence_failed",
    "unknown",
  ];

  const normalizedCode = knownCodes.includes(codeRaw as RegisterNewTargetErrorCode)
    ? (codeRaw as RegisterNewTargetErrorCode)
    : "unknown";

  return {
    code: normalizedCode,
    message: messageRaw,
    detail: typeof detailRaw === "string" ? detailRaw : undefined,
  };
}

function toErrorPayload(error: unknown): RegisterNewTargetErrorPayload {
  const fromObject = asErrorPayload(error);
  if (fromObject) {
    return fromObject;
  }

  if (error instanceof Error) {
    return {
      code: "unknown",
      message: error.message,
    };
  }

  if (typeof error === "string") {
    const raw = error.trim();

    try {
      const parsed = JSON.parse(raw) as unknown;
      const fromJson = asErrorPayload(parsed);
      if (fromJson) {
        return fromJson;
      }
    } catch {
      // String error payload is not JSON; continue with heuristics.
    }

    const lower = raw.toLowerCase();
    if (lower.includes("host key") || lower.includes("identification has changed")) {
      return {
        code: "host_key_changed",
        message: raw,
      };
    }
    if (
      lower.includes("permission denied") ||
      lower.includes("authentication failed") ||
      lower.includes("publickey")
    ) {
      return {
        code: "authentication_failed",
        message: raw,
      };
    }
    if (
      lower.includes("connection refused") ||
      lower.includes("timed out") ||
      lower.includes("no route") ||
      lower.includes("resolve hostname")
    ) {
      return {
        code: "connection_refused",
        message: raw,
      };
    }

    return {
      code: "unknown",
      message: raw,
    };
  }

  return {
    code: "unknown",
    message: "Unexpected enrollment error",
  };
}

function presentError(payload: RegisterNewTargetErrorPayload): {
  title: string;
  message: string;
  detail?: string;
} {
  switch (payload.code) {
    case "connection_refused":
      return {
        title: "Connection Refused",
        message:
          "Could not reach the VM on the provided host/port. Verify network route, firewall rules, and SSH service status.",
        detail: payload.detail ?? payload.message,
      };
    case "authentication_failed":
      return {
        title: "Authentication Failed",
        message:
          "SSH authentication failed. Verify the username and private key, then ensure the key is allowed by remote SSH policy.",
        detail: payload.detail ?? payload.message,
      };
    case "host_key_changed":
      return {
        title: "Host Key Changed",
        message:
          "The observed SSH host key does not match the expected/enrolled key. Re-verify identity out-of-band before re-enrolling.",
        detail: payload.detail ?? payload.message,
      };
    case "dependency_missing":
      return {
        title: "Missing SSH Dependency",
        message:
          "OpenSSH client tools are required for enrollment. Install ssh, ssh-keyscan, and ssh-keygen on this machine.",
        detail: payload.detail ?? payload.message,
      };
    case "validation_failed":
      return {
        title: "Invalid Input",
        message: payload.message,
        detail: payload.detail,
      };
    case "bootstrap_failed":
      return {
        title: "Bootstrap Failed",
        message:
          "Connected to the host but failed while updating remote access state. Check remote shell policy and ~/.ssh permissions.",
        detail: payload.detail ?? payload.message,
      };
    case "persistence_failed":
      return {
        title: "Persistence Failed",
        message:
          "Enrollment completed partially but local registry persistence failed. Retry after checking local filesystem permissions.",
        detail: payload.detail ?? payload.message,
      };
    default:
      return {
        title: "Enrollment Error",
        message: payload.message || "Unexpected error during target enrollment.",
        detail: payload.detail,
      };
  }
}

const AddTargetModal: Component<AddTargetModalProps> = (props) => {
  const [displayName, setDisplayName] = createSignal("");
  const [host, setHost] = createSignal("");
  const [port, setPort] = createSignal("22");
  const [username, setUsername] = createSignal("");
  const [sshPrivateKeyPath, setSshPrivateKeyPath] = createSignal("~/.ssh/kria_id");
  const [expectedHostkeySha256, setExpectedHostkeySha256] = createSignal("");

  const [isSubmitting, setIsSubmitting] = createSignal(false);
  const [errorTitle, setErrorTitle] = createSignal("");
  const [errorMessage, setErrorMessage] = createSignal("");
  const [errorDetail, setErrorDetail] = createSignal<string | undefined>(undefined);

  const parsedPort = createMemo<number | null>(() => {
    const numeric = Number.parseInt(port().trim(), 10);
    if (!Number.isFinite(numeric)) {
      return null;
    }
    if (numeric <= 0 || numeric > 65535) {
      return null;
    }
    return numeric;
  });

  const validationError = createMemo<string | null>(() => {
    if (!host().trim()) {
      return "Host is required.";
    }
    if (!username().trim()) {
      return "Username is required.";
    }
    if (parsedPort() === null) {
      return "Port must be a number between 1 and 65535.";
    }
    return null;
  });

  const canSubmit = createMemo(() => !isSubmitting() && validationError() === null);

  const closeIfAllowed = () => {
    if (isSubmitting()) {
      return;
    }
    props.onClose();
  };

  const submit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (!canSubmit()) {
      setErrorTitle("Invalid Input");
      setErrorMessage(validationError() ?? "Please fix the highlighted fields.");
      setErrorDetail(undefined);
      return;
    }

    setIsSubmitting(true);
    setErrorTitle("");
    setErrorMessage("");
    setErrorDetail(undefined);

    try {
      const response = await appStore.registerNewTarget({
        displayName: displayName().trim() || host().trim(),
        host: host().trim(),
        port: parsedPort() ?? 22,
        username: username().trim(),
        sshPrivateKeyPath: sshPrivateKeyPath().trim() || undefined,
        expectedHostkeySha256: expectedHostkeySha256().trim() || undefined,
      });

      props.onRegistered?.(response);
      props.onClose();
    } catch (error) {
      const normalized = toErrorPayload(error);
      const presented = presentError(normalized);
      setErrorTitle(presented.title);
      setErrorMessage(presented.message);
      setErrorDetail(presented.detail);
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div class="modal-overlay add-target-overlay" onClick={closeIfAllowed}>
      <div class="modal add-target-modal" onClick={(event) => event.stopPropagation()}>
        <form onSubmit={submit}>
          <div class="modal-header add-target-header">
            <h2>Add New Device</h2>
            <button
              type="button"
              class="close-btn"
              onClick={closeIfAllowed}
              disabled={isSubmitting()}
              aria-label="Close add target modal"
            >
              ×
            </button>
          </div>

          <div class="modal-body add-target-body">
            <p class="add-target-description">
              Enroll a VM via SSH bootstrap. KRIA verifies host identity, validates authentication,
              and appends the local KRIA public key to remote authorized_keys.
            </p>

            <div class="add-target-grid">
              <label class="settings-field">
                <span>Display Name</span>
                <input
                  type="text"
                  value={displayName()}
                  onInput={(event) => setDisplayName(event.currentTarget.value)}
                  placeholder="e.g. Office Ubuntu VM"
                  autocomplete="off"
                  disabled={isSubmitting()}
                />
              </label>

              <label class="settings-field">
                <span>Host</span>
                <input
                  type="text"
                  value={host()}
                  onInput={(event) => setHost(event.currentTarget.value)}
                  placeholder="192.168.1.25 or vm.example.com"
                  autocomplete="off"
                  disabled={isSubmitting()}
                />
              </label>

              <label class="settings-field">
                <span>Port</span>
                <input
                  type="number"
                  value={port()}
                  onInput={(event) => setPort(event.currentTarget.value)}
                  placeholder="22"
                  min="1"
                  max="65535"
                  disabled={isSubmitting()}
                />
              </label>

              <label class="settings-field">
                <span>Username</span>
                <input
                  type="text"
                  value={username()}
                  onInput={(event) => setUsername(event.currentTarget.value)}
                  placeholder="root"
                  autocomplete="off"
                  disabled={isSubmitting()}
                />
              </label>

              <label class="settings-field add-target-grid-span-2">
                <span>SSH Private Key Path</span>
                <input
                  type="text"
                  value={sshPrivateKeyPath()}
                  onInput={(event) => setSshPrivateKeyPath(event.currentTarget.value)}
                  placeholder="~/.ssh/kria_id"
                  autocomplete="off"
                  disabled={isSubmitting()}
                />
                <div class="field-hint">
                  If this key does not exist, KRIA generates a new local ed25519 keypair automatically.
                </div>
              </label>

              <label class="settings-field add-target-grid-span-2">
                <span>Expected Host Key (SHA256, optional)</span>
                <input
                  type="text"
                  value={expectedHostkeySha256()}
                  onInput={(event) => setExpectedHostkeySha256(event.currentTarget.value)}
                  placeholder="Optional safety pin for first enrollment"
                  autocomplete="off"
                  disabled={isSubmitting()}
                />
              </label>
            </div>

            <div class="add-target-security-note">
              <strong>Security checks:</strong>
              <ul>
                <li>SSH host key is scanned and fingerprinted before bootstrap.</li>
                <li>Known host mismatch fails closed with Host Key Changed error.</li>
                <li>Enrollment never disables strict host key checks.</li>
              </ul>
            </div>

            <Show when={validationError()}>
              <div class="add-target-feedback warning">{validationError()}</div>
            </Show>

            <Show when={errorMessage()}>
              <div class="add-target-feedback error">
                <strong>{errorTitle()}</strong>
                <div>{errorMessage()}</div>
                <Show when={errorDetail()}>
                  <pre>{errorDetail()}</pre>
                </Show>
              </div>
            </Show>
          </div>

          <div class="modal-footer add-target-footer">
            <button
              type="button"
              class="btn-secondary"
              onClick={closeIfAllowed}
              disabled={isSubmitting()}
            >
              Cancel
            </button>
            <button type="submit" class="btn-primary" disabled={!canSubmit()}>
              {isSubmitting() ? "Enrolling..." : "Enroll Device"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};

export default AddTargetModal;
