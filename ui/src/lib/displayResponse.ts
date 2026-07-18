export type DisplayTone = "ok" | "warn" | "danger" | "neutral";
export type DisplayKind =
  | "table"
  | "card"
  | "list"
  | "action_result"
  | "blocker"
  | "execution_result"
  | "markdown"
  | "raw_preview";

export interface DisplayBadge {
  label: string;
  tone: DisplayTone;
}

export interface DisplayAction {
  label: string;
  prompt?: string;
  dashboard_target?: string;
  dashboardTarget?: string;
  danger?: boolean;
}

export interface DisplayColumn {
  key: string;
  label: string;
}

export interface DisplayRow {
  cells: string[];
}

export interface DisplayCard {
  title: string;
  subtitle?: string;
  badges?: DisplayBadge[];
}

export interface DisplaySection {
  title: string;
  body: string;
}

export interface DisplayResponse {
  version: number;
  kind: DisplayKind;
  title: string;
  summary: string;
  badges?: DisplayBadge[];
  columns?: DisplayColumn[];
  rows?: DisplayRow[];
  cards?: DisplayCard[];
  sections?: DisplaySection[];
  actions?: DisplayAction[];
  blockers?: string[];
  total_rows?: number;
  totalRows?: number;
  visible_rows?: number;
  visibleRows?: number;
  truncated?: boolean;
  raw_preview?: unknown;
  rawPreview?: unknown;
  provenance?: string;
  safety?: {
    redacted?: boolean;
    redacted_keys?: string[];
    redactedKeys?: string[];
    llm_summary_allowed?: boolean;
  };
}

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
  "base64",
  "binary",
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

function safePreview(value: unknown): unknown {
  if (Array.isArray(value)) return value.slice(0, 8).map(safePreview);
  if (!isRecord(value)) return value;
  const out: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value)) {
    const lower = key.toLowerCase();
    if (SENSITIVE_KEYS.some((sensitive) => lower.includes(sensitive))) {
      out[key] = "[redacted]";
      continue;
    }
    out[key] = safePreview(item);
  }
  return out;
}

export function normalizeDisplayResponse(input: unknown): DisplayResponse | null {
  if (!isRecord(input)) return null;
  const candidate = isRecord(input.display_response)
    ? input.display_response
    : isRecord(input.displayResponse)
      ? input.displayResponse
      : input;
  if (!isRecord(candidate)) return null;
  const kind = stringValue(candidate.kind) as DisplayKind;
  const title = stringValue(candidate.title);
  if (!kind || !title || !("version" in candidate)) return null;

  return {
    version: Number(candidate.version) || 1,
    kind,
    title,
    summary: stringValue(candidate.summary),
    badges: Array.isArray(candidate.badges) ? candidate.badges.filter(isRecord).map((badge) => ({
      label: stringValue(badge.label),
      tone: (stringValue(badge.tone) || "neutral") as DisplayTone,
    })).filter((badge) => badge.label) : [],
    columns: Array.isArray(candidate.columns) ? candidate.columns.filter(isRecord).map((column) => ({
      key: stringValue(column.key),
      label: stringValue(column.label),
    })).filter((column) => column.key && column.label) : [],
    rows: Array.isArray(candidate.rows) ? candidate.rows.filter(isRecord).map((row) => ({
      cells: Array.isArray(row.cells) ? row.cells.map(stringValue) : [],
    })).filter((row) => row.cells.length > 0) : [],
    cards: Array.isArray(candidate.cards) ? candidate.cards.filter(isRecord).map((card) => ({
      title: stringValue(card.title),
      subtitle: stringValue(card.subtitle) || undefined,
      badges: Array.isArray(card.badges) ? card.badges.filter(isRecord).map((badge) => ({
        label: stringValue(badge.label),
        tone: (stringValue(badge.tone) || "neutral") as DisplayTone,
      })) : [],
    })).filter((card) => card.title) : [],
    sections: Array.isArray(candidate.sections) ? candidate.sections.filter(isRecord).map((section) => ({
      title: stringValue(section.title),
      body: stringValue(section.body),
    })).filter((section) => section.title || section.body) : [],
    actions: Array.isArray(candidate.actions) ? candidate.actions.filter(isRecord).map((action) => ({
      label: stringValue(action.label),
      prompt: stringValue(action.prompt) || undefined,
      dashboard_target: stringValue(action.dashboard_target ?? action.dashboardTarget) || undefined,
      danger: Boolean(action.danger),
    })).filter((action) => action.label) : [],
    blockers: Array.isArray(candidate.blockers) ? candidate.blockers.map(stringValue).filter(Boolean) : [],
    total_rows: Number(candidate.total_rows ?? candidate.totalRows) || 0,
    visible_rows: Number(candidate.visible_rows ?? candidate.visibleRows) || 0,
    truncated: Boolean(candidate.truncated),
    raw_preview: safePreview(candidate.raw_preview ?? candidate.rawPreview),
    provenance: stringValue(candidate.provenance) || undefined,
    safety: isRecord(candidate.safety) ? {
      redacted: Boolean(candidate.safety.redacted),
      redacted_keys: Array.isArray(candidate.safety.redacted_keys)
        ? candidate.safety.redacted_keys.map(stringValue).filter(Boolean)
        : Array.isArray(candidate.safety.redactedKeys)
          ? candidate.safety.redactedKeys.map(stringValue).filter(Boolean)
          : [],
      llm_summary_allowed: Boolean(candidate.safety.llm_summary_allowed),
    } : undefined,
  };
}

