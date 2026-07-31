/**
 * HomeSpace — the presence homepage surface (design.md §14, Requirement 22).
 *
 * This is the SCAFFOLD stage (tasks 0.2). HomeSpace is the component the home
 * surface routes to when the `home.presence.v2` feature flag is ON; when the
 * flag is OFF the home surface keeps rendering the existing Converse empty
 * state (`ConverseEmptyState`). The current empty state therefore stays fully
 * operational until Phase-2 gates pass (Req 22.1); rollback is a flag flip.
 *
 * Per design §14, HomeSpace's eventual responsibility is to *compose* the
 * homepage — owning layout, the reading-mode switch and width profile, and
 * reading `homeFocusStore` + `coreStore` + `homeStore`. Those internals (the
 * Room, the Focus engine + Focus UI, the unified Composer, hybrid navigation,
 * the 3D Core) are owned by later tasks (1.x–9.x) and are intentionally NOT
 * built here.
 *
 * Failure-mode invariant (design §14): if `homeFocusStore` errors, HomeSpace
 * renders Core + Composer only — NEVER blank. At this scaffold stage there is
 * no `homeFocusStore` yet, so the guarantee is upheld structurally: HomeSpace
 * always renders a labelled region with a prominent `CorePresence` and a
 * heading. It can never return an empty node.
 *
 * Pure presentation: reads `coreStore` (via `CorePresence`) only. No
 * orchestration, no tool calls, no send logic (KRIA runtime-authority
 * invariant). The homepage `Composer` lives in the surrounding Converse layout
 * and is unchanged by this scaffold.
 *
 * Shared light (design §3.2, task 1.2): HomeSpace mounts the shared-light
 * publisher — the single mechanism that writes the `--core-*` custom properties
 * from the Core state (≤1/frame, paused on blur, idle-quiet) — and wraps its
 * content in the `Room`, which CONSUMES those variables so the environment
 * reacts to the one Core light. Publication only READS `coreStore` (never
 * writes back — Req 30.3).
 *
 * Resting calm (task 1.4, Req 1.5): at rest the homepage is Core + optional
 * greeting ONLY. It renders no placeholder widgets, empty cards, stat tiles,
 * charts, or filler — the resting-calm guardrail (`checkRestingCalm`) asserts
 * this against the rendered DOM. The greeting slot is marked `data-slot=
 * "greeting"` so it is recognised as the one allowed optional line, not filler.
 *
 * Reduced motion (task 1.4, Req 1.6/17.4): HomeSpace does not force a motion
 * mode. The `Room` self-detects OS `prefers-reduced-motion` AND the global
 * kill-switch (`data-reduced-motion="on"`) and freezes its atmosphere to a
 * static frame; `CorePresence` independently honors the same signals. So the
 * WHOLE Room composition (particles, floor sheen, undertone, Core) degrades to
 * static together with no per-surface wiring here. The shared-light publisher
 * and undertone controller stay idle-quiet + paused-on-blur, so a static Room
 * costs ~0 at rest.
 *
 * Requirements: 22.1, 22.2, 1.1, 1.5, 1.6, 17.2, 17.4, 17.5
 */
import { Show } from "solid-js";
import { CorePresence } from "../../../components/CorePresence";
import { homeStore } from "../../../stores/homeStore";
import { isElementVisibleAtRest } from "../../viewModeResponsibilityMatrix";
import { Room } from "./Room";
import { VoiceLine } from "./VoiceLine";
import { AdaptiveContextSurface } from "./AdaptiveContextSurface";
import { PermissionSurface } from "./PermissionSurface";
import { Composer } from "./Composer";
import { ContextualChips } from "./ContextualChips";
import { ContextualOrbit } from "./ContextualOrbit";
import { PresenceOnboarding } from "./PresenceOnboarding";
import { TrustIndicator } from "./TrustIndicator";
import { createSharedLightPublisher, presenceIntent } from "./sharedLight";
import { createRoomUndertoneController } from "./roomUndertone";
import "./HomeSpace.css";

export interface HomeSpaceProps {
  /** Optional class hook for the surrounding layout. */
  class?: string;
}

/**
 * The presence homepage. Minimal, never-blank scaffold: a Core-forward region
 * with a single calm heading. Later phases layer the Room, Focus UI, Composer,
 * and navigation on top of this shell.
 */
