/**
 * build-fonts.mjs — Self-host the display / text / mono fonts (Req 14.6, 18.4).
 *
 * Copies the pre-subset **latin** woff2 (+ woff fallback) files that ship inside
 * the @fontsource/* packages into `ui/public/fonts/`, so the app bundles and
 * serves its own fonts (no system-font / GTK-Qt-theme dependency, identical
 * rendering across GNOME/KDE). The matching @font-face declarations live in
 * `src/styles/fonts.css` and resolve the token font-family variables
 * (--font-family-display/text/mono) in `src/styles/tokens.generated.css`.
 *
 * Fonts are already latin-subset by fontsource; that keeps the bundle small.
 * Non-latin coverage (ar/zh/hi/…) is intentionally NOT bundled here — see the
 * NON-LATIN COVERAGE note in fonts.css. Run: `npm run fonts:build`.
 */
import { mkdirSync, copyFileSync, existsSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const uiRoot = join(__dirname, "..");
const outDir = join(uiRoot, "public", "fonts");

// [package, cssFamilyToken, weights] — token family names MUST match the
// --font-family-* values in tokens.generated.css exactly.
const FONTS = [
  { pkg: "@fontsource/space-grotesk", slug: "space-grotesk", weights: [400, 500, 600, 700] }, // display
  { pkg: "@fontsource/ibm-plex-sans", slug: "ibm-plex-sans", weights: [400, 500, 600, 700] }, // text
  { pkg: "@fontsource/jetbrains-mono", slug: "jetbrains-mono", weights: [400, 500, 700] }, // mono
];

const SUBSET = "latin";
const FORMATS = ["woff2", "woff"]; // woff2 primary, woff as a broad fallback

function main() {
  // Clean + recreate so removed weights don't linger.
  if (existsSync(outDir)) rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });

  let copied = 0;
  const missing = [];
  for (const { pkg, slug, weights } of FONTS) {
    const filesDir = join(uiRoot, "node_modules", pkg, "files");
    for (const w of weights) {
      for (const fmt of FORMATS) {
        const name = `${slug}-${SUBSET}-${w}-normal.${fmt}`;
        const src = join(filesDir, name);
        if (!existsSync(src)) {
          missing.push(`${pkg}/files/${name}`);
          continue;
        }
        copyFileSync(src, join(outDir, name));
        copied++;
      }
    }
  }

  if (missing.length) {
    console.warn(`[build-fonts] WARNING: ${missing.length} source file(s) not found:`);
    for (const m of missing) console.warn(`  - ${m}`);
    console.warn(
      "[build-fonts] Ensure devDependencies @fontsource/{space-grotesk,ibm-plex-sans,jetbrains-mono} are installed (`npm install`).",
    );
  }
  console.log(`[build-fonts] Wrote ${copied} font file(s) to public/fonts/`);
  if (copied === 0) process.exitCode = 1;
}

main();
