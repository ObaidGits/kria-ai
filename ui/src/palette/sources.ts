/**
 * Palette source registry (Req 2.1, Req 21.4).
 *
 * Every entity type contributes searchable items through a `PaletteSource`.
 * Built-in sources read live store data (Spaces, settings, memories, workflows,
 * capabilities, models, threads, devices) plus the command + shortcut
 * registries. Later Spaces can `registerSource` to add their own without the
 * palette knowing about them. Sources for empty stores just return `[]`, so the
 * palette works from day one and lights up as stores are populated.
 *
 * Go mode  = navigation sources (space/setting/memory/workflow/capability/
 *            model/thread/device).
 * Do mode  = command + shortcut sources (runnable actions).
 * Ask/Change are free-text modes handled in dispatch.ts (no item sources).
 */
import {
  converseStore,
  memoryStore,
  automationStore,
  capabilityStore,
  machineStore,
  settingsStore,
} from "../stores";
import { navigate, ALL_SPACES } from "../shell/router";
import { SPACE_META } from "../shell/spaces";
import type { CapabilitySegment } from "../stores";
import type { PaletteItem, PaletteMode, PaletteSource } from "./types";
import { listCommands } from "./commands";
import { listShortcuts, formatKeys } from "./shortcuts";

// ─── Custom source registry ─────────────────────────────────────────────────

const customSources = new Map<string, PaletteSource>();

/** Register (or replace) a custom source. Returns an unregister function. */
export function registerSource(source: PaletteSource): () => void {
  customSources.set(source.id, source);
  return () => {
    if (customSources.get(source.id) === source) customSources.delete(source.id);
  };
}

/** Clear custom sources (tests). Built-in sources are unaffected. */
export function clearCustomSources(): void {
  customSources.clear();
}

// ─── Built-in Go sources ─────────────────────────────────────────────────────

const spacesSource: PaletteSource = {
  id: "spaces",
  modes: ["go"],
  getItems: () =>
    ALL_SPACES.map((space) => {
      const meta = SPACE_META[space];
      return {
        id: `space:${space}`,
        type: "space",
        title: meta.label,
        subtitle: "Space",
        icon: meta.icon,
        keywords: space,
        run: () => navigate(space),
      } satisfies PaletteItem;
    }),
};

const settingsSource: PaletteSource = {
  id: "settings",
  modes: ["go"],
  getItems: () =>
    settingsStore.schema().map((s) => ({
      id: `setting:${s.key}`,
      type: "setting" as const,
      title: s.label,
      subtitle: s.description ?? `Setting · ${s.group}`,
      icon: "settings",
      keywords: `${s.key} ${s.group}`,
      run: () => navigate("settings", s.group, s.key),
    })),
};

const memoriesSource: PaletteSource = {
  id: "memories",
  modes: ["go"],
  getItems: () =>
    memoryStore.facts().map((f) => ({
      id: `memory:${f.id}`,
      type: "memory" as const,
      title: f.content,
      subtitle: `Memory · ${f.source}`,
      icon: "brain",
      keywords: f.tags.join(" "),
      run: () => navigate("memory", "explorer", f.id),
    })),
};

const workflowsSource: PaletteSource = {
  id: "workflows",
  modes: ["go"],
  getItems: () =>
    automationStore.workflows().map((w) => ({
      id: `workflow:${w.id}`,
      type: "workflow" as const,
      title: w.name,
      subtitle: w.description || "Workflow",
      icon: "workflow",
      run: () => navigate("automations", "run", w.id),
    })),
};

/** Map a capability descriptor to the Capabilities segment that hosts it. */
function capabilitySegment(type: string): CapabilitySegment {
  switch (type) {
    case "skill":
      return "skills";
    case "model":
      return "models";
    case "integration":
      return "integrations";
    default:
      return "tools";
  }
}

const capabilitiesSource: PaletteSource = {
  id: "capabilities",
  modes: ["go"],
  getItems: () =>
    capabilityStore.capabilities().map((c) => ({
      id: `capability:${c.id}`,
      type: "capability" as const,
      title: c.name,
      subtitle: c.description || `Capability · ${c.type}`,
      icon: "sparkles",
      keywords: `${c.type} ${c.source}`,
      run: () => navigate("capabilities", capabilitySegment(c.type), c.id),
    })),
};

const modelsSource: PaletteSource = {
  id: "models",
  modes: ["go"],
  getItems: () =>
    capabilityStore.providers().map((p) => ({
      id: `model:${p.id}`,
      type: "model" as const,
      title: p.name,
      subtitle: `${p.type} provider${p.active ? " · active" : ""}`,
      icon: "cpu",
      run: () => navigate("capabilities", "models", p.id),
    })),
};

const threadsSource: PaletteSource = {
  id: "threads",
  modes: ["go"],
  getItems: () =>
    converseStore.threads().map((t) => ({
      id: `thread:${t.id}`,
      type: "thread" as const,
      title: t.title || "Untitled thread",
      subtitle: "Conversation",
      icon: "message-circle",
      run: () => {
        converseStore.setActiveThread(t.id);
        navigate("converse");
      },
    })),
};

const devicesSource: PaletteSource = {
  id: "devices",
  modes: ["go"],
  getItems: () =>
    machineStore.devices().map((d) => ({
      id: `device:${d.id}`,
      type: "device" as const,
      title: d.name,
      subtitle: `${d.type} · ${d.os} · ${d.status}`,
      icon: "monitor",
      keywords: d.ip ?? "",
      run: () => navigate("machines", "device", d.id),
    })),
};

// ─── Built-in Do sources ─────────────────────────────────────────────────────

const commandsSource: PaletteSource = {
  id: "commands",
  modes: ["do"],
  getItems: () =>
    listCommands().map((c) => ({
      id: c.id,
      type: "command" as const,
      title: c.title,
      subtitle: c.subtitle,
      icon: c.icon ?? "zap",
      keywords: c.keywords,
      shortcutHint: c.shortcutHint,
      run: c.run,
    })),
};

const shortcutsSource: PaletteSource = {
  id: "shortcuts",
  modes: ["do"],
  getItems: () =>
    listShortcuts().map((s) => ({
      id: s.id,
      type: "shortcut" as const,
      title: s.label,
      subtitle: "Keyboard shortcut",
      icon: s.icon ?? "command",
      keywords: s.keywords,
      shortcutHint: formatKeys(s.keys),
      run: () => s.run?.(),
    })),
};

const BUILT_IN_SOURCES: readonly PaletteSource[] = [
  spacesSource,
  settingsSource,
  memoriesSource,
  workflowsSource,
  capabilitiesSource,
  modelsSource,
  threadsSource,
  devicesSource,
  commandsSource,
  shortcutsSource,
];

/**
 * Collect all items contributing to `mode` from built-in + custom sources.
 */
export function collectItems(mode: PaletteMode): PaletteItem[] {
  const items: PaletteItem[] = [];
  const sources = [...BUILT_IN_SOURCES, ...customSources.values()];
  for (const source of sources) {
    if (!source.modes.includes(mode)) continue;
    items.push(...source.getItems());
  }
  return items;
}
