import { describe, it, expect, afterEach, vi } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

import AdaptiveContextSurface from "./AdaptiveContextSurface";
import VoiceLine from "./VoiceLine";
import { deriveFocusFrame } from "../../../stores/homeFocusStore";
import type { FocusAcs, FocusVoiceLine, FocusInputs } from "../../../stores/homeFocusStore";
import type { ApprovalRequest } from "../../../stores/approvalStore";
import { checkRestingCalm, findEmptyStandingSurfaces } from "./guardrails";
import type { Route } from "../../router";

afterEach(cleanup);

/** Build a FocusAcs subject with sensible defaults for tests. */
function acs(over: Partial<FocusAcs> = {}): FocusAcs {
  return {
    subjectId: over.subjectId ?? "s1",
    title: over.title ?? "1 approval waiting",
    line: over.line ?? "Send the weekly report to the team channel.",
    action: over.action,
    ownerRoute: over.ownerRoute ?? { space: "converse", segment: "approvals" },
  };
}

/** Let the component's createEffect flush. */
const flush = () => Promise.resolve();

describe("AdaptiveContextSurface — Focus body (Req 8.1–8.5)", () => {
  it("renders exactly one surface at a fixed location with a single subject (Req 8.1)", () => {
    const { container, getByText } = render(() => (
      <AdaptiveContextSurface acs={() => acs({ title: "Meeting soon", line: "Standup in 20." })} />
    ));
    const surfaces = container.querySelectorAll('[data-region="adaptive-context-surface"]');
    expect(surfaces.length).toBe(1);
    // One title + one line — never a dashboard / multiple items (Req 8.3).
    expect(container.querySelectorAll(".kria-acs__title").length).toBe(1);
    expect(container.querySelectorAll(".kria-acs__line").length).toBe(1);
    expect(getByText("Meeting soon")).toBeInTheDocument();
    expect(getByText("Standup in 20.")).toBeInTheDocument();
  });

  it("offers at most one action verb (Req 8.2)", () => {
    const run = vi.fn();
    const { container } = render(() => (
      <AdaptiveContextSurface acs={() => acs({ action: { label: "Review", run } })} />
    ));
    // Exactly one action verb; the route-to-owner affordance is separate.
    expect(container.querySelectorAll('[data-role="acs-action"]').length).toBe(1);
    expect(container.querySelectorAll('[data-role="acs-detail"]').length).toBe(1);
  });

  it("action runs its engine-supplied routing/staging callback only — never sends (Req 8.2)", () => {
    const run = vi.fn();
    const { container } = render(() => (
      <AdaptiveContextSurface acs={() => acs({ action: { label: "Open draft", run } })} />
    ));
    const btn = container.querySelector<HTMLButtonElement>('[data-role="acs-action"]');
    expect(btn?.tagName).toBe("BUTTON");
    btn!.click();
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("routes to the owning Space for deeper detail — routing only (Req 8.2)", () => {
    const onNavigate = vi.fn();
    const ownerRoute: Route = { space: "automations", segment: "workflow", entityId: "w-9" };
    const { container } = render(() => (
      <AdaptiveContextSurface acs={() => acs({ ownerRoute })} onNavigate={onNavigate} />
    ));
    const detail = container.querySelector<HTMLButtonElement>('[data-role="acs-detail"]');
    expect(detail?.tagName).toBe("BUTTON");
    detail!.click();
    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith(ownerRoute);
  });

  it("exposes a labelled region whose body is a polite, atomic, once-announcing live region that never steals focus (Req 8.5)", () => {
    const { container } = render(() => <AdaptiveContextSurface acs={() => acs()} />);
    const region = container.querySelector('[data-region="adaptive-context-surface"]')!;
    expect(region.getAttribute("role")).toBe("region");
    expect(region.getAttribute("aria-label")).toBe("Context");
    const body = region.querySelector(".kria-acs__body")!;
    expect(body.getAttribute("role")).toBe("status");
    expect(body.getAttribute("aria-live")).toBe("polite");
    expect(body.getAttribute("aria-atomic")).toBe("true");
    // A live region announces on change and never takes focus itself.
    expect(region.hasAttribute("tabindex")).toBe(false);
    expect(body.hasAttribute("tabindex")).toBe(false);
  });

  it("dissolves (renders NOTHING, no empty box) when no subject qualifies — guardrail clean (Req 8.3)", () => {
    const { container } = render(() => <AdaptiveContextSurface acs={() => undefined} />);
    expect(container.querySelector('[data-region="adaptive-context-surface"]')).not.toBeInTheDocument();
    expect(container.querySelector(".kria-acs")).not.toBeInTheDocument();
    expect(container.querySelector(".kria-acs-slot")).not.toBeInTheDocument();
    // The resting-calm guardrail must find no empty standing surface / filler.
    expect(findEmptyStandingSurfaces(container)).toEqual([]);
    expect(checkRestingCalm(container)).toEqual([]);
  });

  it("dissolves when reading the frame throws (failure → dissolve, design §14)", () => {
    const { container } = render(() => (
      <AdaptiveContextSurface
        acs={() => {
          throw new Error("frame error");
        }}
      />
    ));
    expect(container.querySelector('[data-region="adaptive-context-surface"]')).not.toBeInTheDocument();
    expect(checkRestingCalm(container)).toEqual([]);
  });

  it("recedes to nothing when the subject clears (Req 8.3)", async () => {
    const [subject, setSubject] = createSignal<FocusAcs | undefined>(acs());
    const { container } = render(() => <AdaptiveContextSurface acs={subject} reducedMotion />);
    expect(container.querySelector('[data-region="adaptive-context-surface"]')).toBeInTheDocument();

    setSubject(undefined);
    await flush();
    expect(container.querySelector('[data-region="adaptive-context-surface"]')).not.toBeInTheDocument();
    expect(checkRestingCalm(container)).toEqual([]);
  });

  it("crossfades between subjects — an aria-hidden ghost of the old subject fades out (Req 8.5)", async () => {
    const [subject, setSubject] = createSignal<FocusAcs | undefined>(acs({ title: "First", line: "one" }));
    const { container, getByText } = render(() => <AdaptiveContextSurface acs={subject} />);

    setSubject(acs({ subjectId: "s2", title: "Second", line: "two" }));
    await flush();

    expect(getByText("Second")).toBeInTheDocument();
    const ghost = container.querySelector(".kria-acs--ghost");
    expect(ghost).toBeInTheDocument();
    expect(ghost?.getAttribute("aria-hidden")).toBe("true");
    expect(ghost?.textContent).toContain("First");
    // The ghost never carries the dissolves-when-empty marker → guardrail clean.
    expect(findEmptyStandingSurfaces(container)).toEqual([]);
    expect(
      container.querySelector('[data-region="adaptive-context-surface"]')?.getAttribute("data-transitioning"),
    ).toBe("true");
  });

  it("swaps instantly with no ghost under reduced motion (Req 17.4/21.4)", async () => {
    const [subject, setSubject] = createSignal<FocusAcs | undefined>(acs({ title: "A", line: "a" }));
    const { container, getByText } = render(() => <AdaptiveContextSurface acs={subject} reducedMotion />);
    expect(
      container.querySelector('[data-region="adaptive-context-surface"]')?.getAttribute("data-motion"),
    ).toBe("static");

    setSubject(acs({ subjectId: "s2", title: "B", line: "b" }));
    await flush();
    expect(getByText("B")).toBeInTheDocument();
    expect(container.querySelector(".kria-acs--ghost")).not.toBeInTheDocument();
  });

  it("does not re-announce identical consecutive content (once-announce, Req 8.5)", async () => {
    const [subject, setSubject] = createSignal<FocusAcs | undefined>(acs({ title: "Same", line: "same" }));
    const { container, getAllByText } = render(() => <AdaptiveContextSurface acs={subject} />);
    expect(getAllByText("Same").length).toBe(1);

    // A new object with the SAME subject/content arrives (e.g. recompute).
    setSubject(acs({ subjectId: "s1", title: "Same", line: "same" }));
    await flush();
    // No crossfade ghost was created → it was a silent no-op (no re-announce).
    expect(container.querySelector(".kria-acs--ghost")).not.toBeInTheDocument();
    expect(getAllByText("Same").length).toBe(1);
  });

  it("binds to the SAME subject as the Voice Line when both render (Req 8.4)", () => {
    // Drive both densities from ONE real Focus frame so the binding invariant is
    // exercised end-to-end, not just asserted structurally.
    const approval = {
      id: "ap-1",
      type: "tool-hitl",
      title: "Send weekly report",
      description: "Post the weekly report to #team.",
      risk: "red",
      status: "pending",
      createdAt: 1_000,
    } as ApprovalRequest;
    const inputs: FocusInputs = {
      approvals: [approval],
      threads: [],
      activeThreadId: null,
      conversing: false,
      workflows: [],
      facts: [],
      notifications: [],
      awareness: [],
      now: 2_000,
    };
    const frame = deriveFocusFrame(inputs);
    expect(frame.voiceLine).toBeDefined();
    expect(frame.acs).toBeDefined();

    const voice = () => frame.voiceLine as FocusVoiceLine;
    const surface = () => frame.acs as FocusAcs;
    const { container } = render(() => (
      <>
        <VoiceLine line={voice} />
        <AdaptiveContextSurface acs={surface} />
      </>
    ));
    const region = container.querySelector('[data-region="adaptive-context-surface"]')!;
    // Exactly one surface, bound to the Voice Line's subject id (never two).
    expect(container.querySelectorAll('[data-region="adaptive-context-surface"]').length).toBe(1);
    expect(region.getAttribute("data-subject-id")).toBe(frame.voiceLine!.subjectId);
    expect(frame.acs!.subjectId).toBe(frame.voiceLine!.subjectId);
  });
});
