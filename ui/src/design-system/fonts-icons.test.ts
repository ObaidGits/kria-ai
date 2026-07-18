/**
 * Guards the font self-hosting + Lucide sprite wiring (Req 14.6, 18.4).
 * Uses only Vite-graph imports (json / ?raw / glob) — no mocks, no node deps —
 * so it fails if the token font roles, the bundled woff2 binaries, or the
 * generated sprite drift out of sync.
 */
import { describe, it, expect } from "vitest";
import typeTokens from "../../tokens/base/type.json";
import iconManifest from "../../scripts/icon-manifest.json";
import spriteSvg from "../../public/icons/lucide-sprite.svg?raw";

// Bundled, self-hosted woff2 binaries (what the app actually ships).
const bundledWoff2 = import.meta.glob("../../public/fonts/*.woff2", { eager: true });
const bundledWoff2Names = Object.keys(bundledWoff2).map((p) => p.slice(p.lastIndexOf("/") + 1));

// Primary family = first quoted name in a font-family stack.
const primaryFamily = (stack: string) => (stack.match(/"([^"]+)"/)?.[1] ?? stack.split(",")[0]).trim();
// "Space Grotesk" -> "space-grotesk" (matches fontsource file slugs).
const slug = (family: string) => family.toLowerCase().replace(/\s+/g, "-");

describe("token font roles are self-hosted with bundled woff2", () => {
  const roles = (typeTokens as any).font.family as Record<"display" | "text" | "mono", { value: string }>;

  it("defines the three roles: display, text, mono", () => {
    expect(roles.display?.value).toBeTruthy();
    expect(roles.text?.value).toBeTruthy();
    expect(roles.mono?.value).toBeTruthy();
  });

  it.each(["display", "text", "mono"] as const)(
    "%s role's primary family has a bundled woff2 binary",
    (role) => {
      const family = primaryFamily(roles[role].value);
      const wanted = slug(family);
      const has = bundledWoff2Names.some((f) => f.startsWith(`${wanted}-latin-`) && f.endsWith(".woff2"));
      expect(has, `no bundled woff2 for role "${role}" family "${family}" (expected ${wanted}-latin-*.woff2)`).toBe(true);
    },
  );

  it("bundles the latin subset only (keeps payload small; non-latin documented)", () => {
    expect(bundledWoff2Names.length).toBeGreaterThan(0);
    for (const f of bundledWoff2Names) expect(f).toContain("-latin-");
  });
});

describe("Lucide sprite is generated and covers the manifest", () => {
  it("contains a <symbol> for every manifest icon", () => {
    expect(Array.isArray(iconManifest)).toBe(true);
    expect(spriteSvg).toContain("<symbol");
    for (const name of iconManifest as string[]) {
      expect(spriteSvg, `sprite missing symbol #${name}`).toContain(`<symbol id="${name}"`);
    }
  });
});
