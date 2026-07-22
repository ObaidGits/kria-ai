/**
 * Task 8.10 — GENERATED (property-style) invariants for constrained-width
 * controls + Overlay/VoiceSurface coordination (IU-09; UIE-H-007,
 * UIE-M-002/003/004).
 *
 * These are the GENERATED companions to the deterministic matrices already
 * proven by:
 *   • controlPriority + OverflowControl (task 8.5/8.6 partition invariants),
 *   • overlayLayers.test.ts + overlayInterruption.test.tsx (task 8.2/8.9
 *     deterministic Overlay authority matrix),
 *   • Composer.test.tsx (task 3.4 deterministic grow-then-scroll examples).
 *
 * `fast-check` is NOT a dependency of ui/. Like every other matrix task in this
 * spec, generation uses a deterministic seeded PRNG (mulberry32) so the
 * "generated" cases are FULLY REPRODUCIBLE and never flake. Each `describe`
 * documents the seed it uses.
 *
 * Part a — label/draft:   partition never clips/drops/duplicates a control for
 *                         arbitrary labels; Composer rows clamp to [1,8];
 *                         draft text survives profile/mode changes.
 * Part b — mode × profile: every WindowMode × WidthProfile (× randomized
 *                         relevance/state/order) keeps the toolbar/Composer
 *                         invariants — criticals never overflow, each action is
 *                         inline XOR overflow (no duplication, nothing dropped),
 *                         context-rail toggle reachable, Mini keeps every
 *                         control reachable directly or in the disclosure.
 * Part c — collision:     randomized concurrent overlay states, resolved through
 *                         the REAL overlayLayers contract, keep a pending
 *                         approval the sole blocking interrupt over the
 *                         non-blocking set; the nested approval-confirm always
 *                         outranks + inerts the Approval Center; a plain modal
 *                         never outranks a pending approval; and the outcome is
 *                         INVARIANT under registration/DOM order (PRNG-shuffled).
 *
 * Validates: Requirements 11.1, 11.2, 11.8, 11.9, 11.13, 10.1, 10.2, 10.3, 16.3, 16.4, 16.5
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, cleanup, screen } from "@solidjs/testing-library";

import Composer from "./spaces/converse/Composer";
import { converseStore, coreStore, approvalStore, shellStore } from "../stores";
import {
  CONVERSE_CONTROLS,
  CRITICAL_CONTROL_IDS,
  partitionControls,
  controlTier,
  type TieredControl,
  type CriticalityTier,
} from "./controlPriority";
import {
  resolveConverseComposition,
  widthProfileFor,
  type WidthProfile,
  type LaneRelevance,
} from "./spaces/converseComposition";
import {
  OVERLAY_LAYER_PRIORITY,
  activeBlockingPriority,
  initOverlayInertness,
  registerOverlaySurface,
  type OverlayLayer,
} from "./overlayLayers";
import { openModal, closeModal, type ModalDescriptor } from "./modalHost";
import type { WindowMode } from "../stores/shellStore";
import type { ApprovalRequest } from "../stores/approvalStore";

/** Deterministic, reproducible PRNG so the "generated" cases never flake. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Fisher–Yates shuffle driven by a supplied PRNG (order-independence proofs). */
function shuffle<T>(items: readonly T[], rand: () => number): T[] {
  const out = [...items];
  for (let i = out.length - 1; i > 0; i -= 1) {
    const j = Math.floor(rand() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

const ALL_PROFILES: readonly WidthProfile[] = ["focus", "dual", "assisted", "full"];
const ALL_MODES: readonly WindowMode[] = ["standard", "mini", "immersive"];

// ─────────────────────────────────────────────────────────────────────────────
// Part a — label/draft
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Arbitrary control-label strings spanning the classes the task calls out:
 * short, long, localization-expanded, RTL-ish, whitespace, and very-long single
 * word. Labels are presentation only — the partition must never clip/drop/
 * duplicate a control regardless of what its label contains.
 */
const LABEL_CLASSES: readonly string[] = [
  "OK",
  "Send message",
  "Nachrichtenübermittlungseinstellungen", // localization-expanded (compound)
  "إرسال الرسالة الآن", // RTL-ish (Arabic)
  "   ", // whitespace only
  "\t\n  \n", // mixed whitespace
  "Supercalifragilisticexpialidociousantidisestablishmentarianism", // very long single word
  "", // empty
  "启动语音输入并保存草稿", // CJK expansion
];

describe("Task 8.10a — partition never clips/drops/duplicates a control for arbitrary labels (seed 0x8a10a)", () => {
  // **Validates: Requirements 11.1, 11.2, 16.3**
  it("PROPERTY: for arbitrary control sets, labels, and capacities the partition is a lossless, duplicate-free split that never overflows a critical", () => {
    const rand = mulberry32(0x8a10a);
    const tiers: readonly CriticalityTier[] = ["critical", "primary", "secondary"];

    for (let iter = 0; iter < 400; iter += 1) {
      // Build an arbitrary control set: always include ≥1 of each critical id
      // subset, plus a random spread of primary/secondary controls, each with a
      // randomly-classed label. Ids are unique per case.
      const size = 1 + Math.floor(rand() * 12);
      const controls: TieredControl[] = [];
      for (let n = 0; n < size; n += 1) {
        const tier = tiers[Math.floor(rand() * tiers.length)];
        const label = LABEL_CLASSES[Math.floor(rand() * LABEL_CLASSES.length)];
        controls.push({ id: `c${iter}-${n}`, tier, label });
      }
      // Capacity spans below-zero-effective (0) through more-than-enough.
      const maxInline = Math.floor(rand() * (controls.length + 3)) - 1;

      const { inline, overflow } = partitionControls(controls, maxInline);
      const inlineIds = inline.map((c) => c.id);
      const overflowIds = overflow.map((c) => c.id);
      const allOut = new Set([...inlineIds, ...overflowIds]);

      // (1) Lossless: every input control lands in exactly one partition.
      expect(inlineIds.length + overflowIds.length).toBe(controls.length);
      expect(allOut.size).toBe(controls.length);
      for (const c of controls) expect(allOut.has(c.id)).toBe(true);

      // (2) Duplicate-free: no id appears in both partitions.
      for (const id of inlineIds) expect(overflowIds).not.toContain(id);

      // (3) Criticals NEVER overflow — even at maxInline ≤ 0.
      for (const c of controls) {
        if (c.tier === "critical") {
          expect(inlineIds, `critical ${c.id} stays inline (cap=${maxInline})`).toContain(c.id);
        }
      }

      // (4) Secondary overflows before any primary: if any primary is in
      //     overflow, then every secondary is too (design §29 ordering).
      const primaryOverflowed = overflow.some((c) => c.tier === "primary");
      if (primaryOverflowed) {
        for (const c of controls) {
          if (c.tier === "secondary") expect(overflowIds).toContain(c.id);
        }
      }

      // (5) Original relative order preserved within each partition.
      const orderPreserved = (ids: string[]) => {
        const idx = ids.map((id) => controls.findIndex((c) => c.id === id));
        return idx.every((v, i) => i === 0 || idx[i - 1] < v);
      };
      expect(orderPreserved(inlineIds)).toBe(true);
      expect(orderPreserved(overflowIds)).toBe(true);
    }
  });
});

describe("Task 8.10a — Composer rows stay clamped to [1,8] for arbitrary draft text (seed 0xd7a47)", () => {
  // **Validates: Requirements 11.1, 16.4** (Req 4.4 grow-then-scroll bound)
  beforeEach(() => {
    cleanup();
    converseStore.setActiveThread(null);
    converseStore.updateDraft({ text: "", attachments: [], mode: "assistant" });
    converseStore.clearMessages();
    coreStore.reset();
  });
  afterEach(cleanup);

  /** Deterministic arbitrary draft text across the documented classes. */
  function generateDraft(rand: () => number): string {
    const kind = Math.floor(rand() * 5);
    switch (kind) {
      case 0:
        return ""; // empty → 1 row
      case 1:
        return "one short line";
      case 2: {
        // multiline up to / over the MAX_ROWS=8 cap
        const lines = 1 + Math.floor(rand() * 20);
        return Array.from({ length: lines }, (_, i) => `line ${i}`).join("\n");
      }
      case 3: {
        // very long single word (no newlines → 1 logical line)
        return "x".repeat(200 + Math.floor(rand() * 2000));
      }
      default: {
        // long wrapped text with a handful of newlines
        const lines = 1 + Math.floor(rand() * 12);
        return Array.from(
          { length: lines },
          () => "the quick brown fox ".repeat(1 + Math.floor(rand() * 8)),
        ).join("\n");
      }
    }
  }

  it("PROPERTY: textarea rows == clamp(lineCount, 1, 8) for every generated draft — grows then bounds, never clips input", () => {
    const rand = mulberry32(0xd7a47);
    render(() => <Composer />);
    const textarea = screen.getByLabelText("Message KRIA") as HTMLTextAreaElement;

    for (let iter = 0; iter < 250; iter += 1) {
      const text = generateDraft(rand);
      converseStore.updateDraft({ text });

      const lines = text.length === 0 ? 1 : text.split("\n").length;
      const expected = Math.min(Math.max(1, lines), 8);

      expect(textarea.rows, `rows clamped for ${lines}-line draft`).toBe(expected);
      expect(textarea.rows).toBeGreaterThanOrEqual(1);
      expect(textarea.rows).toBeLessThanOrEqual(8);
      // The full text is retained (past the cap it scrolls internally, never drops).
      expect(textarea.value).toBe(text);
    }
  });
});

describe("Task 8.10a — draft text is never mutated/reset by profile/mode changes (seed 0x0dea7)", () => {
  // **Validates: Requirements 11.1, 11.4, 16.4**
  beforeEach(() => {
    cleanup();
    converseStore.setActiveThread(null);
    converseStore.updateDraft({ text: "", attachments: [], mode: "assistant" });
    coreStore.reset();
    shellStore.setWindowMode("standard");
  });
  afterEach(() => {
    cleanup();
    shellStore.setWindowMode("standard");
  });

  it("PROPERTY: arbitrary staged draft survives every WidthProfile × WindowMode change (replace/re-render never resets it)", () => {
    const rand = mulberry32(0x0dea7);
    const texts = [
      "half-written thought",
      "line1\nline2\nline3",
      "استمرار المسودة", // RTL
      "x".repeat(500),
      "  leading and trailing whitespace  ",
    ];

    for (const seedText of texts) {
      for (let iter = 0; iter < 6; iter += 1) {
        cleanup();
        converseStore.updateDraft({ text: seedText, attachments: [] });

        // Re-render the Composer at a random profile prop and flip the window
        // mode across all values — neither is allowed to touch the draft.
        const profile = ALL_PROFILES[Math.floor(rand() * ALL_PROFILES.length)];
        render(() => <Composer widthProfile={profile} />);
        const textarea = screen.getByLabelText("Message KRIA") as HTMLTextAreaElement;
        expect(textarea.value).toBe(seedText);

        for (const mode of shuffle(ALL_MODES, rand)) {
          shellStore.setWindowMode(mode);
          expect(converseStore.composerDraft().text, `draft preserved in ${mode}/${profile}`).toBe(seedText);
          expect(textarea.value).toBe(seedText);
        }
        // Attachments (other draft fields) preserved too.
        expect(converseStore.composerDraft().attachments).toEqual([]);
      }
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Part b — mode × profile
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Documented per-profile inline capacities (design §11.5/§20.3, from
 * ConverseSpace.TOOLBAR_ACTION_CAPACITY and Composer.COMPOSER_TOOLS_CAPACITY).
 * Mini reduces the toolbar to its disclosure, so its effective inline
 * capacity is the floor — every non-critical control is reached via the
 * labelled overflow. The partition invariants hold at ALL of these capacities.
 */
const TOOLBAR_CAPACITY: Readonly<Record<WidthProfile, number>> = {
  focus: 1,
  dual: 1,
  assisted: 4,
  full: 4,
};
const COMPOSER_CAPACITY: Readonly<Record<WidthProfile, number>> = {
  focus: 1,
  dual: 3,
  assisted: 3,
  full: 3,
};

/** Toolbar controls actually adapted by ConverseSpace (task 8.6). */
const TOOLBAR_CONTROLS: readonly TieredControl[] = CONVERSE_CONTROLS.filter((c) =>
  ["context-rail-toggle", "export", "detach", "open-sidebar"].includes(c.id),
);
/** Composer primary tools actually adapted by Composer (task 8.6). */
const COMPOSER_TOOL_IDS = ["mode-chip", "attach", "voice"] as const;

describe("Task 8.10b — every WindowMode × WidthProfile keeps toolbar/Composer invariants (seed 0x903f11e)", () => {
  // **Validates: Requirements 10.1, 10.2, 10.3, 11.1, 11.2, 16.3, 16.4, 16.5**
  it("PROPERTY: across all 12 mode×profile combos (× randomized relevance/order) composition is deterministic and no critical overflows / no action duplicates", () => {
    const rand = mulberry32(0x903f11e);

    for (const mode of ALL_MODES) {
      for (const profile of ALL_PROFILES) {
        // Mini collapses the toolbar into the disclosure → treat its
        // effective inline capacity as the floor (0). Standard/Immersive use the
        // profile capacity. This exercises the widest capacity spread per combo.
        const toolbarCap = mode === "mini" ? 0 : TOOLBAR_CAPACITY[profile];
        const composerCap = mode === "mini" ? 1 : COMPOSER_CAPACITY[profile];

        for (let iter = 0; iter < 12; iter += 1) {
          // Randomized lane relevance + concurrent state.
          const relevance: LaneRelevance = {
            threads: rand() >= 0.5,
            work: rand() >= 0.5,
            context: rand() >= 0.5,
          };

          // Lane composition is deterministic and bounded by profile capacity.
          const composition = resolveConverseComposition(mode, profile, relevance);
          const again = resolveConverseComposition(mode, profile, relevance);
          expect(composition.id, "composition is deterministic").toBe(again.id);
          const capacity = { focus: 0, dual: 1, assisted: 2, full: 3 }[profile];
          expect(composition.visibleLanes.length, "visible lanes bounded by capacity").toBeLessThanOrEqual(capacity);
          for (const lane of composition.visibleLanes) {
            expect(relevance[lane], "only relevant lanes are visible").toBe(true);
          }

          // ── Toolbar partition (randomized order in — order out preserved). ──
          const toolbarIn = shuffle(TOOLBAR_CONTROLS, rand);
          const tb = partitionControls(toolbarIn, toolbarCap);
          assertLosslessNoDup(tb.inline, tb.overflow, toolbarIn.length);
          assertNoCriticalOverflow(tb.overflow);
          // context-rail toggle is always reachable (inline XOR overflow).
          const railInline = tb.inline.some((c) => c.id === "context-rail-toggle");
          const railOverflow = tb.overflow.some((c) => c.id === "context-rail-toggle");
          expect(railInline !== railOverflow, "context-rail toggle reachable exactly once").toBe(true);

          // ── Composer tool partition. ──
          const composerTools = shuffle(
            COMPOSER_TOOL_IDS.map((id) => ({ id, tier: controlTier(id)!, label: id })),
            rand,
          );
          const cp = partitionControls(composerTools, composerCap);
          assertLosslessNoDup(cp.inline, cp.overflow, composerTools.length);
          assertNoCriticalOverflow(cp.overflow);

          // ── Mini reachability (UIE-M-004): nothing is HIDDEN — every
          //    control is reachable directly or through the disclosure. ──
          if (mode === "mini") {
            const reachable = new Set([
              ...tb.inline.map((c) => c.id),
              ...tb.overflow.map((c) => c.id),
              ...cp.inline.map((c) => c.id),
              ...cp.overflow.map((c) => c.id),
            ]);
            for (const c of [...TOOLBAR_CONTROLS, ...composerTools]) {
              expect(reachable.has(c.id), `Mini keeps ${c.id} reachable`).toBe(true);
            }
          }
        }
      }
    }
  });

  it("PROPERTY: the full canonical control map keeps every CRITICAL affordance inline at every profile capacity", () => {
    const rand = mulberry32(0x2c0de);
    for (let iter = 0; iter < 200; iter += 1) {
      const maxInline = Math.floor(rand() * (CONVERSE_CONTROLS.length + 2));
      const { inline, overflow } = partitionControls(shuffle(CONVERSE_CONTROLS, rand), maxInline);
      assertLosslessNoDup(inline, overflow, CONVERSE_CONTROLS.length);
      const inlineIds = new Set(inline.map((c) => c.id));
      for (const id of CRITICAL_CONTROL_IDS) {
        expect(inlineIds.has(id), `critical ${id} inline at cap=${maxInline}`).toBe(true);
      }
    }
  });
});

function assertLosslessNoDup(
  inline: readonly TieredControl[],
  overflow: readonly TieredControl[],
  total: number,
): void {
  const ids = [...inline.map((c) => c.id), ...overflow.map((c) => c.id)];
  expect(ids.length).toBe(total);
  expect(new Set(ids).size).toBe(total); // no duplication
}

function assertNoCriticalOverflow(overflow: readonly TieredControl[]): void {
  for (const c of overflow) {
    expect(c.tier, `${c.id} in overflow must not be critical`).not.toBe("critical");
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Part c — collision (real overlayLayers contract)
// ─────────────────────────────────────────────────────────────────────────────

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

function makeRequest(): ApprovalRequest {
  return {
    id: "req-collision",
    type: "tool-hitl",
    title: "Send the drafted email",
    description: "why",
    risk: "yellow",
    effects: ["Sends 1 email"],
    payload: {},
    createdAt: Date.now(),
    status: "pending",
  };
}

function makeModal(id: string, layer?: ModalDescriptor["layer"]): ModalDescriptor {
  return { id, title: id, layer, render: () => null };
}

function isInert(node: HTMLElement): boolean {
  return node.hasAttribute("inert") && node.getAttribute("aria-hidden") === "true";
}

type OverlayScenario = {
  approvalPending: boolean;
  nestedConfirm: boolean;
  paletteOpen: boolean;
  notificationOpen: boolean;
  voiceActive: boolean;
  modalOpen: boolean;
  inspectorOpen: boolean;
};

describe("Task 8.10c — randomized overlay collisions resolve through the real overlayLayers contract (seed 0xc0111de)", () => {
  // **Validates: Requirements 11.8, 11.9, 11.13**
  let dispose: (() => void) | undefined;
  const cleanups: Array<() => void> = [];

  beforeEach(() => {
    approvalStore.setQueue([]);
    closeModal();
    dispose = initOverlayInertness();
  });
  afterEach(() => {
    cleanups.splice(0).forEach((c) => c());
    dispose?.();
    approvalStore.setQueue([]);
    closeModal();
    document.body.innerHTML = "";
  });

  function register(layer: OverlayLayer): HTMLElement {
    const node = document.createElement("div");
    node.dataset.layer = layer;
    document.body.appendChild(node);
    cleanups.push(registerOverlaySurface(node, layer));
    return node;
  }

  /** Expected top blocking priority per the §20.3 authority contract. */
  function expectedTop(s: OverlayScenario): number {
    if (s.nestedConfirm) return OVERLAY_LAYER_PRIORITY["approval-confirm"];
    if (s.approvalPending) return OVERLAY_LAYER_PRIORITY.approval;
    if (s.modalOpen) return OVERLAY_LAYER_PRIORITY.modal;
    return 0;
  }

  it("PROPERTY: approval is the sole blocking interrupt, nested confirm outranks it, a modal never outranks it, and the result is invariant under registration order", async () => {
    const rand = mulberry32(0xc0111de);

    for (let iter = 0; iter < 96; iter += 1) {
      // nestedConfirm implies an approval is pending (it confirms one).
      const approvalPending = rand() >= 0.4;
      const scenario: OverlayScenario = {
        approvalPending,
        nestedConfirm: approvalPending && rand() >= 0.5,
        paletteOpen: rand() >= 0.5,
        notificationOpen: rand() >= 0.5,
        voiceActive: rand() >= 0.5,
        // A plain modal and the nested confirm are mutually exclusive at the
        // one-modal-at-a-time host; prefer the confirm when both are drawn.
        modalOpen: rand() >= 0.5,
        inspectorOpen: rand() >= 0.5,
      };

      // Reset per case.
      approvalStore.setQueue([]);
      closeModal();
      cleanups.splice(0).forEach((c) => c());
      document.body.innerHTML = "";

      // Build the set of surfaces to register (with their layers), then register
      // them in a PRNG-SHUFFLED order to prove order-independence.
      const surfaces: Array<{ key: string; layer: OverlayLayer }> = [];
      if (scenario.inspectorOpen) surfaces.push({ key: "inspector", layer: "inspector" });
      if (scenario.paletteOpen) surfaces.push({ key: "palette", layer: "palette" });
      if (scenario.notificationOpen) surfaces.push({ key: "notification", layer: "floating" });
      if (scenario.voiceActive) surfaces.push({ key: "voice", layer: "floating" });
      if (scenario.approvalPending) surfaces.push({ key: "approval", layer: "approval" });
      if (scenario.nestedConfirm) surfaces.push({ key: "confirm", layer: "approval-confirm" });

      const nodes = new Map<string, HTMLElement>();
      for (const s of shuffle(surfaces, rand)) {
        nodes.set(s.key, register(s.layer));
      }

      // Drive the authoritative stores.
      if (scenario.approvalPending) approvalStore.setQueue([makeRequest()]);
      if (scenario.nestedConfirm) {
        openModal(makeModal("approval-confirm-req-collision", "approval-confirm"));
      } else if (scenario.modalOpen) {
        openModal(makeModal("user-modal"));
      }
      await tick();

      const top = activeBlockingPriority();
      const expected = expectedTop(scenario);
      expect(top, `case ${iter}: top blocking priority ${JSON.stringify(scenario)}`).toBe(expected);

      // A plain modal NEVER outranks a pending approval.
      if (scenario.approvalPending && scenario.modalOpen && !scenario.nestedConfirm) {
        expect(top).toBe(OVERLAY_LAYER_PRIORITY.approval);
        expect(top).toBeGreaterThan(OVERLAY_LAYER_PRIORITY.modal);
      }

      // Nested confirm always outranks the Approval Center and inerts it.
      if (scenario.nestedConfirm) {
        expect(top).toBe(OVERLAY_LAYER_PRIORITY["approval-confirm"]);
        expect(top).toBeGreaterThan(OVERLAY_LAYER_PRIORITY.approval);
        expect(isInert(nodes.get("approval")!), "confirm inerts the Approval Center").toBe(true);
        expect(isInert(nodes.get("confirm")!), "confirm stays interactive").toBe(false);
      }

      // A pending approval (no confirm) is the sole blocking interrupt: every
      // registered surface strictly below it is inert; approval itself is not.
      if (scenario.approvalPending && !scenario.nestedConfirm) {
        for (const key of ["palette", "notification", "voice", "inspector"]) {
          const node = nodes.get(key);
          if (node) expect(isInert(node), `${key} inert under pending approval`).toBe(true);
        }
        expect(isInert(nodes.get("approval")!), "approval itself never inerted").toBe(false);
      }

      // When nothing blocks, no surface is inert.
      if (expected === 0) {
        for (const node of nodes.values()) expect(isInert(node)).toBe(false);
      }
    }
  });

  it("PROPERTY: the SAME collision yields the SAME inert set regardless of registration order (order-independence)", async () => {
    const rand = mulberry32(0x5caf01d);
    // A representative loaded collision: approval pending over palette + notif +
    // voice + inspector, with the nested confirm on top.
    const layers: OverlayLayer[] = [
      "inspector",
      "palette",
      "floating",
      "floating",
      "approval",
      "approval-confirm",
    ];

    function inertSignatureAfterShuffle(): Promise<string> {
      return (async () => {
        approvalStore.setQueue([]);
        closeModal();
        cleanups.splice(0).forEach((c) => c());
        document.body.innerHTML = "";

        const shuffled = shuffle(layers, rand);
        const nodes = shuffled.map((layer) => register(layer));
        approvalStore.setQueue([makeRequest()]);
        openModal(makeModal("approval-confirm-req-collision", "approval-confirm"));
        await tick();
        // Signature keyed by layer priority → inert (independent of insert order).
        return nodes
          .map((n) => `${n.dataset.layer}:${isInert(n) ? 1 : 0}`)
          .sort()
          .join("|");
      })();
    }

    const first = await inertSignatureAfterShuffle();
    for (let i = 0; i < 8; i += 1) {
      expect(await inertSignatureAfterShuffle(), "inert set invariant under registration order").toBe(first);
    }
    // Sanity: everything below approval-confirm is inert; the confirm surface is not.
    expect(first).toContain("approval-confirm:0");
    expect(first).toContain("approval:1");
    expect(first).toContain("palette:1");
  });
});
