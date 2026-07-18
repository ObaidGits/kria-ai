export type N8nTone = "ok" | "warn" | "danger" | "neutral";

export interface N8nUiBadge {
  label: string;
  tone: N8nTone;
}

export interface N8nUiAction {
  label: string;
  prompt?: string;
  dashboardTarget?: string;
  danger?: boolean;
}

export interface N8nUiResponse {
  kind: "service_outcome" | "inventory" | "execution_result" | "blocker";
  status: string;
  action?: string;
  title: string;
  summary: string;
  workflowId?: string;
  n8nWorkflowId?: string;
  badges: N8nUiBadge[];
  rows: Array<Record<string, string>>;
  totalRows?: number;
  visibleRows?: number;
  blockers: string[];
  nextActions: N8nUiAction[];
  rawPreview?: unknown;
}

const MAX_ROWS = 25;
const MAX_TEXT = 220;
const SENSITIVE_KEYS = [
  "credential",
  "secret",
  "token",
  "password",
  "api_key",
  "apikey",
  "headers",
  "authorization",
  "workflow_json",
  "nodes",
  "connections",
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function stringValue(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value.trim();
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

function compact(value: unknown, maxLength = MAX_TEXT): string {
  const text = stringValue(value).replace(/\s+/g, " ").trim();
  if (!text) return "";
  if (text.length <= maxLength) return text;
  return `${text.slice(0, Math.max(0, maxLength - 3)).trimEnd()}...`;
}

function titleCase(value: string): string {
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/\b\w/g, (match) => match.toUpperCase());
}

function toneForStatus(status: string): N8nTone {
  const value = status.toLowerCase();
  if (value.includes("danger") || value.includes("delete") || value.includes("blocked") || value.includes("error") || value.includes("failed")) {
    return "danger";
  }
  if (value.includes("warn") || value.includes("review") || value.includes("missing") || value.includes("required") || value.includes("offer")) {
    return "warn";
  }
  if (value.includes("ok") || value.includes("ready") || value.includes("created") || value.includes("approved") || value.includes("restored") || value.includes("archived") || value.includes("complete")) {
    return "ok";
  }
  return "neutral";
}

function rowTone(row: Record<string, string>): N8nTone {
  const status = `${row.status || ""} ${row.runnable || ""} ${row.risk || ""} ${row.blocker || ""}`.toLowerCase();
  if (status.includes("red") || status.includes("danger") || status.includes("blocked") || status.includes("deleted")) return "danger";
  if (status.includes("draft") || status.includes("review") || status.includes("missing") || status.includes("yellow") || status.includes("false") || status.includes("not executable")) return "warn";
  if (status.includes("approved") || status.includes("green") || status.includes("true") || status.includes("executable")) return "ok";
  return "neutral";
}

function normalizeActionLabel(value: unknown): string {
  const label = compact(value, 48);
  if (!label) return "";
  return titleCase(label);
}

function promptForAction(label: string, workflowId?: string): string | undefined {
  if (!workflowId) return undefined;
  const lower = label.toLowerCase();
  if (lower.includes("archive")) return `Archive workflow ${workflowId}`;
  if (lower.includes("restore")) return `Restore workflow ${workflowId}`;
  if (lower.includes("run")) return `Run workflow ${workflowId}`;
  if (lower.includes("test")) return `Test workflow ${workflowId}`;
  if (lower.includes("approve")) return `Approve workflow ${workflowId}`;
  if (lower.includes("review")) return `Review workflow ${workflowId}`;
  return undefined;
}

function safePreview(value: unknown): unknown {
  if (!isRecord(value)) return value;
  const preview: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value)) {
    const lower = key.toLowerCase();
    if (SENSITIVE_KEYS.some((sensitive) => lower.includes(sensitive))) continue;
    if (Array.isArray(item)) {
      preview[key] = item.slice(0, 5).map((row) => isRecord(row) ? safePreview(row) : compact(row, 120));
    } else if (isRecord(item)) {
      preview[key] = safePreview(item);
    } else {
      preview[key] = typeof item === "string" ? compact(item, 180) : item;
    }
  }
  return preview;
}

