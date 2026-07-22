import { describe, it, expect, afterEach, vi } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

import VoiceLine from "./VoiceLine";
import type { FocusVoiceLine } from "../../../stores/homeFocusStore";
import type { Route } from "../../router";

afterEach(cleanup);

/** Build a FocusVoiceLine subject with sensible defaults for tests. */
function subject(over: Partial<FocusVoiceLine> = {}): FocusVoiceLine {
  return {
    subjectId: over.subjectId ?? "s1",
    text: over.text ?? "Standup in 20 — want the notes?",
    key: over.key ?? "k1",
    actionable: over.actionable ?? false,
    link: over.link,
    priority: over.priority ?? 80,
    confidence: over.confidence ?? 0.8,
    emphasis: over.emphasis ?? "high",
  };
}

/** Let the component's createEffect flush. */
const flush = () => Promise.resolve();

describe("VoiceLine — Focus headline (Req 3.1–3.6)", () => {
  it("renders the current Voice Line as a single line beneath the Core (Req 3.1)", () => {
    const { container, getByText } = render(() => (
      <VoiceLine line={() => subject({ text: "Evening, Obaid." })} />
    ));
    const region = container.querySelector('[data-region="voice-line"]');
    expect(region).toBeInTheDocument();
    expect(getByText("Evening, Obaid.")).toBeInTheDocument();
    // Exactly one line node (never a dashboard / multiple items).
    expect(container.querySelectorAll(".kria-voiceline__line").length).toBe(1);
  });

  it("exposes the line as a polite, atomic, once-announcing live region (Req 3.5)", () => {
    const { container } = render(() => <VoiceLine line={() => subject()} />);
    const line = container.querySelector(".kria-voiceline__line")!;
    expect(line.getAttribute("role")).toBe("status");
    expect(line.getAttribute("aria-live")).toBe("polite");
    expect(line.getAttribute("aria-atomic")).toBe("true");
    // A live region announces on text change and never takes focus itself.
    expect(line.hasAttribute("tabindex")).toBe(false);
  });

  it("renders NOTHING when no subject qualifies — never an empty box (design §14, Req 3.2)", () => {
    const { container } = render(() => <VoiceLine line={() => undefined} />);
    expect(container.querySelector('[data-region="voice-line"]')).not.toBeInTheDocument();
    expect(container.querySelector(".kria-voiceline")).not.toBeInTheDocument();
  });

  it("renders NOTHING when reading the frame throws (failure isolation, design §14)", () => {
    const { container } = render(() => (
      <VoiceLine
        line={() => {
          throw new Error("frame error");
        }}
      />
    ));
    expect(container.querySelector('[data-region="voice-line"]')).not.toBeInTheDocument();
  });

  it("does not re-announce identical consecutive text (no consecutive repeat, Req 3.3)", async () => {
    const [line, setLine] = createSignal<FocusVoiceLine | undefined>(
      subject({ text: "Download finished.", key: "k1" }),
    );
    const { container, getAllByText } = render(() => <VoiceLine line={line} />);
    expect(getAllByText("Download finished.").length).toBe(1);

    // A brand-new subject object with the SAME text arrives (e.g. recompute).
    setLine(subject({ text: "Download finished.", key: "k2", subjectId: "s2" }));
    await flush();

    // No crossfade ghost was created → it was a silent no-op (no re-announce).
    expect(container.querySelector(".kria-voiceline__ghost")).not.toBeInTheDocument();
    expect(getAllByText("Download finished.").length).toBe(1);
  });

  it("crossfades between subjects — an aria-hidden ghost of the old line fades out (Req 3.4)", async () => {
    const [line, setLine] = createSignal<FocusVoiceLine | undefined>(
      subject({ text: "First subject." }),
    );
    const { container, getByText } = render(() => <VoiceLine line={line} />);

    setLine(subject({ text: "Second subject.", key: "k2", subjectId: "s2" }));
    await flush();

    // Incoming line shows the new text (live region), outgoing ghost holds the
    // old text and is hidden from AT.
    expect(getByText("Second subject.")).toBeInTheDocument();
    const ghost = container.querySelector(".kria-voiceline__ghost");
    expect(ghost).toBeInTheDocument();
    expect(ghost?.getAttribute("aria-hidden")).toBe("true");
    expect(ghost?.textContent).toBe("First subject.");
    expect(container.querySelector('[data-region="voice-line"]')?.getAttribute("data-transitioning")).toBe("true");
  });

  it("swaps instantly with no ghost under reduced motion (fade-only, Req 17.4/21.4)", async () => {
    const [line, setLine] = createSignal<FocusVoiceLine | undefined>(subject({ text: "A." }));
    const { container, getByText } = render(() => <VoiceLine line={line} reducedMotion />);
    expect(container.querySelector('[data-region="voice-line"]')?.getAttribute("data-motion")).toBe("static");

    setLine(subject({ text: "B.", key: "k2", subjectId: "s2" }));
    await flush();

    expect(getByText("B.")).toBeInTheDocument();
    // No crossfade ghost in the static path.
    expect(container.querySelector(".kria-voiceline__ghost")).not.toBeInTheDocument();
  });

  it("renders a routing-only deep link that navigates, never sends (Req 3.6)", () => {
    const onNavigate = vi.fn();
    const link: Route = { space: "converse", segment: "thread", entityId: "t-42" };
    const { container } = render(() => (
      <VoiceLine
        line={() => subject({ text: "Resume your draft", actionable: true, link })}
        onNavigate={onNavigate}
      />
    ));
    const control = container.querySelector<HTMLButtonElement>(".kria-voiceline__link");
    expect(control).toBeInTheDocument();
    // Keyboard-operable: it is a real button (Enter/Space activate natively).
    expect(control?.tagName).toBe("BUTTON");
    expect(control?.getAttribute("data-role")).toBe("deep-link");

    control!.click();
    // Activation ROUTES ONLY — exactly the supplied route, no other effect.
    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith(link);
    expect(container.querySelector('[data-region="voice-line"]')?.getAttribute("data-actionable")).toBe("true");
  });

  it("renders a plain, non-interactive line when the subject is not actionable (Req 3.6)", () => {
    const { container } = render(() => (
      <VoiceLine line={() => subject({ text: "Just resting.", actionable: false })} />
    ));
    expect(container.querySelector(".kria-voiceline__link")).not.toBeInTheDocument();
    expect(container.querySelector(".kria-voiceline__text")?.textContent).toBe("Just resting.");
    expect(container.querySelector('[data-region="voice-line"]')?.getAttribute("data-actionable")).toBe("false");
  });

  it("recedes to nothing when the subject clears (Req 3.2)", async () => {
    const [line, setLine] = createSignal<FocusVoiceLine | undefined>(subject({ text: "Something." }));
    const { container } = render(() => <VoiceLine line={line} reducedMotion />);
    expect(container.querySelector('[data-region="voice-line"]')).toBeInTheDocument();

    setLine(undefined);
    await flush();
    expect(container.querySelector('[data-region="voice-line"]')).not.toBeInTheDocument();
  });
});
