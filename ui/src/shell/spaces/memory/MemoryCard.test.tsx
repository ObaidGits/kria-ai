import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { MemoryCard } from "./MemoryCard";
import { shellStore, type MemoryFact } from "../../../stores";

function makeFact(over: Partial<MemoryFact> = {}): MemoryFact {
  const now = Date.now();
  return {
    id: "m1",
    content: "the sky is blue",
    confidence: 0.82,
    worth: 0.6,
    staleness: 0.1,
    source: "conversation",
    createdAt: now,
    updatedAt: now,
    tags: [],
    ...over,
  };
}

describe("MemoryCard (task 6.2, Req 5.2/17.3)", () => {
  beforeEach(() => shellStore.setInspectorTarget(null));
  afterEach(() => cleanup());

  it("renders the content and cue fields as text (icon+text, not color-only)", () => {
    render(() => <MemoryCard fact={makeFact()} />);
    expect(screen.getByText("the sky is blue")).toBeInTheDocument();
    expect(screen.getByText("82% confidence")).toBeInTheDocument();
    expect(screen.getByText("worth 60%")).toBeInTheDocument();
    expect(screen.getByText("fresh")).toBeInTheDocument();
    expect(screen.getByText("conversation")).toBeInTheDocument();
  });

  it("is a single labelled button (keyboard-operable)", () => {
    render(() => <MemoryCard fact={makeFact()} />);
    const btn = screen.getByRole("button", { name: /Memory: the sky is blue/ });
    expect(btn).toBeInTheDocument();
  });

  it("opens the shared Inspector for this memory on click (Req 1.6/5.2)", () => {
    render(() => <MemoryCard fact={makeFact({ id: "abc" })} />);
    fireEvent.click(screen.getByRole("button", { name: /Memory:/ }));
    const t = shellStore.inspectorTarget();
    expect(t?.type).toBe("memory");
    expect(t?.id).toBe("abc");
  });

  it("uses an onOpen override when provided (stories/tests)", () => {
    let opened: string | null = null;
    render(() => <MemoryCard fact={makeFact({ id: "z9" })} onOpen={(f) => (opened = f.id)} />);
    fireEvent.click(screen.getByRole("button", { name: /Memory:/ }));
    expect(opened).toBe("z9");
    // Override means the shared inspector target is NOT set here.
    expect(shellStore.inspectorTarget()).toBeNull();
  });

  it("reflects selection via aria-selected", () => {
    render(() => <MemoryCard fact={makeFact()} selected />);
    expect(screen.getByRole("button", { name: /Memory:/ })).toHaveAttribute("aria-selected", "true");
  });
});