function normalizeRows(payload: unknown): Array<Record<string, string>> {
  const sourceRows = Array.isArray(payload)
    ? payload
    : isRecord(payload)
      ? (Array.isArray(payload.rows)
        ? payload.rows
        : Array.isArray(payload.items)
          ? payload.items
          : Array.isArray(payload.inventory)
            ? payload.inventory
            : Array.isArray(payload.workflows)
              ? payload.workflows
              : [])
      : [];

  return sourceRows
    .filter(isRecord)
    .slice(0, MAX_ROWS)
    .map((row) => {
      const normalized: Record<string, string> = {};
      const fields: Array<[string, unknown]> = [
        ["name", row.display_name ?? row.name ?? row.workflow_name ?? row.n8n_workflow_name],
        ["ref", row.message_ref ?? row.ref ?? row.id],
        ["from", row.from ?? row.sender],
        ["subject", row.subject ?? row.title],
        ["preview", row.preview ?? row.snippet ?? row.summary],
        ["workflow_id", row.workflow_id ?? row.kria_workflow_id ?? row.id],
        ["n8n_workflow_id", row.n8n_workflow_id ?? row.raw_n8n_workflow_id ?? row.n8n_id],
        ["status", row.status ?? row.lifecycle_status ?? row.routing_status],
        ["runnable", row.runnable ?? row.executable ?? row.can_run],
        ["risk", row.risk ?? row.risk_tier ?? row.risk_estimate],
        ["credential", row.credential_state ?? row.credential_status ?? row.credentials],
        ["blocker", row.blocker ?? row.short_blocker ?? row.reason],
      ];
      for (const [key, value] of fields) {
        const text = compact(value, key.includes("id") ? 64 : 90);
        if (text) normalized[key] = text;
      }
      return normalized;
    });
}

function isInventoryPayload(payload: Record<string, unknown>, n8n?: Record<string, unknown>): boolean {
  const action = stringValue(n8n?.action ?? payload.action ?? payload.status).toLowerCase();
  return (
    action.includes("list") ||
    action.includes("inventory") ||
    action.includes("find") ||
    Array.isArray(n8n?.rows) ||
    Array.isArray(n8n?.inventory) ||
    Array.isArray(payload.inventory) ||
    Array.isArray(payload.workflows)
  );
}

