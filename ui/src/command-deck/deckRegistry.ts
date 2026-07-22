/**
 * Command Deck registry — the composition seam that holds the deck's operational
 * panels. Panels are registered (with layout `region`s) in `registerDeckPanels`;
 * the Mission Control shell arranges them into its operational-flow zones.
 *
 * Extension point: register a new `SurfacePanelSpec` (id, title, region, render)
 * to add an operational panel — the shell places it by region automatically.
 */
import { createPanelRegistry } from "../app/panelRegistry";

export const commandDeckRegistry = createPanelRegistry();
