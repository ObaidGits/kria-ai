/**
 * Composer — grow-then-scroll, attachments, mode chip, voice entry, and the
 * single Send⇄Stop action (task 3.4, Req 4.4 / 4.5 / 4.9).
 *
 * Send/Stop are injected as stubs here so we assert the COMPOSER's behavior
 * (when it dispatches) without touching the pipeline; the store test proves
 * those defaults route through the existing commands.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";
import Composer from "./Composer";
import { converseStore, coreStore } from "../../../stores";

function resetDraft(): void {
  converseStore.setActiveThread(null);
  converseStore.updateDraft({ text: "", attachments: [], mode: "assistant" });
  converseStore.clearMessages();
  coreStore.reset(); // idle → not working
}

beforeEach(() => {
  cleanup();
  resetDraft();
});

describe("Composer — grow-then-scroll (Req 4.4)", () => {
  it("grows the textarea rows with content, then caps at the max", () => {
    render(() => <Composer />);
    const textarea = screen.getByLabelText("Message KRIA") as HTMLTextAreaElement;

    // Single line → 1 row.
    expect(textarea.rows).toBe(1);

    // Three lines → 3 rows (grows).
    converseStore.updateDraft({ text: "a\nb\nc" });
    expect(textarea.rows).toBe(3);

    // Twenty lines → capped at 8 rows (then scrolls internally via CSS).
    converseStore.updateDraft({ text: Array.from({ length: 20 }, (_, i) => `l${i}`).join("\n") });
    expect(textarea.rows).toBe(8);
  });
});

describe("Composer — Enter to send / Shift+Enter newline (Req 4.4)", () => {
  it("sends on Enter when non-empty and idle", () => {
    const onSend = vi.fn();
    converseStore.updateDraft({ text: "ship it" });
    render(() => <Composer onSend={onSend} />);
    const textarea = screen.getByLabelText("Message KRIA");

    fireEvent.keyDown(textarea, { key: "Enter" });
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it("does NOT send on Shift+Enter (inserts a newline instead)", () => {
    const onSend = vi.fn();
    converseStore.updateDraft({ text: "line one" });
    render(() => <Composer onSend={onSend} />);
    const textarea = screen.getByLabelText("Message KRIA");

    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
  });

  it("does NOT send on Enter when the draft is empty", () => {
    const onSend = vi.fn();
    render(() => <Composer onSend={onSend} />);
    const textarea = screen.getByLabelText("Message KRIA");

    fireEvent.keyDown(textarea, { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();
  });
});

describe("Composer — single Send⇄Stop action (Req 4.4)", () => {
  it("disables Send when the draft is empty", () => {
    render(() => <Composer />);
    const send = screen.getByRole("button", { name: "Send message" });
    expect(send).toBeDisabled();
  });

  it("enables Send when there is text", () => {
    converseStore.updateDraft({ text: "hi" });
    render(() => <Composer />);
    expect(screen.getByRole("button", { name: "Send message" })).not.toBeDisabled();
  });

  it("becomes a prominent Stop while KRIA works, and Stop dispatches cancel", () => {
    const onStop = vi.fn();
    converseStore.updateDraft({ text: "working…" });
    coreStore.setState("thinking"); // Core active → working
    render(() => <Composer onStop={onStop} />);

    // Send is replaced by Stop.
    expect(screen.queryByRole("button", { name: "Send message" })).toBeNull();
    const stop = screen.getByRole("button", { name: "Stop" });
    expect(stop).not.toBeDisabled();

    fireEvent.click(stop);
    expect(onStop).toHaveBeenCalledTimes(1);
  });
});

describe("Composer — mode chip Assistant⇆Lab, per-thread (Req 4.9 / 4.5)", () => {
  it("toggles Assistant → Lab and reflects pressed state", () => {
    converseStore.setActiveThread("mode-thread");
    render(() => <Composer />);

    const chip = screen.getByRole("button", { name: "Assistant" });
    expect(chip).toHaveAttribute("aria-pressed", "false");

    fireEvent.click(chip);

    const lab = screen.getByRole("button", { name: "Lab" });
    expect(lab).toHaveAttribute("aria-pressed", "true");
    expect(converseStore.composerDraft().mode).toBe("lab");
  });

  it("persists the mode per thread (restores Lab on return)", () => {
    converseStore.setActiveThread("m1");
    render(() => <Composer />);
    fireEvent.click(screen.getByRole("button", { name: "Assistant" }));
    expect(converseStore.composerDraft().mode).toBe("lab");

    converseStore.setActiveThread("m2");
    expect(converseStore.composerDraft().mode).toBe("assistant");

    converseStore.setActiveThread("m1");
    expect(converseStore.composerDraft().mode).toBe("lab");
  });
});

describe("Composer — draft text persists per thread on switch (Req 4.5)", () => {
  it("restores the thread's draft text when switching back", () => {
    converseStore.setActiveThread("d1");
    render(() => <Composer />);
    converseStore.updateDraft({ text: "remember me" });

    converseStore.setActiveThread("d2");
    expect((screen.getByLabelText("Message KRIA") as HTMLTextAreaElement).value).toBe("");

    converseStore.setActiveThread("d1");
    expect((screen.getByLabelText("Message KRIA") as HTMLTextAreaElement).value).toBe("remember me");
  });
});

describe("Composer — attachments add/remove (Req 4.4)", () => {
  it("adds attachment chips from the file picker and removes them", () => {
    render(() => <Composer />);

    // Seed a real attachment payload through the draft.
    converseStore.updateDraft({
      attachments: [{
        id: "notes-attachment",
        name: "notes.txt",
        mime: "text/plain",
        size: 5,
        bytes: new Uint8Array([104, 101, 108, 108, 111]),
      }],
    });
    expect(screen.getByText("notes.txt")).toBeInTheDocument();

    // Remove it.
    fireEvent.click(screen.getByRole("button", { name: "Remove notes.txt" }));
    expect(screen.queryByText("notes.txt")).toBeNull();
    expect(converseStore.composerDraft().attachments).toHaveLength(0);
  });

  it("stages real file bytes selected via the hidden file input", async () => {
    const { container } = render(() => <Composer />);
    const input = container.querySelector<HTMLInputElement>("input[type=file]")!;
    const file = new File(["x"], "report.pdf", { type: "application/pdf" });

    fireEvent.change(input, { target: { files: [file] } });
    await waitFor(() => {
      expect(converseStore.composerDraft().attachments[0]).toMatchObject({
        name: "report.pdf",
        mime: "application/pdf",
        size: 1,
      });
    });
    expect(Array.from(converseStore.composerDraft().attachments[0].bytes)).toEqual([120]);
  });
});

describe("Composer — voice entry (Req 12 entry affordance)", () => {
  it("triggers the voice-start path when the mic button is pressed", () => {
    const onVoiceStart = vi.fn();
    render(() => <Composer onVoiceStart={onVoiceStart} />);
    fireEvent.click(screen.getByRole("button", { name: "Start voice input" }));
    expect(onVoiceStart).toHaveBeenCalledTimes(1);
  });
});

describe("Composer — no separate slash menu (Req 4.7)", () => {
  it("does NOT open a slash menu when the input starts with '/'", () => {
    const { container } = render(() => <Composer />);
    const textarea = screen.getByLabelText("Message KRIA") as HTMLTextAreaElement;

    // Type a leading slash + a would-be command name.
    fireEvent.input(textarea, { target: { value: "/clear" } });

    // "/" is a normal character in the composer — the text is preserved and no
    // slash menu (legacy) is rendered. Commands live in the Command Palette.
    expect(converseStore.composerDraft().text).toBe("/clear");
    expect(container.querySelector(".slash-menu")).toBeNull();
    expect(container.querySelector(".slash-command-item")).toBeNull();
    // No competing menu/listbox popup is spawned by the composer either.
    expect(screen.queryByRole("menu")).toBeNull();
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("sends a '/'-prefixed message like any other text (no interception)", () => {
    const onSend = vi.fn();
    converseStore.updateDraft({ text: "/session" });
    render(() => <Composer onSend={onSend} />);
    const textarea = screen.getByLabelText("Message KRIA");

    fireEvent.keyDown(textarea, { key: "Enter" });
    expect(onSend).toHaveBeenCalledTimes(1);
  });
});
