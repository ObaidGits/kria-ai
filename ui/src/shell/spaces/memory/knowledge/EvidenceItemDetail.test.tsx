/**
 * Tests for EvidenceItemDetail component (task 4.4.2).
 *
 * Validates:
 * - Renders source
 * - Renders locator when non-null; hides when null
 * - Renders method
 * - Renders version
 * - Renders polarity with data-polarity attribute
 * - "support" polarity shows "Supports" text
 * - "contradict" polarity shows "Contradicts" text
 * - Renders score when non-null as "X/1.0" format; hides when null
 * - Renders semanticsLabel
 * - Renders policyLabel when non-null; hides when null
 *
 * Requirements: F4.4 (task 4.4.2)
 */
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@solidjs/testing-library";
import { EvidenceItemDetail } from "./EvidenceItemDetail";
import type { EvidenceItem } from "./Inspector";

afterEach(() => cleanup());

function makeItem(overrides: Partial<EvidenceItem> = {}): EvidenceItem {
  return {
    id: "ev-test",
    source: "test-source",
    locator: null,
    method: "extract",
    version: "1.0",
    polarity: "support",
    score: null,
    semanticsLabel: "test-semantics",
    policyLabel: null,
    ...overrides,
  };
}

function renderItem(overrides: Partial<EvidenceItem> = {}) {
  const item = makeItem(overrides);
  render(() => <EvidenceItemDetail item={item} />);
  return screen.getByTestId(`evidence-detail-${item.id}`);
}

describe("EvidenceItemDetail", () => {
  // ─── source ────────────────────────────────────────────────────────────────

  it("renders source", () => {
    const el = renderItem({ source: "my-corpus" });
    expect(el.querySelector("[data-field='source']")).toHaveTextContent("my-corpus");
  });

  // ─── locator ───────────────────────────────────────────────────────────────

  it("renders locator when non-null", () => {
    const el = renderItem({ locator: "https://example.com/doc" });
    expect(el.querySelector("[data-field='locator']")).toHaveTextContent("https://example.com/doc");
  });

  it("hides locator when null", () => {
    const el = renderItem({ locator: null });
    expect(el.querySelector("[data-field='locator']")).toBeNull();
  });

  // ─── method ────────────────────────────────────────────────────────────────

  it("renders method", () => {
    const el = renderItem({ method: "nlp-extract" });
    expect(el.querySelector("[data-field='method']")).toHaveTextContent("nlp-extract");
  });

  // ─── version ───────────────────────────────────────────────────────────────

  it("renders version", () => {
    const el = renderItem({ version: "2.3.1" });
    expect(el.querySelector("[data-field='version']")).toHaveTextContent("2.3.1");
  });

  // ─── polarity ──────────────────────────────────────────────────────────────

  it("renders polarity with data-polarity attribute for support", () => {
    const el = renderItem({ polarity: "support" });
    const polEl = el.querySelector("[data-field='polarity']");
    expect(polEl).not.toBeNull();
    expect(polEl).toHaveAttribute("data-polarity", "support");
  });

  it("renders polarity with data-polarity attribute for contradict", () => {
    const el = renderItem({ polarity: "contradict" });
    const polEl = el.querySelector("[data-field='polarity']");
    expect(polEl).not.toBeNull();
    expect(polEl).toHaveAttribute("data-polarity", "contradict");
  });

  it("support polarity shows 'Supports' text", () => {
    const el = renderItem({ polarity: "support" });
    expect(el.querySelector("[data-field='polarity']")).toHaveTextContent("Supports");
  });

  it("contradict polarity shows 'Contradicts' text", () => {
    const el = renderItem({ polarity: "contradict" });
    expect(el.querySelector("[data-field='polarity']")).toHaveTextContent("Contradicts");
  });

  // ─── score ─────────────────────────────────────────────────────────────────

  it("renders score when non-null as 'X/1.0' format", () => {
    const el = renderItem({ score: 0.87 });
    expect(el.querySelector("[data-field='score']")).toHaveTextContent("0.87/1.0");
  });

  it("hides score when null", () => {
    const el = renderItem({ score: null });
    expect(el.querySelector("[data-field='score']")).toBeNull();
  });

  it("renders score of 1 as '1/1.0'", () => {
    const el = renderItem({ score: 1 });
    expect(el.querySelector("[data-field='score']")).toHaveTextContent("1/1.0");
  });

  it("renders score of 0 as '0/1.0'", () => {
    const el = renderItem({ score: 0 });
    expect(el.querySelector("[data-field='score']")).toHaveTextContent("0/1.0");
  });

  // ─── semanticsLabel ────────────────────────────────────────────────────────

  it("renders semanticsLabel exactly from backend", () => {
    const el = renderItem({ semanticsLabel: "fact-assertion-v2" });
    expect(el.querySelector("[data-field='semantics-label']")).toHaveTextContent("fact-assertion-v2");
  });

  // ─── policyLabel ───────────────────────────────────────────────────────────

  it("renders policyLabel when non-null", () => {
    const el = renderItem({ policyLabel: "policy-XYZ" });
    expect(el.querySelector("[data-field='policy-label']")).toHaveTextContent("policy-XYZ");
  });

  it("hides policyLabel when null", () => {
    const el = renderItem({ policyLabel: null });
    expect(el.querySelector("[data-field='policy-label']")).toBeNull();
  });
});
