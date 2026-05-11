import { Component, createSignal, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { SkillCapabilities } from "../types/openclaw";

interface PermissionModalProps {
  slug: string;
  name: string;
  capabilities: SkillCapabilities;
  onClose: () => void;
}

const PermissionModal: Component<PermissionModalProps> = (props) => {
  const [approved, setApproved] = createSignal(false);
  const [error, setError] = createSignal("");

  const capabilityList = (): string[] => {
    const caps: string[] = [];
    if (props.capabilities.network) caps.push("Network Access");
    if (props.capabilities.filesystem_read) caps.push("Filesystem Read");
    if (props.capabilities.filesystem_write) caps.push("Filesystem Write");
    if (props.capabilities.subprocess) caps.push("Subprocess Execution");
    if (props.capabilities.browser) caps.push("Browser Automation");
    if (props.capabilities.image_generation) caps.push("Image Generation");
    if (props.capabilities.media) caps.push("Media Processing");
    return caps;
  };

  const approveAndInstall = async () => {
    try {
      await invoke("clawhub_install_skill", {
        slug: props.slug,
        approvedCapabilities: props.capabilities,
      });
      setApproved(true);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div class="modal-overlay" onClick={() => props.onClose()}>
      <div class="modal permission-modal" onClick={(e) => e.stopPropagation()}>
        <h3>Permission Request — {props.name}</h3>
        <p>This skill requires the following capabilities:</p>
        <ul>
          <For each={capabilityList()}>{(cap) => <li>{cap}</li>}</For>
        </ul>
        <Show when={error()}>
          <p class="settings-error">{error()}</p>
        </Show>
        <Show when={approved()}>
          <p class="settings-success">Skill installed successfully!</p>
        </Show>
        <div class="modal-footer">
          <button class="btn-secondary" onClick={() => props.onClose()}>Reject</button>
          <button class="btn-primary" onClick={approveAndInstall} disabled={approved()}>Approve &amp; Install</button>
        </div>
      </div>
    </div>
  );
};

export default PermissionModal;
