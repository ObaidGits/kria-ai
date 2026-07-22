/**
 * Panel registry — the relocatable widget set.
 *
 * Each panel is a self-contained, prop-free presentational module reading only
 * the static demo data. They are the units that later phases MOVE (not rewrite)
 * into the Command Deck or Developer Observatory. Grouped here by their intended
 * future destination so migration is a one-line change per phase.
 */
export { OverviewPanel } from "./OverviewPanel";
export { IntelligenceFeedPanel } from "./IntelligenceFeedPanel";
export { AgentsPanel } from "./AgentsPanel";
export { TimelinePanel } from "./TimelinePanel";
export { SystemMonitorPanel } from "./SystemMonitorPanel";
export { MemoryInsightsPanel } from "./MemoryInsightsPanel";
export { LlmStatusPanel } from "./LlmStatusPanel";

// Destinations are now wired directly at registration: the Command Deck
// arranges these panels into Mission Control zones (see
// `command-deck/registerDeckPanels`); the Developer Observatory registers the
// engineering panels (see `developer/registerDevPanels`).
