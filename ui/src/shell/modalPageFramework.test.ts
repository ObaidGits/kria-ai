/**
 * Modal-vs-Page framework — structural invariant properties (task 9.1).
 *
 * design.md §10 (Modal vs Page Decision Framework) is a PERMANENT guideline.
 * The full written framework + the classification of every current
 * modal/page/overlay/inspector lives in
 * `.kiro/specs/homepage-presence-redesign/modal-vs-page-framework.md`.
 *
 * This file MACHINE-CHECKS the four structural invariants the framework
 * preserves (Requirements 18.1/18.2/18.3), exercising the REAL enforcement
 * points rather than re-deriving them:
 *
 *   • Property F1 — ≤1 modal & NO NESTING: for any sequence of open/close
 *     commands against the real `modalHost`, at most one modal is ever active,
 *     and opening a second while one is up is REFUSED (a modal can never spawn
 *     a modal). **Validates: Requirements 18.3**
 *   • Property F2 — SINGLE INSPECTOR: for any sequence of open/close commands
 *     against the real `shellStore` inspector, at most one target is ever set;
 *     opening a new target REPLACES the current one (never stacks).
 *     **Validates: Requirements 18.3**
 *   • Property F3 — SINGLE OVERLAY MANAGER: `overlayLayers.activeBlockingPriority`
 *     is the one authority for the top blocking layer; it is total and
 *     single-valued over every {modal-kind × approval-pending} combination and
 *     matches the documented precedence (approval-confirm > approval > modal >
 *     none). Because the single modal host holds ONE modal, an approval-confirm
 *     and a plain modal can never both block → no modal-on-modal across layers.
 *     **Validates: Requirements 18.1, 18.2, 18.3**
 *
 * Order-independence of the inertness partition across randomized concurrent
 * overlays is already proven in `task-8.10-generated-invariants.test.tsx`
 * (part c) and is referenced here, not duplicated.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fc from "fast-check";

import { modalHost, openModal, closeModal, isModalOpen } from "./modalHost";
import {
  OVERLAY_LAYER_PRIORITY,
  activeBlockingPriority,
  type OverlayLayer,
} from "./overlayLayers";
import { shellStore } from "../stores";
import { approvalStore } from "../stores";
import type { ApprovalRequest } from "../stores/approvalStore";

// ─── Helpers ─────────────────────────────────────────────────────────────────

function pendingRequest(id: string): ApprovalRequest {
  return {
    id,
    type: "tool-hitl",
    title: id,
    description: "",
    risk: "red",
    payload: null,
    createdAt: 1,
    status: "pending",
  };
}

/** Fully reset the three singleton authorities to a clean rest state. */
function resetSurfaces(): void {
  closeModal();
  shellStore.closeInspector();
  approvalStore.setQueue([]);
}

beforeEach(() => {
  // Silence the DEV "refused to open a second modal" warning that Property F1
  // intentionally provokes; it is expected behaviour, not a failure.
  vi.spyOn(console, "warn").mockImplementation(() => {});
  resetSurfaces();
});

afterEach(() => {
  resetSurfaces();
  vi.restoreAllMocks();
});

// ═══════════════════════════════════════════════════════════════════════════
// Property F1 — ≤1 modal & no nesting (single modal host)
// ═══════════════════════════════════════════════════════════════════════════

type ModalCmd = { op: "open"; id: string } | { op: "close" };

const arbModalCmd: fc.Arbitrary<ModalCmd> = fc.oneof(
  fc.record({ op: fc.constant("open" as const), id: fc.string({ minLength: 1, maxLength: 6 }) }),
  fc.record({ op: fc.constant("close" as const) }),
);

