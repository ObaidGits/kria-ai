/**
 * Developer Observatory population (Phase 2 relocation).
 *
 * Registers the diagnostic/engineering panels that moved OFF the homepage INTO
 * the Developer Observatory. Panel components are unchanged — moved, not
 * rewritten.
 */
import { developerRegistry } from "./devRegistry";
import { LlmStatusPanel, MemoryInsightsPanel, SystemMonitorPanel } from "../command-center/panels";

let registered = false;

/** Idempotently register the Developer Observatory's diagnostic tools. */
export function registerDeveloperPanels(): void {
  if (registered) return;
  registered = true;
  developerRegistry.register({ id: "system-monitor", title: "System Monitor", render: () => <SystemMonitorPanel /> });
  developerRegistry.register({ id: "llm-status", title: "LLM Status", render: () => <LlmStatusPanel /> });
  developerRegistry.register({ id: "memory-insights", title: "Memory Insights", render: () => <MemoryInsightsPanel /> });
}
