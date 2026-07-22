/**
 * CcIcon — a tiny self-contained inline-SVG icon set for the Command Center.
 *
 * Deliberately independent of the app's sprite `Icon` so the Command Center is
 * fully self-contained (frontend-only demo). All icons stroke with
 * `currentColor` so the HUD theme controls their color via CSS.
 */
import type { JSX } from "solid-js";

/**
 * Each entry is a FACTORY that returns fresh glyph nodes on every call. This is
 * intentional: Solid moves (does not clone) reused JSX element instances, so a
 * shared node object would empty out an icon whenever the same name renders in
 * two places at once (e.g. dock + orbit). Factories guarantee every CcIcon gets
 * its own geometry — no missing/placeholder icons.
 */
const PATHS: Record<string, () => JSX.Element> = {
  grid: () => <><rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" /><rect x="3" y="14" width="7" height="7" /><rect x="14" y="14" width="7" height="7" /></>,
  core: () => <><circle cx="12" cy="12" r="4" /><circle cx="12" cy="12" r="9" /></>,
  agents: () => <><circle cx="12" cy="8" r="3" /><path d="M5 20c0-3.5 3-6 7-6s7 2.5 7 6" /></>,
  tasks: () => <><path d="M4 7h16M4 12h16M4 17h10" /><path d="M18 15l2 2 3-3" /></>,
  calendar: () => <><rect x="3" y="4" width="18" height="17" rx="2" /><path d="M3 9h18M8 2v4M16 2v4" /></>,
  memory: () => <><path d="M12 3a6 6 0 0 0-6 6v2a4 4 0 0 0 0 8h12a4 4 0 0 0 0-8V9a6 6 0 0 0-6-6Z" /><path d="M9 12h6" /></>,
  chat: () => <><path d="M4 5h16v11H8l-4 4V5Z" /></>,
  book: () => <><path d="M5 4h11a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3V4Z" /><path d="M5 17a3 3 0 0 1 3-3h11" /></>,
  tools: () => <><path d="M14 7a3 3 0 0 1 4 4l-8 8-4 1 1-4 8-8a3 3 0 0 1-1-1Z" /></>,
  flow: () => <><rect x="3" y="4" width="6" height="5" rx="1" /><rect x="15" y="15" width="6" height="5" rx="1" /><path d="M6 9v4a3 3 0 0 0 3 3h6" /></>,
  mic: () => <><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5 11a7 7 0 0 0 14 0M12 18v3" /></>,
  spark: () => <><path d="M12 3l2 6 6 2-6 2-2 6-2-6-6-2 6-2 2-6Z" /></>,
  shield: () => <><path d="M12 3l7 3v5c0 5-3 8-7 10-4-2-7-5-7-10V6l7-3Z" /></>,
  warn: () => <><path d="M12 3l10 18H2L12 3Z" /><path d="M12 10v5M12 18h.01" /></>,
  git: () => <><circle cx="6" cy="6" r="2.5" /><circle cx="6" cy="18" r="2.5" /><circle cx="18" cy="9" r="2.5" /><path d="M6 8.5v7M6 12h6a3 3 0 0 0 3-3" /></>,
  focus: () => <><circle cx="12" cy="12" r="3" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3" /></>,
  cpu: () => <><rect x="7" y="7" width="10" height="10" rx="1" /><path d="M10 2v3M14 2v3M10 19v3M14 19v3M2 10h3M2 14h3M19 10h3M19 14h3" /></>,
  code: () => <><path d="M9 8l-5 4 5 4M15 8l5 4-5 4" /></>,
  search: () => <><circle cx="11" cy="11" r="7" /><path d="M20 20l-3.5-3.5" /></>,
  globe: () => <><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18" /></>,
  plus: () => <><path d="M12 5v14M5 12h14" /></>,
  play: () => <><path d="M8 5l11 7-11 7V5Z" /></>,
  bell: () => <><path d="M6 9a6 6 0 0 1 12 0c0 5 2 6 2 6H4s2-1 2-6" /><path d="M10 20a2 2 0 0 0 4 0" /></>,
  gear: () => <><circle cx="12" cy="12" r="3" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2" /></>,
  wifi: () => <><path d="M2 8.5a15 15 0 0 1 20 0M5 12a10 10 0 0 1 14 0M8.5 15.5a5 5 0 0 1 7 0" /><path d="M12 19h.01" /></>,
  cloud: () => <><path d="M7 18a4 4 0 0 1 0-8 5 5 0 0 1 9.6-1.5A3.5 3.5 0 0 1 17 18H7Z" /></>,
  pin: () => <><path d="M12 2l2 7h5l-4 4 1.5 7L12 16l-4.5 4L9 13 5 9h5l2-7Z" /></>,
  user: () => <><circle cx="12" cy="8" r="3.5" /><path d="M5 20c0-3.5 3-6 7-6s7 2.5 7 6" /></>,
  send: () => <><path d="M4 12l16-8-6 16-3-6-7-2Z" /></>,
  check: () => <><path d="M4 12l5 5L20 6" /></>,
  chevron: () => <><path d="M9 6l6 6-6 6" /></>,
  brief: () => <><rect x="3" y="7" width="18" height="13" rx="2" /><path d="M8 7V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></>,
  waveform: () => <><path d="M3 12h2l2-6 3 14 3-18 3 14 2-4h3" /></>,
  bolt: () => <><path d="M13 2 4 14h6l-1 8 9-12h-6l1-8Z" /></>,
  hexlogo: () => <><path d="M12 2 20.66 7V17L12 22 3.34 17V7Z" /><path d="M12 8.2 15.2 13.5H8.8L12 8.2Z" /></>,
  sun: () => <><circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M19.1 4.9l-1.4 1.4M6.3 17.7l-1.4 1.4" /></>,
  sync: () => <><path d="M4 12a8 8 0 0 1 13.7-5.6L20 8M20 4v4h-4" /><path d="M20 12a8 8 0 0 1-13.7 5.6L4 16M4 20v-4h4" /></>,
  brain: () => <><path d="M9 4a3 3 0 0 0-3 3 3 3 0 0 0-1 5 3 3 0 0 0 2 4 3 3 0 0 0 5 1V4.5A2.5 2.5 0 0 0 9 4Z" /><path d="M15 4a3 3 0 0 1 3 3 3 3 0 0 1 1 5 3 3 0 0 1-2 4 3 3 0 0 1-5 1" /></>,
  clock: () => <><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></>,
  layers: () => <><path d="M12 3 3 8l9 5 9-5-9-5Z" /><path d="M3 13l9 5 9-5" /></>,
  monitor: () => <><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M8 20h8M12 16v4" /></>,
  activity: () => <><path d="M3 12h4l2.5 7 5-14 2.5 7h4" /></>,
};

export function CcIcon(props: { name: string; size?: number; class?: string }) {
  const size = () => props.size ?? 18;
  const glyph = () => (PATHS[props.name] ?? PATHS.core)();
  return (
    <svg
      class={`cc-icon ${props.class ?? ""}`.trim()}
      width={size()}
      height={size()}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {glyph()}
    </svg>
  );
}

export default CcIcon;
