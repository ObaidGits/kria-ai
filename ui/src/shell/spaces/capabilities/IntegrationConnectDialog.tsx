/**
 * IntegrationConnectDialog — the connect form for integrations that need input
 * before connecting (task 8.2, Req 7.4): MCP servers (name + launch command)
 * and the Telegram bridge (bot token + allowed chat ids). Google / Colab need
 * no input and connect directly from the card.
 *
 * ── ARCHITECTURE INVARIANT ──────────────────────────────────────────────────
 * Performs NO connect itself — on submit it calls the injected handler, wired
 * to the dispatch-only capabilityActions bridge (`add_mcp_server` /
 * `update_telegram_config`). The bot token is a secret: it is bound to a
 * password field and NEVER echoed back or logged.
 *
 * Requirements: 7.4
 */
import { Show, createSignal } from "solid-js";
import { Button, Dialog, Input } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { IntegrationKind, CapabilityActionResult } from "../../../stores";
import { connectMcpServer, connectTelegram } from "../../../bridge";

export interface IntegrationConnectDialogProps {
  kind: Extract<IntegrationKind, "mcp" | "telegram">;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Test overrides (default to the capabilityActions dispatch bridge). */
  connectMcp?: typeof connectMcpServer;
  connectTelegramBridge?: typeof connectTelegram;
}

export function IntegrationConnectDialog(props: IntegrationConnectDialogProps) {
  const [name, setName] = createSignal("");
  const [command, setCommand] = createSignal("");
  const [args, setArgs] = createSignal("");
  const [botToken, setBotToken] = createSignal("");
  const [allowedChatIds, setAllowedChatIds] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const title = () => (props.kind === "mcp" ? "Connect an MCP server" : "Connect Telegram");

  const canSubmit = () =>
    props.kind === "mcp"
      ? name().trim().length > 0 && command().trim().length > 0
      : botToken().trim().length > 0;

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      let res: CapabilityActionResult;
      if (props.kind === "mcp") {
        const mcp = props.connectMcp ?? connectMcpServer;
        res = await mcp({
          name: name().trim(),
          command: command().trim(),
          args: args().trim() ? args().trim().split(/\s+/) : [],
        });
      } else {
        const tg = props.connectTelegramBridge ?? connectTelegram;
        res = await tg({
          enabled: true,
          botToken: botToken().trim(),
          allowedChatIds: allowedChatIds().trim(),
          autoStart: true,
        });
      }
      if (!res.ok) {
        setError(res.message);
        return;
      }
      props.onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      title={title()}
      open={props.open}
      onOpenChange={props.onOpenChange}
      footer={
        <>
          <Button variant="ghost" onClick={() => props.onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!canSubmit() || busy()} onClick={submit}>
            <Icon name="plus" size={15} aria-hidden={true} />
            {busy() ? "Connecting…" : "Connect"}
          </Button>
        </>
      }
    >
      <div class="kria-integration-connect" data-testid="integration-connect">
        <Show when={props.kind === "mcp"}>
          <Input label="Server name" value={name()} onChange={setName} />
          <Input
            label="Launch command"
            placeholder="e.g. npx @modelcontextprotocol/server-filesystem"
            value={command()}
            onChange={setCommand}
          />
          <Input
            label="Arguments (space-separated, optional)"
            value={args()}
            onChange={setArgs}
          />
        </Show>

        <Show when={props.kind === "telegram"}>
          <Input label="Bot token" type="password" value={botToken()} onChange={setBotToken} />
          <Input
            label="Allowed chat IDs (comma-separated, optional)"
            value={allowedChatIds()}
            onChange={setAllowedChatIds}
          />
        </Show>

        <Show when={error()}>
          <p class="kria-integration-connect__error" role="alert">
            <Icon name="alert-triangle" size={13} aria-hidden={true} /> {error()}
          </p>
        </Show>
      </div>
    </Dialog>
  );
}

export default IntegrationConnectDialog;