export function normalizeN8nResponse(input: unknown): N8nUiResponse | null {
  if (!isRecord(input)) return null;
  const payload = input;
  const n8n = isRecord(payload.n8n) ? payload.n8n : payload;
  if (!isRecord(n8n)) return null;
  const explicitN8nPayload =
    isRecord(payload.n8n) ||
    stringValue(payload.provider).toLowerCase() === "n8n" ||
    Boolean(payload.n8n_workflow_id) ||
    Boolean(payload.workflow_id && (payload.evidence || payload.success != null || payload.source === "n8n"));
  if (!explicitN8nPayload) return null;

  const action = compact(n8n.action ?? payload.action, 64);
  const status = compact(n8n.routing_status ?? n8n.status ?? payload.status, 64) || "unknown";
  const workflowId = compact(n8n.workflow_id ?? payload.workflow_id, 96) || undefined;
  const n8nWorkflowId = compact(n8n.n8n_workflow_id ?? payload.n8n_workflow_id, 96) || undefined;
  const displayName = compact(n8n.display_name ?? payload.display_name, 96);
  const card = isRecord(n8n.card) ? n8n.card : {};
  const result = isRecord(n8n.result) ? n8n.result : isRecord(payload.result) ? payload.result : {};
  const evidence = isRecord(payload.evidence) ? payload.evidence : undefined;
  const rows = normalizeRows(n8n.rows ?? n8n.inventory ?? result.rows ?? result.inventory ?? payload.inventory ?? payload.workflows ?? evidence?.messages);
  const inventory = isInventoryPayload(payload, n8n);
  const blockers = Array.isArray(n8n.blockers)
    ? n8n.blockers.map((item) => compact(item, 140)).filter(Boolean)
    : [];

  const nextActionValues = Array.isArray(n8n.next_actions) ? n8n.next_actions : [];
  const cardPrimary = normalizeActionLabel(card.primary_action);
  const cardSecondary = Array.isArray(card.secondary_actions) ? card.secondary_actions.map(normalizeActionLabel).filter(Boolean) : [];
  const nextActions = Array.from(new Set([
    cardPrimary,
    ...cardSecondary,
    ...nextActionValues.map(normalizeActionLabel),
  ].filter(Boolean))).slice(0, 5).map((label) => ({
    label,
    prompt: promptForAction(label, workflowId),
    dashboardTarget: label.toLowerCase().includes("dashboard") || label.toLowerCase().includes("danger") ? workflowId : undefined,
    danger: label.toLowerCase().includes("danger") || label.toLowerCase().includes("delete"),
  }));

  const title = compact(card.title, 80)
    || (inventory ? "n8n workflows" : displayName || workflowId || "n8n result");
  const summary = compact(
    card.subtitle
    ?? payload.reply
    ?? payload.message
    ?? result.message
    ?? evidence?.result
    ?? evidence?.message,
    260,
  ) || (inventory ? "Workflow inventory returned by KRIA." : "n8n action completed.");

  const badges: N8nUiBadge[] = [
    { label: titleCase(status), tone: toneForStatus(status) },
  ];
  if (action) badges.push({ label: titleCase(action), tone: "neutral" });
  if (workflowId) badges.push({ label: `KRIA ${workflowId}`, tone: "neutral" });
  if (n8nWorkflowId) badges.push({ label: `n8n ${n8nWorkflowId}`, tone: "neutral" });
  if (rows.length > 0) badges.push({ label: `${rows.length}${inventory ? "+" : ""} rows`, tone: "neutral" });

  let kind: N8nUiResponse["kind"] = inventory ? "inventory" : "service_outcome";
  if (blockers.length > 0 || status.toLowerCase().includes("blocked") || status.toLowerCase().includes("required")) {
    kind = "blocker";
  } else if (payload.success != null || evidence) {
    kind = "execution_result";
  }

  const totalRows = Number(n8n.total ?? result.total ?? payload.total ?? (Array.isArray(payload.workflows) ? payload.workflows.length : rows.length));

  return {
    kind,
    status,
    action: action || undefined,
    title,
    summary,
    workflowId,
    n8nWorkflowId,
    badges,
    rows,
    totalRows: Number.isFinite(totalRows) && totalRows > 0 ? totalRows : rows.length,
    visibleRows: rows.length,
    blockers,
    nextActions,
    rawPreview: safePreview(n8n.result_preview ?? n8n.result ?? payload.evidence ?? payload),
  };
}

export function n8nResponseToText(response: N8nUiResponse): string {
  const lines = [response.title, response.summary].filter(Boolean);
  if (response.workflowId) lines.push(`KRIA workflow ID: ${response.workflowId}`);
  if (response.n8nWorkflowId) lines.push(`n8n workflow ID: ${response.n8nWorkflowId}`);
  if (response.blockers.length) lines.push(`Blockers: ${response.blockers.join("; ")}`);
  if (response.rows.length) {
    lines.push(
      ...response.rows.slice(0, 8).map((row, index) => {
        const name = row.name || row.subject || row.ref || row.workflow_id || `Workflow ${index + 1}`;
        const ids = [row.workflow_id, row.n8n_workflow_id].filter(Boolean).join(" / ");
        const status = [row.status, row.runnable, row.risk].filter(Boolean).join(", ");
        const details = [
          row.ref && !ids.includes(row.ref) ? `ref ${row.ref}` : "",
          row.from ? `from ${row.from}` : "",
          row.preview,
        ].filter(Boolean).join(" - ");
        return `${index + 1}. ${name}${ids ? ` (${ids})` : ""}${status ? ` - ${status}` : ""}${details ? ` - ${details}` : ""}`;
      }),
    );
  }
  return lines.join("\n");
}

export function toneForN8nRow(row: Record<string, string>): N8nTone {
  return rowTone(row);
}
