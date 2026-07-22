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
import { getTerm } from "../../terminology";

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

describe("Composer — primary idle task-entry hierarchy (UIE-H-001, Req 5.1)", () => {
  it("marks the composer root as the primary task-entry control", () => {
    const { container } = render(() => <Composer />);
    const root = container.querySelector(".kria-composer");
    expect(root).not.toBeNull();
    // The prominence marker the CSS keys off so the Composer visually dominates
    // the reduced-weight command-palette trigger.
    expect(root?.getAttribute("data-primary-entry")).toBe("true");
  });

  it("gives the composer a bordered, raised surface for visual prominence", async () => {
    const { default: composerCss } = await import("./Composer.css?raw");
    expect(composerCss).toMatch(
      /\.kria-composer\[data-primary-entry="true"\]\s*\{[\s\S]*?border:\s*1px\s+solid\s+var\(--color-border-default\);[\s\S]*?background:\s*var\(--color-surface-2\);/,
    );
    // Focus-within accent ring reinforces it as the place work begins.
    expect(composerCss).toMatch(
      /\.kria-composer\[data-primary-entry="true"\]:focus-within\s*\{[\s\S]*?border-color:\s*var\(--color-accent-border\);/,
    );
  });

  it("keeps the labelled Message KRIA field as the focal input", () => {
    render(() => <Composer />);
    expect(screen.getByLabelText("Message KRIA")).toBeInTheDocument();
  });
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

describe("Composer — tall-draft clearance backstop (task 9.5, IU-10; Req 15.5–15.7)", () => {
  // A grown draft must NOT let the Composer expand without bound: once the
  // `rows` attribute clamps at MAX_ROWS=8, the CSS max-height + internal scroll
  // is the visual backstop so a tall draft scrolls INSIDE the textarea instead
  // of growing the Composer row and covering the last readable message. This
  // locks the CSS half of the grow-then-scroll contract the rows-clamp test
  // above proves on the component side.
  it("caps the textarea height and scrolls internally (max-height + overflow-y:auto)", async () => {
    const { default: composerCss } = await import("./Composer.css?raw");
    expect(composerCss).toMatch(
      /\.kria-composer__textarea\s*\{[\s\S]*?max-height:\s*calc\([\s\S]*?\);[\s\S]*?overflow-y:\s*auto;/,
    );
    // The cap is tied to 8 rows (MAX_ROWS) so growth stops exactly where the
    // component clamps `rows` — the two halves agree.
    expect(composerCss).toMatch(/\.kria-composer__textarea\s*\{[\s\S]*?max-height:\s*calc\([\s\S]*?8em/);
  });

  it("keeps the tall-draft textarea itself the only Composer scroller (not the last message)", () => {
    render(() => <Composer />);
    const textarea = screen.getByLabelText("Message KRIA") as HTMLTextAreaElement;
    // Even a very tall draft clamps rows at MAX_ROWS=8; past the cap the textarea
    // (overflow-y:auto) owns the overflow — the Composer row height stays bounded,
    // so the stream above it is never pushed/covered.
    converseStore.updateDraft({ text: Array.from({ length: 200 }, (_, i) => `l${i}`).join("\n") });
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

    // Send is replaced by Stop (scope-named "Stop response", UIE-M-015).
    expect(screen.queryByRole("button", { name: "Send message" })).toBeNull();
    const stop = screen.getByRole("button", { name: "Stop response" });
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

  it("describes the Lab-mode decision with the concise matrix outcome (task 7.7, Req 7.6)", () => {
    resetDraft();
    converseStore.setActiveThread("mode-desc-thread");
    render(() => <Composer />);

    // The mode chip carries the Lab-mode outcome READ FROM the terminology
    // matrix (single source of truth) as its hover/focus description — so the
    // Assistant⇆Lab distinction is explained at this decision point.
    const chip = screen.getByRole("button", { name: "Assistant" });
    expect(chip.getAttribute("title")).toBe(`Lab mode: ${getTerm("lab-mode").outcome}`);
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

describe("Composer — Width Profile control adaptation (task 8.6, UIE-M-003)", () => {
  const profiles = ["focus", "dual", "assisted", "full"] as const;

  /** Open a kit Menu / OverflowControl trigger via its accessible name (a11y path). */
  function openDisclosure(name: string): void {
    const trigger = screen.getByRole("button", { name });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
  }

  it("keeps Send⇄Stop inline and full-size at every profile", () => {
    for (const profile of profiles) {
      cleanup();
      resetDraft();
      converseStore.updateDraft({ text: "ready" });
      render(() => <Composer widthProfile={profile} />);
      // Send is a direct, full-size Button (never in a disclosure) at all widths.
      expect(screen.getByRole("button", { name: "Send message" }), `${profile}: Send inline`).toBeInTheDocument();

      // While working, the same slot is a direct Stop.
      cleanup();
      resetDraft();
      converseStore.updateDraft({ text: "working…" });
      coreStore.setState("thinking");
      render(() => <Composer widthProfile={profile} />);
      expect(screen.getByRole("button", { name: "Stop response" }), `${profile}: Stop inline`).toBeInTheDocument();
    }
  });

  it("keeps Attach and Voice REACHABLE at every profile (inline or labelled disclosure)", () => {
    for (const profile of profiles) {
      cleanup();
      resetDraft();
      render(() => <Composer widthProfile={profile} />);

      const inlineAttach = screen.queryByRole("button", { name: "Attach a file" });
      const inlineVoice = screen.queryByRole("button", { name: "Start voice input" });

      if (inlineAttach && inlineVoice) {
        // Wide profiles: both tools inline, no disclosure needed.
        expect(screen.queryByRole("button", { name: "More composer actions" }), `${profile}: no disclosure`).toBeNull();
      } else {
        // Narrow profile: both fold into ONE labelled disclosure — never absent.
        expect(inlineAttach, `${profile}: attach not inline`).toBeNull();
        expect(inlineVoice, `${profile}: voice not inline`).toBeNull();
        openDisclosure("More composer actions");
        expect(screen.getByRole("menuitem", { name: "Attach a file" }), `${profile}: attach reachable`).toBeInTheDocument();
        expect(screen.getByRole("menuitem", { name: "Start voice input" }), `${profile}: voice reachable`).toBeInTheDocument();
      }
    }
  });

  it("collapses Attach + Voice into the disclosure at the narrowest (focus) profile, keeping the mode chip inline", () => {
    render(() => <Composer widthProfile="focus" />);
    // Mode chip (primary) stays directly reachable.
    expect(screen.getByRole("button", { name: "Assistant" })).toBeInTheDocument();
    // Attach/Voice are not inline buttons at focus.
    expect(screen.queryByRole("button", { name: "Attach a file" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Start voice input" })).toBeNull();
    // One labelled disclosure carries them.
    expect(screen.getByRole("button", { name: "More composer actions" })).toBeInTheDocument();
  });

  it("shows Attach + Voice inline with no disclosure at wide profiles (dual/assisted/full)", () => {
    for (const profile of ["dual", "assisted", "full"] as const) {
      cleanup();
      resetDraft();
      render(() => <Composer widthProfile={profile} />);
      expect(screen.getByRole("button", { name: "Attach a file" }), `${profile}: attach inline`).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Start voice input" }), `${profile}: voice inline`).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "More composer actions" }), `${profile}: no disclosure`).toBeNull();
    }
  });

  it("never renders a tool both inline and in the disclosure (no duplicate action)", () => {
    render(() => <Composer widthProfile="focus" />);
    // Collapsed: attach absent inline.
    expect(screen.queryByRole("button", { name: "Attach a file" })).toBeNull();
    openDisclosure("More composer actions");
    // Present as exactly one menuitem, and still not as an inline button.
    expect(screen.getAllByRole("menuitem", { name: "Attach a file" })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Attach a file" })).toBeNull();
  });

  it("keeps the textarea grow-then-scroll behavior unchanged at the narrowest profile", () => {
    render(() => <Composer widthProfile="focus" />);
    const textarea = screen.getByLabelText("Message KRIA") as HTMLTextAreaElement;
    expect(textarea.rows).toBe(1);
    converseStore.updateDraft({ text: "a\nb\nc" });
    expect(textarea.rows).toBe(3);
    converseStore.updateDraft({ text: Array.from({ length: 20 }, (_, i) => `l${i}`).join("\n") });
    expect(textarea.rows).toBe(8);
  });

  it("preserves draft text, attachments, and mode across a focus→full→focus tool transition", () => {
    converseStore.setActiveThread("composer-preserve");
    converseStore.updateDraft({
      text: "keep this draft",
      mode: "lab",
      attachments: [{
        id: "keep-attachment",
        name: "keep.txt",
        mime: "text/plain",
        size: 3,
        bytes: new Uint8Array([97, 98, 99]),
      }],
    });

    // Narrow → Attach/Voice collapsed.
    const view = render(() => <Composer widthProfile="focus" />);
    expect(screen.getByRole("button", { name: "More composer actions" })).toBeInTheDocument();
    expect((screen.getByLabelText("Message KRIA") as HTMLTextAreaElement).value).toBe("keep this draft");

    // Widen → tools inline. Draft/attachment/mode are store-owned, so they hold.
    view.unmount();
    render(() => <Composer widthProfile="full" />);
    expect(screen.getByRole("button", { name: "Attach a file" })).toBeInTheDocument();
    expect(screen.getByText("keep.txt")).toBeInTheDocument();
    expect((screen.getByLabelText("Message KRIA") as HTMLTextAreaElement).value).toBe("keep this draft");
    expect(converseStore.composerDraft().mode).toBe("lab");
    // Mode chip reflects the preserved Lab mode.
    expect(screen.getByRole("button", { name: "Lab" })).toHaveAttribute("aria-pressed", "true");
  });
});
