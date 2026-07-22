/** Runtime mapping for WCAG preferences received from KRIA config authority. */
export const FONT_SCALE_STEPS = ["0.8", "0.9", "1.0", "1.2", "1.5", "2.0"] as const;
export type FontScaleStep = (typeof FONT_SCALE_STEPS)[number];

export interface AccessibilityPreferences {
  highContrast: boolean;
  reducedMotion: boolean;
  /**
   * Steady-lighting preference (Req 1.4 / 21.4). When on, the homepage disables
   * the time-of-day undertone shift so the Room's ambient tone never drifts.
   * Purely a mood/atmosphere control — it carries no data meaning.
   */
  steadyLighting: boolean;
  fontScale: FontScaleStep;
}

export function normalizeFontScale(value: unknown): FontScaleStep {
  const parsed = Number.parseFloat(String(value ?? "1"));
  if (!Number.isFinite(parsed)) return "1.0";
  return FONT_SCALE_STEPS.reduce<FontScaleStep>((nearest, candidate) =>
    Math.abs(Number(candidate) - parsed) < Math.abs(Number(nearest) - parsed)
      ? candidate
      : nearest,
  "1.0");
}

export function accessibilityPreferences(settings: Record<string, unknown>): AccessibilityPreferences {
  const ui = settings.ui && typeof settings.ui === "object" && !Array.isArray(settings.ui)
    ? settings.ui as Record<string, unknown>
    : {};
  return {
    highContrast: ui.high_contrast === true,
    reducedMotion: ui.reduce_motion === true,
    steadyLighting: ui.steady_lighting === true,
    fontScale: normalizeFontScale(ui.font_scale),
  };
}

export function applyAccessibilityPreferences(
  settings: Record<string, unknown>,
  root: HTMLElement | undefined = typeof document === "undefined" ? undefined : document.documentElement,
): AccessibilityPreferences {
  const preferences = accessibilityPreferences(settings);
  if (!root) return preferences;
  const osReduced = typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-reduced-motion: reduce)").matches
    : false;
  root.dataset.highContrast = String(preferences.highContrast);
  root.dataset.fontScale = preferences.fontScale;
  root.dataset.reduceMotion = String(preferences.reducedMotion);
  root.dataset.reducedMotion = preferences.reducedMotion || osReduced ? "on" : "off";
  // Homepage time-of-day undertone reads this attribute to disable itself
  // (Req 1.4 / 21.4). Mirrors the `data-high-contrast` boolean convention.
  root.dataset.steadyLighting = String(preferences.steadyLighting);
  return preferences;
}
