export const FEATURE_CONTROL_STATES = [
  "disabled",
  "starting",
  "running",
  "stopping",
  "error",
] as const;

export type FeatureControlState = (typeof FEATURE_CONTROL_STATES)[number];

export interface FeatureControl {
  id: string;
  label: string;
  description: string;
  desiredEnabled: boolean;
  state: FeatureControlState;
  detail?: string;
  error?: string;
}

export type FeatureControlsCollectionStatus =
  | "unavailable"
  | "empty"
  | "populated"
  | "partial";

export type FeatureControlsDiagnosticCode =
  | "missing-collection"
  | "null-collection"
  | "invalid-collection"
  | "invalid-entry";

export interface FeatureControlsDiagnostic {
  code: FeatureControlsDiagnosticCode;
  message: string;
  receivedType: string;
  index?: number;
  fields?: readonly string[];
}

export interface NormalizedFeatureControlsCollection {
  status: FeatureControlsCollectionStatus;
  controls: readonly FeatureControl[];
  diagnostics: readonly FeatureControlsDiagnostic[];
  rejectedCount: number;
}

const stateSet = new Set<string>(FEATURE_CONTROL_STATES);

function receivedType(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value;
}
function invalidFields(value: unknown): readonly string[] {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return ["entry"];
  }

  const entry = value as Record<string, unknown>;
  const invalid: string[] = [];
  if (typeof entry.id !== "string") invalid.push("id");
  if (typeof entry.label !== "string") invalid.push("label");
  if (typeof entry.description !== "string") invalid.push("description");
  if (typeof entry.desiredEnabled !== "boolean") invalid.push("desiredEnabled");
  if (typeof entry.state !== "string" || !stateSet.has(entry.state)) invalid.push("state");
  if (entry.detail !== undefined && typeof entry.detail !== "string") invalid.push("detail");
  if (entry.error !== undefined && typeof entry.error !== "string") invalid.push("error");
  return invalid;
}

export function normalizeFeatureControls(payload: unknown): NormalizedFeatureControlsCollection {
  if (payload === undefined) {
    return {
      status: "unavailable",
      controls: [],
      diagnostics: [{
        code: "missing-collection",
        message: "Feature-control collection is missing.",
        receivedType: "undefined",
      }],
      rejectedCount: 0,
    };
  }
  if (payload === null) {
    return {
      status: "unavailable",
      controls: [],
      diagnostics: [{
        code: "null-collection",
        message: "Feature-control collection is null.",
        receivedType: "null",
      }],
      rejectedCount: 0,
    };
  }
  if (!Array.isArray(payload)) {
    return {
      status: "unavailable",
      controls: [],
      diagnostics: [{
        code: "invalid-collection",
        message: "Feature-control payload must be an array.",
        receivedType: receivedType(payload),
      }],
      rejectedCount: 0,
    };
  }
  if (payload.length === 0) {
    return { status: "empty", controls: [], diagnostics: [], rejectedCount: 0 };
  }

  const controls: FeatureControl[] = [];
  const diagnostics: FeatureControlsDiagnostic[] = [];
  payload.forEach((entry, index) => {
    const fields = invalidFields(entry);
    if (fields.length === 0) {
      controls.push(entry as FeatureControl);
      return;
    }
    diagnostics.push({
      code: "invalid-entry",
      message: `Feature-control entry ${index} is invalid.`,
      receivedType: receivedType(entry),
      index,
      fields,
    });
  });

  return {
    status: controls.length === 0 ? "unavailable" : diagnostics.length > 0 ? "partial" : "populated",
    controls,
    diagnostics,
    rejectedCount: diagnostics.length,
  };
}
