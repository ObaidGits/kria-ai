/**
 * Developer Observatory registry — the seam future phases use to populate the
 * developer tooling surface (debug panels, logs, internal metrics, AI reasoning
 * inspection, memory inspection, provider diagnostics, performance analysis).
 *
 * Empty in Phase 1 (build the destination; do NOT migrate tools yet).
 */
import { createPanelRegistry } from "../app/panelRegistry";

export const developerRegistry = createPanelRegistry();