export function HomeSpace(props: HomeSpaceProps) {
  // Publish `--core-*` shared-light variables from the Core state (≤1/frame,
  // paused on blur, idle-quiet). Reactive + rAF-throttled; no always-on loop.
  // Read-only w.r.t. coreStore (Req 30.3). Teardown is owned by this scope.
  createSharedLightPublisher();

  // Slow, mood-only time-of-day undertone (≤6% toward info in morning / warning
  // at night). Coarse-scheduled + paused-on-blur + fully disabled under the
  // steady-lighting preference (Req 1.4 / 21.4). Writes only `--room-undertone`.
  createRoomUndertoneController();

  return (
    <section
      class={`kria-home ${props.class ?? ""}`.trim()}
      data-region="home-space"
      aria-label="Home"
    >
      {/* The Room consumes the published `--core-*` light so the environment
          reacts to the one Core presence (design §3.2). */}
      <Room class="kria-home__room">
        {/* Core-forward: the living presence anchors the room. CorePresence
            carries its own accessible label + reduced-motion handling (Req 3).
            On the homepage the Core is INTERACTIVE with exactly the two talking
            interactions (Req 2.3): activate opens voice + focus-readies the
            Composer, press-hold is push-to-talk. Activate drives the meaningful-
            intent lean toward the Composer via `presenceIntent` (design §4.2 /
            §3.2); the Composer itself (task 5.1) will consume the same signal
            and take DOM focus. No navigation/menu behavior is attached (Req 2.4). */}
        <div class="kria-home__core">
          <CorePresence
            size="lg"
            interactive
            onRequestComposerFocus={() => presenceIntent.setComposerFocused(true)}
          />

          {/* The Contextual Orbit — partial, temporary capability-awareness
              light-points AROUND the Core (design §6.4, task 6.2). Absent at
              rest; appears only while engaged (`homeStore.orbitEngaged` — set on
              composer focus / task start) with ≥1 lit point, and fades out on
              disengage. Body language, not a menu; actionable points ROUTE ONLY
              (Req 6.4). It is the single capability-awareness system (Req 6.5)
              and degrades to static labelled dots under reduced motion (Req
              6.6). Reads the same live Focus frame as the Voice Line / chips.
              Per the §29 responsibility matrix the Orbit is part of the resting
              composition only in Immersive/Standard; Mini/Companion hide it
              (palette owns capability lookup there). */}
          <Show when={isElementVisibleAtRest("orbit", homeStore.viewMode())}>
            <ContextualOrbit class="kria-home__orbit" />
          </Show>
        </div>

        {/* Optional greeting slot — the ONE line allowed beside the Core at
            rest (Req 1.5). Marked so the resting-calm guardrail treats it as a
            greeting, never filler. The Focus engine (task 3.x) will later drive
            this copy; the static line keeps the surface never-blank until then. */}
        <h2 class="kria-home__title" data-slot="greeting">What can I help with?</h2>

        {/* Presence Onboarding: one-time Core, shared navigation rail, and
            Orbit capability cues. NOT a tour and NEVER repeats: each hint is an
            independent one-time cue persisted in the existing coach-hint ledger.
            Once every hint is retired it renders nothing, so it is additive and
            never a resting placeholder. Teaching/routing only — it never sends,
            executes, or writes coreStore (runtime-authority invariant). */}
        <PresenceOnboarding class="kria-home__onboarding" />

        {/* The Voice Line — one adaptive sentence beneath the Core, bound to the
            live Focus frame (design §6.1, task 4.1). It renders NOTHING at rest
            (never an empty box), so resting calm (Req 1.5) is preserved; the
            Focus engine supplies a subject only when one qualifies. */}
        <VoiceLine class="kria-home__voice-line" />

        {/* The Adaptive Context Surface — the BODY of the current Focus subject,
            same subject as the Voice Line when both render (design §6.2, task
            4.2). One living-glass surface at one fixed location; it renders
            NOTHING at rest / on failure (dissolves — never an empty box), so
            resting calm (Req 1.5 / 8.3) is preserved. The Focus engine supplies
            an ACS only for a HIGH-emphasis subject. Per the §29 responsibility
            matrix the ACS composes only in Immersive/Standard; Mini/Companion
            hide it. */}
        <Show when={isElementVisibleAtRest("acs", homeStore.viewMode())}>
          <AdaptiveContextSurface class="kria-home__acs" />
        </Show>

        {/* The Permission UX — approval through presence (design §10.4, task
            8.5). It renders the ONE current permission subject from
            `approvalStore` in the presence style its risk tier demands: GREEN →
            a report + Undo (non-blocking, Req 10.1); YELLOW → intent + a brief
            halt window (Req 10.2); RED/BLACK → a single-line Allow/Deny with
            what/why visible, routing detail to the Approval Center (Req
            10.3/10.4). It renders NOTHING at rest, and DEFERS (renders nothing)
            whenever a blocking overlay/modal is already open — so it never
            stacks a modal-on-modal (Req 10.3). Reuses the existing approvalStore
            + Approval Center; it never executes an action itself. */}
        <PermissionSurface class="kria-home__permission" />

        {/* The Composer — the homepage's SINGLE primary action target, on the
            true vertical center axis (design §2, task 5.1). Unified
            text/command/voice with the mic as a peer input and a discoverable
            ⌘K/Ctrl K command hint (Req 4.1/4.2). On focus it strengthens its
            own rim-light AND drives the meaningful-intent lean so the Core leans
            toward it (Req 4.3). It reuses the Converse Composer + the same
            per-thread draft, so a chip-staged draft appears here (Req 4.3). When
            this presence homepage owns the surface the sticky Converse composer
            is suppressed (see ConverseSpace), so the homepage has exactly one
            ask-field (Req 4.2 — no second competing field). */}
        <Composer class="kria-home__composer" />

        {/* Contextual Chips — ≤3 live next-actions, positioned BENEATH the
            Composer (design §2/§6.3, task 4.3). They render NOTHING at rest /
            when no real action exists (never generic filler), so resting calm
            (Req 1.5 / 5.2) is preserved. Each chip stages a reviewable draft or
            routes — it NEVER sends or executes (Req 5.3). Per the §29
            responsibility matrix the chips compose only in Immersive/Standard;
            Mini hides them (palette owns quick actions) and Companion has none. */}
        <Show when={isElementVisibleAtRest("chips", homeStore.viewMode())}>
          <ContextualChips class="kria-home__chips" />
        </Show>

        {/* Trust confirmation — the quiet on-device/local-first affordance,
            positioned near the Composer (design §2, task 8.6, Req 9). It stays
            lit whether online or offline (local-first is healthy, never an
            error — Req 9.1), is MUTED/non-emerald (Req 9.2), lights a directed
            Core→edge reach cue while KRIA acts on the device (Req 9.1), and
            routes full privacy detail to the Memory & Privacy Settings group on
            activation (Req 9.3). Read-only over coreStore + connectivity; never
            sends, executes, or writes coreStore. */}
        <TrustIndicator class="kria-home__trust" />
      </Room>
    </section>
  );
}

export default HomeSpace;
