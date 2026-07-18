/**
 * Space registry — maps each of the 7 Spaces (Req 1.2) to its component and
 * its Dock metadata (label + icon). Converse is eager (initial bundle,
 * design.md §2.3); the other six are lazily-loaded chunks so only the shell +
 * Converse are in the initial bundle (Req 16 startup budget).
 *
 * Requirements: 1.2, 16
 */
import { lazy, type Component } from "solid-js";
import type { Space } from "../router";
import ConverseSpace from "./ConverseSpace";

/** Dock metadata for each Space. */
export interface SpaceMeta {
  label: string;
  /** Lucide icon id present in the sprite. */
  icon: string;
}

export const SPACE_META: Record<Space, SpaceMeta> = {
  converse: { label: "Converse", icon: "message-circle" },
  memory: { label: "Memory", icon: "brain" },
  automations: { label: "Automations", icon: "workflow" },
  capabilities: { label: "Capabilities", icon: "sparkles" },
  machines: { label: "Machines", icon: "monitor" },
  observatory: { label: "Observatory", icon: "activity" },
  settings: { label: "Settings", icon: "settings" },
};

/**
 * Component registry. Converse is imported eagerly; the rest are lazy chunks
 * (code-split on first navigation to that Space).
 */
export const SPACE_COMPONENTS: Record<Space, Component> = {
  converse: ConverseSpace,
  memory: lazy(() => import("./MemorySpace")),
  automations: lazy(() => import("./AutomationsSpace")),
  capabilities: lazy(() => import("./CapabilitiesSpace")),
  machines: lazy(() => import("./MachinesSpace")),
  observatory: lazy(() => import("./ObservatorySpace")),
  settings: lazy(() => import("./SettingsSpace")),
};
