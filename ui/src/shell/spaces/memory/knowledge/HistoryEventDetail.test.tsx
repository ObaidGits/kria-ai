/**
 * Tests for HistoryEventDetail component (task 4.4.2).
 *
 * Validates:
 * - "correction"    eventType shows "Correction applied" and data-event-class="mutation"
 * - "supersession"  eventType shows "Superseded"          and data-event-class="lifecycle"
 * - "contradiction" eventType shows "Contradiction recorded" and data-event-class="truth"
 * - "creation"      eventType shows "Created"              and data-event-class="lifecycle"
 * - "deletion"      eventType shows "Deleted"              and data-event-class="lifecycle"
 * - Unknown eventType shows raw value from backend
 * - Renders timestamp
 * - Renders actor when non-null; hides when null
 * - Renders description
 *
 * Requirements: F4.4 (task 4.4.2)
 */
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@solidjs/testing-library";
import { HistoryEventDetail } from "./HistoryEventDetail";
import type { HistoryEvent } from "./Inspector";

afterEach(() => cleanup());

function makeEvent(overrides: Partial<HistoryEvent> = {}): HistoryEvent {
  return {
    id: "ev-test",
    eventType: "creation",
    timestamp: "2024-01-01T00:00:00Z",
    actor: null,
    description: "Test description",
    ...overrides,
  };
}

function renderEvent(overrides: Partial<HistoryEvent> = {}) {
  const event = makeEvent(overrides);
  render(() => <HistoryEventDetail event={event} />);
  return screen.getByTestId(`history-detail-${event.id}`);
}

describe("HistoryEventDetail", () => {
  // ─── eventType label and class mapping ─────────────────────────────────────

  it('"correction" eventType shows "Correction applied" and data-event-class="mutation"', () => {
    const el = renderEvent({ eventType: "correction" });
    const typeEl = el.querySelector("[data-field='event-type']");
    expect(typeEl).toHaveTextContent("Correction applied");
    expect(typeEl).toHaveAttribute("data-event-class", "mutation");
  });

  it('"supersession" eventType shows "Superseded" and data-event-class="lifecycle"', () => {
    const el = renderEvent({ eventType: "supersession" });
    const typeEl = el.querySelector("[data-field='event-type']");
    expect(typeEl).toHaveTextContent("Superseded");
    expect(typeEl).toHaveAttribute("data-event-class", "lifecycle");
  });

  it('"contradiction" eventType shows "Contradiction recorded" and data-event-class="truth"', () => {
    const el = renderEvent({ eventType: "contradiction" });
    const typeEl = el.querySelector("[data-field='event-type']");
    expect(typeEl).toHaveTextContent("Contradiction recorded");
    expect(typeEl).toHaveAttribute("data-event-class", "truth");
  });

  it('"creation" eventType shows "Created" and data-event-class="lifecycle"', () => {
    const el = renderEvent({ eventType: "creation" });
    const typeEl = el.querySelector("[data-field='event-type']");
    expect(typeEl).toHaveTextContent("Created");
    expect(typeEl).toHaveAttribute("data-event-class", "lifecycle");
  });

  it('"deletion" eventType shows "Deleted" and data-event-class="lifecycle"', () => {
    const el = renderEvent({ eventType: "deletion" });
    const typeEl = el.querySelector("[data-field='event-type']");
    expect(typeEl).toHaveTextContent("Deleted");
    expect(typeEl).toHaveAttribute("data-event-class", "lifecycle");
  });

  it("unknown eventType shows raw value from backend", () => {
    const el = renderEvent({ eventType: "some-custom-event" });
    const typeEl = el.querySelector("[data-field='event-type']");
    expect(typeEl).toHaveTextContent("some-custom-event");
  });

  it("data-event-type attribute always reflects the raw backend value", () => {
    const el = renderEvent({ eventType: "correction" });
    expect(el.querySelector("[data-field='event-type']")).toHaveAttribute("data-event-type", "correction");
  });

  it("unknown eventType does not set data-event-class", () => {
    const el = renderEvent({ eventType: "unknown-type" });
    const typeEl = el.querySelector("[data-field='event-type']");
    // data-event-class should be undefined/absent for unknown types
    expect(typeEl?.getAttribute("data-event-class")).toBeFalsy();
  });

  // ─── timestamp ─────────────────────────────────────────────────────────────

  it("renders timestamp", () => {
    const el = renderEvent({ timestamp: "2024-06-15T12:30:00Z" });
    expect(el.querySelector("[data-field='timestamp']")).toHaveTextContent("2024-06-15T12:30:00Z");
  });

  // ─── actor ─────────────────────────────────────────────────────────────────

  it("renders actor when non-null", () => {
    const el = renderEvent({ actor: "system-agent" });
    expect(el.querySelector("[data-field='actor']")).toHaveTextContent("system-agent");
  });

  it("hides actor when null", () => {
    const el = renderEvent({ actor: null });
    expect(el.querySelector("[data-field='actor']")).toBeNull();
  });

  // ─── description ───────────────────────────────────────────────────────────

  it("renders description exactly from backend", () => {
    const el = renderEvent({ description: "Value was corrected from X to Y" });
    expect(el.querySelector("[data-field='description']")).toHaveTextContent("Value was corrected from X to Y");
  });
});