describe("Property F1 — ≤1 modal, never nested (Req 18.3)", () => {
  it("across any open/close sequence, at most one modal is active and a second open is refused", () => {
    fc.assert(
      fc.property(fc.array(arbModalCmd, { maxLength: 40 }), (cmds) => {
        resetSurfaces();
        let modelOpen = false;
        let activeId: string | null = null;

        for (const cmd of cmds) {
          if (cmd.op === "open") {
            const wasOpen = isModalOpen();
            const opened = openModal({ id: cmd.id, title: cmd.id, render: () => null });
            if (wasOpen) {
              // No modal-on-modal: the second open is refused, active unchanged.
              expect(opened).toBe(false);
              expect(modalHost.activeModal()?.id).toBe(activeId);
            } else {
              expect(opened).toBe(true);
              modelOpen = true;
              activeId = cmd.id;
            }
          } else {
            closeModal();
            modelOpen = false;
            activeId = null;
          }

          // Invariant after every step: the host holds either 0 or 1 modal.
          const active = modalHost.activeModal();
          expect(active === null || typeof active === "object").toBe(true);
          expect(isModalOpen()).toBe(modelOpen);
        }
      }),
    );
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// Property F2 — single shared Inspector (open replaces, never stacks)
// ═══════════════════════════════════════════════════════════════════════════

type InspectorCmd = { op: "open"; type: string; id: string } | { op: "close" };

const arbInspectorCmd: fc.Arbitrary<InspectorCmd> = fc.oneof(
  fc.record({
    op: fc.constant("open" as const),
    type: fc.constantFrom("memory", "capability", "automation", "device", "observatory"),
    id: fc.string({ minLength: 1, maxLength: 6 }),
  }),
  fc.record({ op: fc.constant("close" as const) }),
);

describe("Property F2 — single Inspector (Req 18.3)", () => {
  it("across any open/close sequence, at most one target is set and open replaces the prior", () => {
    fc.assert(
      fc.property(fc.array(arbInspectorCmd, { maxLength: 40 }), (cmds) => {
        resetSurfaces();
        let expected: { type: string; id: string } | null = null;

        for (const cmd of cmds) {
          if (cmd.op === "open") {
            shellStore.openInspector(cmd.type, cmd.id);
            expected = { type: cmd.type, id: cmd.id };
          } else {
            shellStore.closeInspector();
            expected = null;
          }

          const target = shellStore.inspectorTarget();
          if (expected === null) {
            expect(target).toBeNull();
          } else {
            // A single-valued signal: exactly the last-opened target, never a
            // stack of two. Opening a new subject REPLACED the previous one.
            expect(target).not.toBeNull();
            expect(target?.type).toBe(expected.type);
            expect(target?.id).toBe(expected.id);
          }
        }
      }),
    );
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// Property F3 — single overlay manager / one blocking authority
// ═══════════════════════════════════════════════════════════════════════════

type ModalKind = "none" | "modal" | "approval-confirm";
const arbModalKind = fc.constantFrom<ModalKind>("none", "modal", "approval-confirm");

describe("Property F3 — single overlay manager, documented precedence (Req 18.1/18.2/18.3)", () => {
  it("activeBlockingPriority is total & single-valued and matches §10.4 precedence", () => {
    fc.assert(
      fc.property(arbModalKind, fc.boolean(), (modalKind, approvalPending) => {
        resetSurfaces();

        if (modalKind === "modal") {
          openModal({ id: "user-modal", title: "user-modal", render: () => null });
        } else if (modalKind === "approval-confirm") {
          openModal({
            id: "approval-confirm-1",
            title: "confirm",
            render: () => null,
            layer: "approval-confirm",
          });
        }
        if (approvalPending) {
          approvalStore.setQueue([pendingRequest("req-1")]);
        }

        const top = activeBlockingPriority();

        // Documented precedence: approval-confirm > approval > modal > none.
        const expected =
          modalKind === "approval-confirm"
            ? OVERLAY_LAYER_PRIORITY["approval-confirm"]
            : approvalPending
              ? OVERLAY_LAYER_PRIORITY.approval
              : modalKind === "modal"
                ? OVERLAY_LAYER_PRIORITY.modal
                : 0;

        expect(top).toBe(expected);
        // Single-valued: the authority returns one number in the known set.
        const known: number[] = [0, ...Object.values(OVERLAY_LAYER_PRIORITY)];
        expect(known).toContain(top);
      }),
    );
  });

  it("the single modal host means only ONE modal-layer can block at a time (no modal-on-modal)", () => {
    resetSurfaces();
    // Open an approval-confirm modal, then attempt a plain modal on top.
    expect(
      openModal({ id: "confirm", title: "confirm", render: () => null, layer: "approval-confirm" }),
    ).toBe(true);
    expect(openModal({ id: "plain", title: "plain", render: () => null })).toBe(false);
    // Still exactly one modal active; the blocking authority reports one layer.
    expect(modalHost.activeModal()?.id).toBe("confirm");
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY["approval-confirm"]);
  });

  it("exposes exactly one blocking layer set (the sole overlay manager's z-tokens)", () => {
    // Structural: overlayLayers is the single manager; its priority table is the
    // one source of truth mapped 1:1 to the z-index tokens.
    const layers: OverlayLayer[] = [
      "shell",
      "inspector",
      "floating",
      "palette",
      "modal",
      "approval",
      "approval-confirm",
    ];
    expect(Object.keys(OVERLAY_LAYER_PRIORITY).sort()).toEqual([...layers].sort());
  });
});
