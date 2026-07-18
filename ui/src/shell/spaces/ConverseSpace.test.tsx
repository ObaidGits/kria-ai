import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import ConverseSpace from "./ConverseSpace";
import { converseStore, coreStore, shellStore } from "../../stores";
import type { WorkBlock } from "../../stores/converseStore";

function makeWorkBlock(id: string): WorkBlock {
  return {
    id,
    type: "tool-call",
    status: "running",
    summary: `work ${id}`,
    startedAt: Date.now(),
  };
}

describe("ConverseSpace — three-lane layout (task 3.1, Req 4.1/4.3)", () => {
  beforeEach(() => {
    // Reset shared singletons so each test starts from a clean, standard shell.
    shellStore.setWindowMode("standard");
    converseStore.clearMessages(); // clears messages + work blocks + context rail
    coreStore.reset(); // idle → not active
  });

  it("presents the ConversationLane as the focal, dominant lane (Req 4.1/4.3)", () => {
    render(() => <ConverseSpace />);
    const conversation = screen.getByRole("region", { name: "Conversation" });
    expect(conversation).toBeInTheDocument();
    // Dominance is asserted via the layout marker the CSS keys off of.
    expect(conversation).toHaveAttribute("data-dominant", "true");
    expect(conversation).toHaveAttribute("data-lane", "conversation");
  });

  it("renders the focal message-stream container and the sticky Composer (Req 4.1/4.4)", () => {
    const { container } = render(() => <ConverseSpace />);
    expect(screen.getByRole("log", { name: "Message stream" })).toBeInTheDocument();
    expect(container.querySelector('[data-region="composer"]')).not.toBeNull();
  });

  it("keeps the Composer AFTER the message region in the DOM so it never covers the last message (Req 4.4)", () => {
    const { container } = render(() => <ConverseSpace />);
    const stream = container.querySelector('[data-region="message-stream"]')!;
    const composer = container.querySelector('[data-region="composer"]')!;
    expect(stream).not.toBeNull();
    expect(composer).not.toBeNull();
    // compareDocumentPosition: FOLLOWING (4) means composer comes after stream.
    const rel = stream.compareDocumentPosition(composer);
    expect(rel & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("hides the WorkLane by default and reveals it on work activity (Req 4.1/4.2)", () => {
    // Default: idle Core, no work blocks → WorkLane hidden.
    render(() => <ConverseSpace />);
    expect(screen.queryByRole("complementary", { name: "Work" })).toBeNull();
    cleanup();

    // With a work block present, the adaptive WorkLane reveals.
    converseStore.addWorkBlock(makeWorkBlock("wb-1"));
    render(() => <ConverseSpace />);
    expect(screen.getByRole("complementary", { name: "Work" })).toBeInTheDocument();
  });

  it("streams new work blocks into the WorkLane as they arrive (Req 4.2)", async () => {
    converseStore.addWorkBlock(makeWorkBlock("wb-stream-1"));
    render(() => <ConverseSpace />);
    // First block rendered as a real WorkBlock group with its summary.
    expect(await screen.findByText("work wb-stream-1")).toBeInTheDocument();

    // A new block arriving appends without a re-mount (fine-grained reactivity).
    converseStore.addWorkBlock(makeWorkBlock("wb-stream-2"));
    expect(await screen.findByText("work wb-stream-2")).toBeInTheDocument();
    expect(screen.getAllByRole("group").length).toBeGreaterThanOrEqual(2);
  });

  it("keeps the ContextRail on-demand: hidden by default, toggled by the user (Req 4.1)", async () => {
    render(() => <ConverseSpace />);
    const toggle = screen.getByRole("button", { name: "Toggle context rail" });
    expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull();
    expect(toggle).toHaveAttribute("aria-pressed", "false");

    // Toggle it open via the on-demand control.
    fireEvent.click(toggle);
    expect(await screen.findByRole("complementary", { name: "Context" })).toBeInTheDocument();

    // Toggle it closed again.
    fireEvent.click(toggle);
    expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull();
  });

  it("offers a collapsible ThreadSidebar in Standard mode (Req 4.1 / §6.1)", () => {
    render(() => <ConverseSpace />);
    expect(screen.getByRole("navigation", { name: "Threads" })).toBeInTheDocument();
  });

  it("curates away the secondary lanes in Compact window-mode (Req 15.2)", () => {
    shellStore.setWindowMode("compact");
    // Even with work activity, the secondary lanes are dropped (not squished).
    converseStore.addWorkBlock(makeWorkBlock("wb-2"));
    render(() => <ConverseSpace />);

    // Focal conversation + composer remain.
    expect(screen.getByRole("region", { name: "Conversation" })).toBeInTheDocument();
    // Secondary lanes are curated away.
    expect(screen.queryByRole("complementary", { name: "Work" })).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Threads" })).toBeNull();
    expect(screen.queryByRole("button", { name: /context rail/i })).toBeNull();
  });
});
