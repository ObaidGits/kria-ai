import type { Meta, StoryObj } from "storybook-solidjs-vite";
import type { JSX } from "solid-js";

/**
 * Homepage presence — component workbench SCAFFOLDS (task 0.4, Requirement
 * 16.4: "version design-system changes and update the component workbench
 * stories accordingly").
 *
 * Per design.md §14, the presence homepage introduces a set of new/adapted
 * components. Only `HomeSpace` exists as a real scaffold today (see
 * `HomeSpace.stories.tsx`). This file reserves a labelled workbench entry for
 * every remaining component so later tasks (1.x–9.x) have a story slot to fill
 * in as each component is built — while keeping the story build, `tsc` and the
 * raw-color lint green.
 *
 * These entries intentionally render a truthful "not yet implemented"
 * placeholder rather than inventing component internals. When a component lands
 * in its owning task, replace the matching story's `render` with the real
 * component (and, where useful, split it into its own `<Component>.stories.tsx`
 * next to the component, matching the ConverseSpace/MemorySpace convention).
 *
 * Zero raw color: placeholders are text/structure only — no color literals —
 * so they never trip the design-system raw-color lint.
 */

/** A truthful, clearly-labelled scaffold placeholder for a pending component. */
function ScaffoldPlaceholder(props: {
  name: string;
  task: string;
  requirements: string;
  summary: string;
}): JSX.Element {
  return (
    <section
      class="kria-scaffold-placeholder"
      data-scaffold="not-implemented"
      data-component={props.name}
      aria-label={`${props.name} — not yet implemented`}
    >
      <p data-scaffold-status>Not yet implemented — workbench scaffold.</p>
      <h2>{props.name}</h2>
      <p>{props.summary}</p>
      <dl>
        <dt>Owning task</dt>
        <dd>{props.task}</dd>
        <dt>Requirements</dt>
        <dd>{props.requirements}</dd>
      </dl>
    </section>
  );
}

const meta = {
  title: "Spaces/Homepage Presence (Scaffolds)",
  component: ScaffoldPlaceholder,
} satisfies Meta<typeof ScaffoldPlaceholder>;

export default meta;
type Story = StoryObj<typeof meta>;

/**
 * `Room` is now a real component (task 1.1) — its workbench entries live in the
 * sibling `Room.stories.tsx` (title "Spaces/Home/Room"), following the
 * one-story-file-per-component convention. This scaffold slot is intentionally
 * retired; no placeholder remains for Room.
 */

/** `CorePresence` (extended) — 3D-gated renderer + shared-light publication. */
export const CorePresenceExtended: Story = {
  args: {
    name: "CorePresence (extended)",
    task: "2.1–2.3, 7.1",
    requirements: "2.1, 2.2, 2.5, 2.6",
    summary:
      "Adds --core-* variable publication, lean/step-forward/recede behaviors, activate/press-hold interactions, and a capability-gated 3D renderer. The 2D scaffold ships today via CorePresence.stories usage in HomeSpace.",
  },
};

/**
 * `VoiceLine` is now a real component (task 4.1) — its workbench entries live in
 * the sibling `VoiceLine.stories.tsx` (title "Spaces/Home/VoiceLine"), following
 * the one-story-file-per-component convention. This scaffold slot is retired.
 */

/**
 * `AdaptiveContextSurface` is now a real component (task 4.2) — its workbench
 * entries live in the sibling `AdaptiveContextSurface.stories.tsx` (title
 * "Spaces/Home/AdaptiveContextSurface"), following the one-story-file-per-
 * component convention. This scaffold slot is retired.
 */

/**
 * `ContextualChips` is now a real component (task 4.3) — its workbench entries
 * live in the sibling `ContextualChips.stories.tsx` (title
 * "Spaces/Home/ContextualChips"), following the one-story-file-per-component
 * convention. This scaffold slot is retired.
 */

/**
 * `ContextualOrbit` is now a real component (task 6.2) — its workbench entries
 * live in the sibling `ContextualOrbit.stories.tsx` (title
 * "Spaces/Home/ContextualOrbit"), following the one-story-file-per-component
 * convention. This scaffold slot is retired; no placeholder remains for
 * ContextualOrbit.
 */

/**
 * `HiddenDock` is now a real component (task 6.1) — its workbench entries live
 * in the sibling `HiddenDock.stories.tsx` (title "Spaces/Home/HiddenDock"),
 * following the one-story-file-per-component convention. This scaffold slot is
 * retired; no placeholder remains for HiddenDock.
 */

/**
 * `CompanionEmber` is now a real component (task 8.3) — its workbench entries
 * live in the sibling `CompanionEmber.stories.tsx` (title
 * "Spaces/Home/CompanionEmber"), following the one-story-file-per-component
 * convention. This scaffold slot is retired; no placeholder remains for
 * CompanionEmber.
 */

/** `TrustIndicator` — muted on-device confirmation. */
export const TrustIndicator: Story = {
  args: {
    name: "TrustIndicator",
    task: "8.6",
    requirements: "9.1, 9.2, 9.3",
    summary:
      "Muted on-device confirmation; stays lit offline; visible Core→edge reach on desktop action; routes to Settings.",
  },
};

/**
 * `ReadingMode` is now real (task 8.4): the depth-recession layer ships as
 * `ReadingBackdrop`, with its workbench entries in the sibling
 * `ReadingBackdrop.stories.tsx` (title "Spaces/Home/ReadingBackdrop"), and the
 * first-send→recede / empty→reverse wiring + near-solid AA reading backing live
 * in `readingMode.ts` + `ConverseSpace`. This scaffold slot is retired.
 */
