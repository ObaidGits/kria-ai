/**
 * Tests for RelationItemDetail component (task 4.4.2).
 *
 * Validates:
 * - Renders direction with data-direction attribute
 * - "outgoing"  direction shows "→" arrow
 * - "incoming"  direction shows "←" arrow
 * - "symmetric" direction shows "↔" arrow
 * - Renders registryLabel
 * - Renders sourceLabel
 * - Renders targetLabel
 * - Renders evidenceCount
 * - Renders validity
 *
 * Requirements: F4.4 (task 4.4.2)
 */
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@solidjs/testing-library";
import { RelationItemDetail } from "./RelationItemDetail";
import type { RelationItem } from "./Inspector";

afterEach(() => cleanup());

function makeItem(overrides: Partial<RelationItem> = {}): RelationItem {
  return {
    id: "rel-test",
    direction: "outgoing",
    registryLabel: "knows",
    sourceLabel: "Alice",
    targetLabel: "Bob",
    evidenceCount: 3,
    validity: "active",
    ...overrides,
  };
}

function renderItem(overrides: Partial<RelationItem> = {}) {
  const item = makeItem(overrides);
  render(() => <RelationItemDetail item={item} />);
  return screen.getByTestId(`relation-detail-${item.id}`);
}

describe("RelationItemDetail", () => {
  // ─── direction ─────────────────────────────────────────────────────────────

  it("renders direction with data-direction attribute for outgoing", () => {
    const el = renderItem({ direction: "outgoing" });
    const dirEl = el.querySelector("[data-field='direction']");
    expect(dirEl).not.toBeNull();
    expect(dirEl).toHaveAttribute("data-direction", "outgoing");
  });

  it("renders direction with data-direction attribute for incoming", () => {
    const el = renderItem({ direction: "incoming" });
    const dirEl = el.querySelector("[data-field='direction']");
    expect(dirEl).not.toBeNull();
    expect(dirEl).toHaveAttribute("data-direction", "incoming");
  });

  it("renders direction with data-direction attribute for symmetric", () => {
    const el = renderItem({ direction: "symmetric" });
    const dirEl = el.querySelector("[data-field='direction']");
    expect(dirEl).not.toBeNull();
    expect(dirEl).toHaveAttribute("data-direction", "symmetric");
  });

  it("outgoing direction shows → arrow", () => {
    const el = renderItem({ direction: "outgoing" });
    expect(el.querySelector("[data-field='direction']")).toHaveTextContent("→");
  });

  it("incoming direction shows ← arrow", () => {
    const el = renderItem({ direction: "incoming" });
    expect(el.querySelector("[data-field='direction']")).toHaveTextContent("←");
  });

  it("symmetric direction shows ↔ arrow", () => {
    const el = renderItem({ direction: "symmetric" });
    expect(el.querySelector("[data-field='direction']")).toHaveTextContent("↔");
  });

  // ─── registryLabel ─────────────────────────────────────────────────────────

  it("renders registryLabel", () => {
    const el = renderItem({ registryLabel: "member-of" });
    expect(el.querySelector("[data-field='registry-label']")).toHaveTextContent("member-of");
  });

  // ─── sourceLabel ───────────────────────────────────────────────────────────

  it("renders sourceLabel", () => {
    const el = renderItem({ sourceLabel: "Paris" });
    expect(el.querySelector("[data-field='source-label']")).toHaveTextContent("Paris");
  });

  // ─── targetLabel ───────────────────────────────────────────────────────────

  it("renders targetLabel", () => {
    const el = renderItem({ targetLabel: "France" });
    expect(el.querySelector("[data-field='target-label']")).toHaveTextContent("France");
  });

  // ─── evidenceCount ─────────────────────────────────────────────────────────

  it("renders evidenceCount", () => {
    const el = renderItem({ evidenceCount: 7 });
    expect(el.querySelector("[data-field='evidence-count']")).toHaveTextContent("7");
  });

  it("renders evidenceCount of 0", () => {
    const el = renderItem({ evidenceCount: 0 });
    expect(el.querySelector("[data-field='evidence-count']")).toHaveTextContent("0");
  });

  // ─── validity ──────────────────────────────────────────────────────────────

  it("renders validity", () => {
    const el = renderItem({ validity: "active" });
    expect(el.querySelector("[data-field='validity']")).toHaveTextContent("active");
  });

  it("renders validity when expired", () => {
    const el = renderItem({ validity: "expired" });
    expect(el.querySelector("[data-field='validity']")).toHaveTextContent("expired");
  });

  it("renders validity when pending", () => {
    const el = renderItem({ validity: "pending" });
    expect(el.querySelector("[data-field='validity']")).toHaveTextContent("pending");
  });
});
