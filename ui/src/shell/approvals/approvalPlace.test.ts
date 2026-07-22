/**
 * Approval place snapshot + focus-return tests (task 8.3, gap G7).
 *
 * Proves the AppShell approval place snapshot (design §20.3 Focus_Return_Owner)
 * returns focus following the §20.4 fallback ladder when the pending Approval
 * Center queue clears:
 *   • original invoker still present → focus (and caret) returns exactly there;
 *   • invoker removed → owning region heading, then container;
 *   • region removed → `#space-root`;
 *   • `#space-root` removed → a stable shell control;
 *   • focus never lands on an Approve/destructive control;
 *   • scroll place is preserved and no draft/selection state is reset.
 */
import { describe, it, expect, beforeEach } from "vitest";
import {
  captureApprovalPlace,
  restoreApprovalPlace,
  isDestructiveTarget,
} from "./approvalPlace";

beforeEach(() => {
  document.body.innerHTML = "";
});

function mount(html: string): void {
  document.body.innerHTML = html;
}

describe("approval place focus-return (design §20.4, gap G7)", () => {
  it("returns focus to the original invoker when it still exists", () => {
    mount(`
      <section aria-label="Converse">
        <h2>Converse</h2>
        <button id="invoker">Reply</button>
      </section>
    `);
    const invoker = document.getElementById("invoker") as HTMLButtonElement;
    invoker.focus();
    expect(document.activeElement).toBe(invoker);

    const snap = captureApprovalPlace();
    // …Approval Center seizes focus while pending…
    document.body.querySelector<HTMLElement>(".kria-approvals")?.focus();

    restoreApprovalPlace(snap);
    expect(document.activeElement).toBe(invoker);
  });

  it("restores the caret/selection of a text invoker", () => {
    mount(`<section><textarea id="composer">hello world</textarea></section>`);
    const ta = document.getElementById("composer") as HTMLTextAreaElement;
    ta.focus();
    ta.setSelectionRange(2, 5);

    const snap = captureApprovalPlace();
    (document.activeElement as HTMLElement)?.blur();

    restoreApprovalPlace(snap);
    expect(document.activeElement).toBe(ta);
    expect(ta.selectionStart).toBe(2);
    expect(ta.selectionEnd).toBe(5);
  });

  it("falls back to the owning region heading when the invoker is removed", () => {
    mount(`
      <section id="region" aria-label="Converse">
        <h2 id="heading">Converse</h2>
        <button id="invoker">Reply</button>
      </section>
    `);
    const invoker = document.getElementById("invoker") as HTMLButtonElement;
    invoker.focus();
    const snap = captureApprovalPlace();

    // Route/lane change removed the invoker but the region survived.
    invoker.remove();

    restoreApprovalPlace(snap);
    expect(document.activeElement).toBe(document.getElementById("heading"));
  });

  it("falls back to the owning region container when it has no heading", () => {
    mount(`
      <section id="region" aria-label="Converse">
        <button id="invoker">Reply</button>
      </section>
    `);
    const invoker = document.getElementById("invoker") as HTMLButtonElement;
    invoker.focus();
    const snap = captureApprovalPlace();

    invoker.remove();

    restoreApprovalPlace(snap);
    expect(document.activeElement).toBe(document.getElementById("region"));
  });

  it("falls back to #space-root when the owning region is gone", () => {
    mount(`
      <main id="space-root" tabindex="-1" aria-label="Primary workspace"></main>
      <section id="region" aria-label="Converse">
        <button id="invoker">Reply</button>
      </section>
    `);
    const invoker = document.getElementById("invoker") as HTMLButtonElement;
    invoker.focus();
    const snap = captureApprovalPlace();

    document.getElementById("region")?.remove();

    restoreApprovalPlace(snap);
    expect(document.activeElement).toBe(document.getElementById("space-root"));
  });

  it("falls back to a stable shell control when #space-root is also gone", () => {
    mount(`
      <a class="kria-skip-link" href="#space-root">Skip to workspace</a>
      <section id="region" aria-label="Converse">
        <button id="invoker">Reply</button>
      </section>
    `);
    const invoker = document.getElementById("invoker") as HTMLButtonElement;
    invoker.focus();
    const snap = captureApprovalPlace();

    document.getElementById("region")?.remove();

    restoreApprovalPlace(snap);
    expect(document.activeElement).toBe(
      document.querySelector<HTMLElement>(".kria-skip-link")
    );
  });

  it("never returns focus to an Approve/destructive control (§20.4)", () => {
    // The captured invoker itself is an Approve control (edge case): the ladder
    // must skip it and land on the safe owning region instead.
    mount(`
      <section id="region" aria-label="Converse">
        <h2 id="heading">Converse</h2>
        <button id="invoker" class="kria-approval-card__approve">Approve</button>
      </section>
    `);
    const invoker = document.getElementById("invoker") as HTMLButtonElement;
    invoker.focus();
    const snap = captureApprovalPlace();

    restoreApprovalPlace(snap);
    expect(document.activeElement).not.toBe(invoker);
    expect(document.activeElement).toBe(document.getElementById("heading"));
  });

  it("classifies approve/deny/destructive controls", () => {
    mount(`
      <button id="a" class="kria-approval-card__approve">Approve</button>
      <button id="b" class="kria-approval-card__deny">Deny</button>
      <button id="c" data-destructive="true">Wipe</button>
      <button id="d" aria-label="Delete thread">x</button>
      <button id="safe">Reply</button>
    `);
    for (const id of ["a", "b", "c", "d"]) {
      expect(isDestructiveTarget(document.getElementById(id) as HTMLElement)).toBe(true);
    }
    expect(isDestructiveTarget(document.getElementById("safe") as HTMLElement)).toBe(false);
  });

  it("preserves scroll place and does not disturb draft text on restore", () => {
    mount(`
      <section id="region" aria-label="Converse">
        <div id="scroller" style="overflow:auto"></div>
        <textarea id="composer">draft text</textarea>
        <button id="invoker">Reply</button>
      </section>
    `);
    const scroller = document.getElementById("scroller") as HTMLElement;
    // jsdom has no layout; force a non-zero scroll offset so it is captured.
    Object.defineProperty(scroller, "scrollTop", { value: 120, writable: true });
    const invoker = document.getElementById("invoker") as HTMLButtonElement;
    invoker.focus();

    const snap = captureApprovalPlace();
    // Simulate the invoker being removed so the fallback path runs.
    invoker.remove();
    scroller.scrollTop = 0;

    restoreApprovalPlace(snap);
    expect(scroller.scrollTop).toBe(120);
    // Draft content is untouched by focus restoration.
    expect((document.getElementById("composer") as HTMLTextAreaElement).value).toBe("draft text");
  });
});
