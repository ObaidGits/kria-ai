/**
 * operationCopy — presentation copy for the shared operation vocabulary
 * (sub-task 12.6; UIE-M-013; Req 13.1, 13.2, 13.5; design §17).
 *
 * Pins that {@link describeOperation}:
 *   • NAMES the operation when a name is supplied, and stays truthfully
 *     objectless (never fabricates a name) when it is not (Req 13.1).
 *   • Surfaces a determinate percentage ONLY when the snapshot measured
 *     `progress`, never a fabricated one (UIE-M-013).
 *   • Presents cause + affected scope for failures and omits the cause when the
 *     source provides none (Req 13.2).
 *   • Returns `null` for `empty` so stale loading/error copy CLEARS (Req 13.5).
 *   • Emits `recovered` under a STABLE key so restoration announces once
 *     (Req 13.5).
 */
import { describe, it, expect } from "vitest";
import { describeOperation } from "./operationCopy";
import {
  deriveOperationSnapshot,
  OPERATION_STATES,
  type OperationSnapshot,
  type OperationState,
} from "./operationState";
import { t } from "./i18n";

/** Build a minimal snapshot for a target state (bypasses derivation ordering). */
function snap(
  state: OperationState,
  extra: Partial<OperationSnapshot> = {},
): OperationSnapshot {
  return { state, source: "test", ...extra };
}

describe("describeOperation — vocabulary coverage (Req 13.6)", () => {
  it("returns a non-empty copy line for every non-empty state, named and unnamed", () => {
    for (const state of OPERATION_STATES) {
      const named = describeOperation(snap(state), { operation: "Memory" });
      const unnamed = describeOperation(snap(state));
      if (state === "empty") {
        expect(named).toBeNull();
        expect(unnamed).toBeNull();
        continue;
      }
      expect(named?.text.length ?? 0).toBeGreaterThan(0);
      expect(unnamed?.text.length ?? 0).toBeGreaterThan(0);
      // A supplied name is actually surfaced (operation-specific, Req 13.1).
      expect(named!.text).toContain("Memory");
    }
  });
});

describe("describeOperation — empty clears stale copy (Req 13.5)", () => {
  it("returns null for empty regardless of a supplied name", () => {
    expect(describeOperation(snap("empty"))).toBeNull();
    expect(describeOperation(snap("empty"), { operation: "Settings" })).toBeNull();
  });
});

describe("describeOperation — loading names the operation (Req 13.1)", () => {
  it("names the operation without inventing progress", () => {
    const copy = describeOperation(snap("loading"), { operation: "Capabilities" });
    expect(copy!.key).toBe("operation_copy_loading_named");
    expect(copy!.text).toBe("Loading Capabilities…");
    expect(copy!.text).not.toMatch(/\d+%/); // no fabricated percentage
  });

  it("stays objectless but truthful when no name is supplied", () => {
    const copy = describeOperation(snap("loading"));
    expect(copy!.key).toBe("operation_copy_loading");
    expect(copy!.text).toBe(t("operation_copy_loading"));
  });
});

describe("describeOperation — determinate progress only when measured (UIE-M-013)", () => {
  it("shows a percentage when the snapshot carries measured progress", () => {
    // deriveOperationSnapshot only keeps progress on a progress-bearing state.
    const s = deriveOperationSnapshot({ source: "test", loading: true, progress: 0.42 });
    expect(s.progress).toBe(0.42);
    const copy = describeOperation(s, { operation: "Machines" });
    expect(copy!.key).toBe("operation_copy_loading_progress");
    expect(copy!.text).toBe("Loading Machines… 42%");
  });

  it("does not show a percentage when progress is absent", () => {
    const copy = describeOperation(snap("loading"), { operation: "Machines" });
    expect(copy!.text).not.toMatch(/%/);
  });

  it("rounds a measured fraction to a whole percent", () => {
    const s = deriveOperationSnapshot({ source: "test", loading: true, progress: 0.6666 });
    const copy = describeOperation(s);
    expect(copy!.text).toBe("Loading… 67%");
  });
});

