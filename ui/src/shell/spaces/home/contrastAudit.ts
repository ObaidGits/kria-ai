/**
 * contrastAudit — shared WCAG contrast primitives for the presence homepage
 * (design.md §15, Requirement 21.4).
 *
 * These pure helpers verify TEXT CONTRAST against the ACTUAL composited surface
 * the homepage paints on — the living-glass fills, the Reading-Mode backing, and
 * the Room gradient — rather than against a nominal token in isolation. That is
 * the "verify against real composited surfaces" rigor Req 21.4 mandates: a
 * translucent glass fill over the Room is what the user actually reads text on,
 * so contrast MUST be measured over that composited stack.
 *
 * Originally these primitives lived inside the Reading-Mode contrast test (task
 * 8.4). Task 10.1 extracts them here so BOTH the Reading-Mode AA property AND
 * the homepage-wide a11y AA property reuse ONE implementation (no drift, no
 * parallel system). Everything here is framework-free and DOM-free so it can be
 * driven by `fast-check` over the real generated token values.
 */

/** An sRGB color with a straight (non-premultiplied) alpha. Channels 0–255. */
export interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

/** Parse a hex (`#rgb` / `#rrggbb` / `#rrggbbaa`) or functional sRGB string. */
export function parseColor(value: string): Rgba {
  const hex = value.trim().match(/^#([0-9a-fA-F]{3,8})$/);
  if (hex) {
    let h = hex[1];
    if (h.length === 3) h = h.split("").map((c) => c + c).join("");
    return {
      r: parseInt(h.slice(0, 2), 16),
      g: parseInt(h.slice(2, 4), 16),
      b: parseInt(h.slice(4, 6), 16),
      a: h.length >= 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1,
    };
  }
  const rgb = value.match(
    /rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:[,\s/]+([\d.]+))?\s*\)/,
  );
  if (rgb) {
    return {
      r: Number(rgb[1]),
      g: Number(rgb[2]),
      b: Number(rgb[3]),
      a: rgb[4] !== undefined ? Number(rgb[4]) : 1,
    };
  }
  throw new Error(`unparseable color: ${value}`);
}

/** Alpha-composite `top` over an opaque `base` (source-over). Returns opaque. */
export function over(top: Rgba, base: Rgba): Rgba {
  const a = top.a;
  return {
    r: top.r * a + base.r * (1 - a),
    g: top.g * a + base.g * (1 - a),
    b: top.b * a + base.b * (1 - a),
    a: 1,
  };
}

/**
 * Composite an ordered list of translucent layers (back → front) over an
 * opaque base. `layers[0]` sits closest to the base; the last is on top. This
 * mirrors how the homepage paints text backings: e.g. `[glass-fill]` over the
 * Room base, or `[reading-dim, reading-backing]` over the receded Room.
 */
export function compositeOver(base: Rgba, layers: readonly Rgba[]): Rgba {
  let acc: Rgba = { ...base, a: 1 };
  for (const layer of layers) acc = over(layer, acc);
  return acc;
}

/** WCAG relative luminance for an opaque sRGB color (0–255 channels). */
export function relativeLuminance({ r, g, b }: Rgba): number {
  const lin = (c: number): number => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast ratio between two opaque colors (order-independent). */
export function contrastRatio(fg: Rgba, bg: Rgba): number {
  const l1 = relativeLuminance(fg);
  const l2 = relativeLuminance(bg);
  const [hi, lo] = l1 >= l2 ? [l1, l2] : [l2, l1];
  return (hi + 0.05) / (lo + 0.05);
}

/** WCAG 2.1 AA minimum contrast for normal-size body/caption text. */
export const AA_BODY = 4.5;

// ─── Generated-token parsing ─────────────────────────────────────────────────
// These read the REAL `tokens.generated.css` so a property can never drift from
// what ships. `:root` carries the dark defaults; `[data-theme="light"]` carries
// the light overrides.

/** Extract the declaration block for a top-level `selector { … }` rule. */
export function themeBlock(css: string, selector: string): string {
  const start = css.indexOf(selector);
  if (start === -1) throw new Error(`theme block not found: ${selector}`);
  const open = css.indexOf("{", start);
  const close = css.indexOf("}", open);
  return css.slice(open + 1, close);
}

/** Read a single `--token: value;` from a declaration block. */
export function tokenValue(block: string, name: string): string {
  const m = block.match(new RegExp(`${name}\\s*:\\s*([^;]+);`));
  if (!m) throw new Error(`token not found: ${name}`);
  return m[1].trim();
}

/** Convenience: parse a token straight from a theme block into an {@link Rgba}. */
export function tokenColor(block: string, name: string): Rgba {
  return parseColor(tokenValue(block, name));
}

/** The two shipped themes and the selector that carries each one's tokens. */
export const THEME_SELECTORS: ReadonlyArray<readonly [string, string]> = [
  ["dark", ":root"],
  ["light", '[data-theme="light"]'],
];