export function displayResponseToText(response: DisplayResponse): string {
  const lines = [response.title, response.summary].filter(Boolean);
  for (const blocker of response.blockers ?? []) lines.push(`Blocker: ${blocker}`);
  const columns = response.columns ?? [];
  for (const [index, row] of (response.rows ?? []).slice(0, 8).entries()) {
    const listLine = displayListRowToText(response, row, index);
    if (listLine) {
      lines.push(listLine);
      continue;
    }
    if (columns.length > 0) {
      const pairs = row.cells
        .map((cell, cellIndex) => {
          const label = columns[cellIndex]?.label;
          return label ? `${label}: ${cell}` : cell;
        })
        .filter(Boolean);
      lines.push(`${index + 1}. ${pairs.join(" | ")}`);
    } else {
      lines.push(`${index + 1}. ${row.cells.filter(Boolean).join(" | ")}`);
    }
  }
  for (const section of response.sections ?? []) {
    const title = section.title?.trim();
    const body = section.body?.trim();
    if (title && body) lines.push(`${title}:\n${body}`);
    else if (body) lines.push(body);
  }
  if (response.truncated) {
    lines.push(`Showing ${response.visible_rows ?? response.rows?.length ?? 0} of ${response.total_rows ?? "many"} result(s).`);
  }
  return lines.join("\n");
}

function displayListRowToText(response: DisplayResponse, row: DisplayRow, index: number): string | null {
  if (response.kind !== "list") return null;
  const keys = (response.columns ?? []).map((column) => column.key);
  if (keys.length === 1 && (keys[0] === "display_name" || keys[0] === "name")) {
    const name = row.cells[0]?.trim();
    return name ? `${index + 1}. ${name}` : null;
  }
  if (
    keys.length === 2 &&
    keys[0] === "serial" &&
    (keys[1] === "display_name" || keys[1] === "name")
  ) {
    const serial = row.cells[0]?.trim() || String(index + 1);
    const name = row.cells[1]?.trim();
    return name ? `${serial}. ${name}` : null;
  }
  if (
    keys.length === 2 &&
    (keys[0] === "display_name" || keys[0] === "name") &&
    (keys[1] === "n8n_workflow_id" || keys[1] === "n8n_id")
  ) {
    const name = row.cells[0]?.trim();
    const n8nId = row.cells[1]?.trim();
    if (!name) return null;
    return n8nId ? `${index + 1}. ${name} | n8n ID: ${n8nId}` : `${index + 1}. ${name}`;
  }
  return null;
}