describe("describeOperation — failure states cause + scope + recovery (Req 13.2)", () => {
  it("names the operation (affected scope) and the source-owned cause", () => {
    const copy = describeOperation(snap("failed", { message: "Model unreachable" }), {
      operation: "Converse",
    });
    expect(copy!.key).toBe("operation_copy_failed_message");
    expect(copy!.text).toBe("Converse failed: Model unreachable");
    expect(copy!.actionable).toBe(true); // recovery action available
  });

  it("omits the cause when the source provides none (no fabrication)", () => {
    const copy = describeOperation(snap("failed"), { operation: "Converse" });
    expect(copy!.key).toBe("operation_copy_failed_named");
    expect(copy!.text).not.toContain(":");
    expect(copy!.actionable).toBe(true);
  });

  it("surfaces the cause even without a scope name", () => {
    const copy = describeOperation(snap("failed", { message: "Timed out" }));
    expect(copy!.key).toBe("operation_copy_failed_message_unnamed");
    expect(copy!.text).toBe("Failed: Timed out");
  });
});

describe("describeOperation — blocked points at the approval owner (Req 13.2)", () => {
  it("is actionable and names the operation needing approval", () => {
    const copy = describeOperation(snap("blocked"), { operation: "Automations" });
    expect(copy!.text).toBe("Automations needs your approval to continue");
    expect(copy!.actionable).toBe(true);
  });
});

describe("describeOperation — optional service unavailable (Req 13.6)", () => {
  it("names the offline optional service and is actionable", () => {
    const copy = describeOperation(snap("optional-service-unavailable"), {
      operation: "n8n",
    });
    expect(copy!.key).toBe("operation_copy_unavailable_named");
    expect(copy!.text).toBe("n8n is unavailable");
    expect(copy!.actionable).toBe(true);
  });

  it("surfaces a source-owned reason when present", () => {
    const copy = describeOperation(
      snap("optional-service-unavailable", { message: "sidecar offline" }),
      { operation: "n8n" },
    );
    expect(copy!.key).toBe("operation_copy_unavailable_message");
    expect(copy!.text).toBe("n8n unavailable: sidecar offline");
  });
});

describe("describeOperation — recovery announces once with a stable key (Req 13.5)", () => {
  it("uses a stable recovered key so a live region de-duplicates by identity", () => {
    const a = describeOperation(snap("recovered"), { operation: "Memory" });
    const b = describeOperation(snap("recovered"), { operation: "Memory" });
    expect(a!.key).toBe("operation_copy_recovered_named");
    expect(a!.key).toBe(b!.key); // same identity → announced once, not per tick
    expect(a!.actionable).toBe(true);
  });

  it("models the full failed→retrying→recovered→empty lifecycle clearing stale copy", () => {
    // failed: names cause + scope, offers recovery.
    const failed = describeOperation(snap("failed", { message: "boom" }), { operation: "Memory" });
    expect(failed!.text).toBe("Memory failed: boom");

    // retrying: recovery attempt in progress.
    const retrying = describeOperation(snap("retrying"), { operation: "Memory" });
    expect(retrying!.key).toBe("operation_copy_retrying_named");
    expect(retrying!.text).toBe("Retrying Memory…");

    // recovered: restoration, announced once.
    const recovered = describeOperation(snap("recovered"), { operation: "Memory" });
    expect(recovered!.key).toBe("operation_copy_recovered_named");

    // empty: stale state cleared — no lingering running/failed copy.
    const cleared = describeOperation(snap("empty"), { operation: "Memory" });
    expect(cleared).toBeNull();
  });
});

describe("describeOperation — actionability (Req 13.2)", () => {
  it("marks attention + recovery states actionable and calm states not", () => {
    const actionable: OperationState[] = [
      "failed",
      "blocked",
      "waiting",
      "optional-service-unavailable",
      "retrying",
      "recovered",
    ];
    const calm: OperationState[] = ["loading", "active", "completed"];
    for (const s of actionable) {
      expect(describeOperation(snap(s))!.actionable).toBe(true);
    }
    for (const s of calm) {
      expect(describeOperation(snap(s))!.actionable).toBe(false);
    }
  });
});
