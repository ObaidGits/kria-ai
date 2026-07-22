/**
 * Command Deck population — operational panels organised into Mission Control
 * zones (Phase 7).
 *
 * The panel COMPONENTS are unchanged (relocated in Phase 2). Here we assign each
 * a layout `region` so the deck arranges them into a coherent operational flow
 * (Current Activity → Running Operations → Mission Status → Upcoming) rather than
 * an undifferentiated grid. The former "Quick Commands" panel is superseded by
 * the context-aware quick operational actions in the Mission Header.
 */
import { commandDeckRegistry } from "./deckRegistry";
import {
  AgentsPanel,
  IntelligenceFeedPanel,
  OverviewPanel,
  TimelinePanel,
} from "../command-center/panels";

let registered = false;

/** Idempotently register the Command Deck's operational panels (with zones). */
export function registerDeckPanels(): void {
  if (registered) return;
  registered = true;
  // Current Activity — the primary operational stream.
  commandDeckRegistry.register({ id: "intelligence-feed", title: "Live Intelligence Feed", region: "activity", render: () => <IntelligenceFeedPanel /> });
  // Running Operations — what KRIA is actively doing.
  commandDeckRegistry.register({ id: "agents", title: "Active Agents", region: "operations", render: () => <AgentsPanel /> });
  // Mission Status — system posture.
  commandDeckRegistry.register({ id: "overview", title: "AI Core Overview", region: "status", render: () => <OverviewPanel /> });
  // Upcoming — what's next.
  commandDeckRegistry.register({ id: "timeline", title: "Mission Timeline", region: "upcoming", render: () => <TimelinePanel /> });
}
