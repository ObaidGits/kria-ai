/**
 * ReadingBackdrop — the receded Room + ambient Core behind Reading Mode
 * (design.md §11, Requirement 11.1 / 11.2 / 11.3).
 *
 * This is the depth-recession made visible. When a conversation begins the
 * homepage does NOT swap to a new page or dock the Core to a corner (Req 11.1):
 * the Room and Core stay in the SAME space and recede in depth behind the
 * conversation. `ReadingBackdrop` renders that receded layer:
 *
 *   • the Room — reused, not rebuilt — pushed back in depth (transform recede)
 *     with its particles/floor **settled** to a still frame (Req 11.2 "particles
 *     and filament SHALL settle"), and
 *   • an ambient, non-interactive `CorePresence` that has "drifted up/back and
 *     dims to an ambient glow" (Req 11.1) — CorePresence continuity is preserved
 *     (same component, same live `coreStore` state), just calmed and receded,
 *   • a hard-dim scrim (`--reading-dim`) drawn over the Room so the atmosphere
 *     recedes HARD behind text (Req 11.2), beneath the near-solid reading
 *     backing that the conversation column carries.
 *
 * Pure presentation + decoration: the whole backdrop is `aria-hidden` (the
 * conversation/message-stream is the dominant, announced surface in Reading
 * Mode, Req 11.4). It reads `coreStore` only via `CorePresence`; no store
 * writes, no orchestration, no `coreStore` mutation (authority invariant).
 *
 * Token-only (zero raw color, Req 16.2): the recede depth, dim, and backing are
 * all token-driven. Reduced-motion safe (Req 11.2 "settle motion" → instant):
 * the settle entrance is opacity/transform only and is removed under the global
 * kill-switch (`data-reduced-motion="on"`) and OS `prefers-reduced-motion`, so
 * the receded frame simply appears.
 *
 * Requirements: 11.1, 11.2, 11.3
 */
import { CorePresence } from "../../../components/CorePresence";
import { Room } from "./Room";
import "./ReadingBackdrop.css";

/**
 * Particle count for the receded Room. The field is frozen (settled) via the
 * `reducedMotion` Room prop; a lightly reduced count keeps the receded
 * atmosphere calm and cheap behind the reading column (design §11.5 sheds
 * particles first). Kept ≤ the tokenized `--particle-count-max`.
 */
export const READING_PARTICLE_COUNT = 18;

export interface ReadingBackdropProps {
  /** Optional class hook for the surrounding layout. */
  class?: string;
}

/**
 * The receded Room + ambient Core layer rendered behind the conversation while
 * Reading Mode is active. Render it as the first (behind) child of the
 * conversation surface; the message stream + its near-solid reading backing sit
 * above it in normal flow.
 */
export function ReadingBackdrop(props: ReadingBackdropProps) {
  return (
    <div
      class={`kria-reading-backdrop ${props.class ?? ""}`.trim()}
      data-region="reading-backdrop"
      aria-hidden="true"
    >
      {/* Receded Room — reused, pushed back in depth, particles settled. The
          recede transform + settle entrance live in CSS on this wrapper so the
          Room component itself stays generic (Req 11.1 depth-recession). */}
      <div class="kria-reading-backdrop__room">
        <Room reducedMotion particleCount={READING_PARTICLE_COUNT}>
          {/* Ambient Core — drifted up/back to a dim glow (Req 11.1). Non-
              interactive: the two talking interactions belong to the resting
              homepage Core / Composer, not to this receded ambient presence. */}
          <div class="kria-reading-backdrop__core">
            <CorePresence size="sm" />
          </div>
        </Room>
      </div>

      {/* Hard-dim scrim — the Room recedes hard behind text (Req 11.2). Sits
          above the receded Room, beneath the reading column's near-solid
          backing. Fades in with the settle so the recession reads as one calm
          motion (reduced-motion → instant). */}
      <div class="kria-reading-backdrop__dim" aria-hidden="true" />
    </div>
  );
}

export default ReadingBackdrop;
