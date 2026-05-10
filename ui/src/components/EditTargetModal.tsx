import { Component, Show, createSignal, createMemo } from "solid-js";
import { DeviceTargetView } from "../hooks/useDeviceStatus";
import { IroncladEnrolledTargetSnapshot } from "../stores/app";

interface EditTargetModalProps {
  target: DeviceTargetView;
  enrolledTarget?: IroncladEnrolledTargetSnapshot | null;
  onClose: () => void;
  onUpdated: (request: UpdateTargetRequest) => Promise<void>;
}

interface UpdateTargetRequest {
  targetId: string;
  displayName?: string;
  host?: string;
  port?: number;
  username?: string;
  sshPrivateKeyPath?: string;
}

const EditTargetModal: Component<EditTargetModalProps> = (props) => {
  const enrolled = () => props.enrolledTarget;
  const [displayName, setDisplayName] = createSignal(props.target.displayName);
  const [host, setHost] = createSignal(enrolled()?.host ?? "");
  const [port, setPort] = createSignal(String(enrolled()?.port ?? 22));
  const [username, setUsername] = createSignal(enrolled()?.username ?? "");
  const [sshPrivateKeyPath, setSshPrivateKeyPath] = createSignal("");
  const [isSubmitting, setIsSubmitting] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal("");

  const parsedPort = createMemo<number | null>(() => {
    const numeric = Number.parseInt(port().trim(), 10);
    if (!Number.isFinite(numeric) || numeric <= 0 || numeric > 65535) return null;
    return numeric;
  });

  const isDirty = createMemo(() => {
    const enrolledData = enrolled();
    return (
      displayName().trim() !== props.target.displayName ||
      host().trim() !== (enrolledData?.host ?? "") ||
      parsedPort() !== (enrolledData?.port ?? 22) ||
      username().trim() !== (enrolledData?.username ?? "") ||
      sshPrivateKeyPath().trim() !== ""
    );
  });

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    if (!isDirty()) return;

    setIsSubmitting(true);
    setErrorMessage("");

    try {
      const request: UpdateTargetRequest = {
        targetId: props.target.targetId,
        displayName: displayName().trim(),
      };
      if (host().trim() !== (enrolled()?.host ?? "")) {
        request.host = host().trim();
      }
      if (parsedPort() !== null && parsedPort() !== (enrolled()?.port ?? 22)) {
        request.port = parsedPort()!;
      }
      if (username().trim() !== (enrolled()?.username ?? "")) {
        request.username = username().trim();
      }
      if (sshPrivateKeyPath().trim()) {
        request.sshPrivateKeyPath = sshPrivateKeyPath().trim();
      }
      await props.onUpdated(request);
    } catch (err: any) {
      setErrorMessage(err?.message ?? String(err));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div class="modal-overlay" onClick={props.onClose}>
      <div class="modal-content" onClick={(e) => e.stopPropagation()}>
        <div class="modal-header">
          <h2>Edit Target: {props.target.displayName}</h2>
          <button class="modal-close" onClick={props.onClose}>
            ✕
          </button>
        </div>

        <form class="modal-body" onSubmit={handleSubmit}>
          <div class="form-field">
            <label>Target ID</label>
            <input type="text" value={props.target.targetId} disabled />
          </div>

          <div class="form-field">
            <label>Display Name</label>
            <input
              type="text"
              value={displayName()}
              onInput={(e) => setDisplayName(e.currentTarget.value)}
              placeholder="e.g. My VM"
            />
          </div>

          <div class="form-field">
            <label>Host</label>
            <input
              type="text"
              value={host()}
              onInput={(e) => setHost(e.currentTarget.value)}
              placeholder="192.168.1.25 or vm.example.com"
              autocomplete="off"
            />
          </div>

          <div class="form-field">
            <label>Port</label>
            <input
              type="number"
              value={port()}
              onInput={(e) => setPort(e.currentTarget.value)}
              placeholder="22"
              min="1"
              max="65535"
            />
          </div>

          <div class="form-field">
            <label>Username</label>
            <input
              type="text"
              value={username()}
              onInput={(e) => setUsername(e.currentTarget.value)}
              placeholder="root"
              autocomplete="off"
            />
          </div>

          <div class="form-field">
            <label>SSH Private Key Path</label>
            <input
              type="text"
              value={sshPrivateKeyPath()}
              onInput={(e) => setSshPrivateKeyPath(e.currentTarget.value)}
              placeholder="Leave blank to keep current key"
              autocomplete="off"
            />
            <div class="field-hint">Only fill in to change the SSH key. Leave blank to keep the current key.</div>
          </div>

          <div class="form-field">
            <label>State</label>
            <input type="text" value={props.target.state} disabled />
          </div>

          <div class="form-field">
            <label>Health</label>
            <input
              type="text"
              value={`${Math.round((props.target.healthScore > 0 ? props.target.healthScore : 1) * 100)}%`}
              disabled
            />
          </div>

          <div class="form-field">
            <label>Latency</label>
            <input
              type="text"
              value={`${props.target.latencyEwmaMs.toFixed(1)}ms`}
              disabled
            />
          </div>

          <Show when={errorMessage()}>
            <div class="form-error">{errorMessage()}</div>
          </Show>

          <div class="modal-footer">
            <button type="button" class="btn-secondary" onClick={props.onClose}>
              Cancel
            </button>
            <button
              type="submit"
              class="btn-primary"
              disabled={isSubmitting() || !isDirty()}
            >
              {isSubmitting() ? "Saving…" : "Save Changes"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};

export default EditTargetModal;
