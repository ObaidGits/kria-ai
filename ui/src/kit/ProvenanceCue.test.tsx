import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { ProvenanceCue, type ProvenanceSource } from "./ProvenanceCue";

afterEach(cleanup);

describe("ProvenanceCue — authorship (Req 20.5)", () => {
  const cases: Array<[ProvenanceSource, string, string]> = [
    ["kria", "KRIA", "AI-authored by KRIA"],
    ["user", "You", "User-authored"],
  ];

  it.each(cases)("marks %s content with icon, text, and machine-readable cue", (source, text, label) => {
    const { container } = render(() => <ProvenanceCue source={source} />);
    const cue = screen.getByLabelText(label);
    expect(cue).toHaveAttribute("data-provenance-cue", source);
    expect(cue).toHaveTextContent(text);
    expect(container.querySelector("svg")).not.toBeNull();
  });

  it("supports context-specific KRIA action labels", () => {
    render(() => <ProvenanceCue source="kria" label="KRIA action" />);
    expect(screen.getByLabelText("AI-authored by KRIA")).toHaveTextContent("KRIA action");
  });
});
