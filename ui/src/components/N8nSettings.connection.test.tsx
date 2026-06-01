import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

import N8nSettings from "./N8nSettings";

function runtimeStatus(overrides: Record<string, any> = {}) {
  return {
    enabled: true,
    mode: "external",
    base_url: "http://127.0.0.1:5678",
    dashboard_url: "http://127.0.0.1:5678",
    callback_url: "http://127.0.0.1:3001/api/n8n/callback",
    config: {
      enabled: true,
      mode: "external",
      base_url: "http://127.0.0.1:5678",
      dashboard_url: "http://127.0.0.1:5678",
      api_key_env: "KRIA_N8N_API_KEY",
      api_key_file: "~/.kria/secrets/n8n_api_key",
      signing_secret_env: "KRIA_N8N_SIGNING_SECRET",
      signing_secret_file: "~/.kria/secrets/n8n.key",
      callback_base_url: "",
      callback_path: "/api/n8n/callback",
      request_timeout_secs: 30,
      max_payload_bytes: 65536,
      auto_start: false,
      open_dashboard_on_start: false,
      open_dashboard_from_settings: true,
      healthcheck_timeout_secs: 5,
      healthcheck_interval_secs: 30,
      execution_poll_interval_secs: 5,
      event_stream_enabled: true,
      callback_freshness_window_secs: 300,
      future_callback_skew_secs: 30,
      default_requested_by: "local-user",
      managed_docker: {
        container_name: "kria-n8n",
        image: "n8nio/n8n:2.22.5",
        image_digest: "",
        bind_host: "127.0.0.1",
        host_port: 5678,
        container_port: 5678,
        data_dir: "~/.kria/n8n/docker",
        network: "bridge",
        restart_policy: "unless-stopped",
        pull_policy: "if_missing",
        host_gateway_name: "host.docker.internal",
        privileged: false,
        user: "",
        volume_mode: "rw",
        port_collision_policy: "fail_with_guidance",
        healthcheck_path: "/healthz",
        n8n_encryption_key_file: "~/.kria/secrets/n8n_encryption_key",
        dashboard_auth_required: true,
        basic_auth_user_env: "KRIA_N8N_BASIC_AUTH_USER",
        basic_auth_password_file: "~/.kria/secrets/n8n_basic_auth_password",
      },
      ...overrides.config,
    },
    secret_sources: {
      api_key: { source: "missing", present: false, file: "~/.kria/secrets/n8n_api_key" },
      signing_secret: { source: "file", present: true, file: "~/.kria/secrets/n8n.key" },
    },
    runtime: {
      container: { running: false, status: "not_managed" },
      last_connection: { status: "untested", message: "", checked_at_ms: 0 },
    },
    ...overrides,
  };
}

describe("N8nSettings connection wizard", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_n8n_runtime_status") return runtimeStatus();
      if (command === "detect_n8n_connection_candidates") {
        return {
          status: "ok",
          candidates: [
            {
              id: "local",
              label: "Local n8n on 127.0.0.1",
              connection_mode: "existing_local",
              base_url: "http://127.0.0.1:5678",
              dashboard_url: "http://127.0.0.1:5678",
              reachable: true,
              recommended: true,
            },
          ],
        };
      }
      if (command === "save_n8n_api_key_secret") {
        return { status: "saved", message: "Saved the API key securely." };
      }
      if (command === "test_n8n_connection_profile") {
        return {
          setup_status: "connected_monitor_only",
          api_auth_status: "ok",
          workflow_api_status: "working",
          runner_status: "monitor_only",
          connection_mode: "cloud_or_locked_down",
          base_url: "https://n8n.example.com",
          dashboard_url: "https://n8n.example.com",
          blockers: [],
          next_action: "This n8n can be used for workflow API, webhooks, broker, and monitoring.",
        };
      }
      return {};
    });
  });

  afterEach(() => cleanup());

  it("renders layman connection choices and hides advanced settings by default", async () => {
    render(() => <N8nSettings />);

    expect(await screen.findByText("Connection Wizard")).toBeInTheDocument();
    expect(screen.getByText("Use KRIA managed n8n")).toBeInTheDocument();
    expect(screen.getByText("Connect existing local n8n")).toBeInTheDocument();
    expect(screen.getByText("Advanced n8n settings")).toBeInTheDocument();
    expect(screen.getByText("Advanced n8n settings").closest("details")).not.toHaveAttribute("open");
  });

  it("saves pasted API key through the secret command", async () => {
    render(() => <N8nSettings />);
    const input = await screen.findByPlaceholderText("Paste n8n API key");

    fireEvent.input(input, { target: { value: "test-api-key" } });
    await waitFor(() => expect(screen.getByText("Save API key")).not.toBeDisabled());
    fireEvent.click(screen.getByText("Save API key"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("save_n8n_api_key_secret", {
        request: {
          apiKey: "test-api-key",
          apiKeyFile: "~/.kria/secrets/n8n_api_key",
        },
      });
    });
    expect(await screen.findByText("Saved the API key securely.")).toBeInTheDocument();
  });

  it("shows monitor-only as a usable connection state", async () => {
    render(() => <N8nSettings />);

    const testButtons = await screen.findAllByText("Test connection");
    await waitFor(() => expect(testButtons[0]).not.toBeDisabled());
    fireEvent.click(testButtons[0]);

    expect(await screen.findByText("connected_monitor_only")).toBeInTheDocument();
    expect(
      await screen.findByText("This n8n can be used for workflow API, webhooks, broker, and monitoring.")
    ).toBeInTheDocument();
  });
});
