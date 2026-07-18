// KRIA token gate — Req 14.2 / design Property 6.
// Scans every production UI source file for raw colors and unresolved CSS vars.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(here, "..");
const generatedTokens = resolve(uiRoot, "src/styles/tokens.generated.css");

export const INCLUDE_DIRS = [
  "src/design-system",
  "src/kit",
  "src/shell",
  "src/palette",
  "src/prototypes",
];
export const INCLUDE_EXTENSIONS = [".ts", ".tsx", ".css"];
const EXCLUDE_FILE = /\.(test|spec|stories)\.(ts|tsx)$/;

const RAW_COLOR_PATTERNS = [
  { name: "hex-color", re: /#[0-9a-fA-F]{3,8}\b/g },
  { name: "rgb-color", re: /\brgba?\s*\(/gi },
  { name: "hsl-color", re: /\bhsla?\s*\(/gi },
];

function findingsForPattern(text, pattern, rule) {
  const findings = [];
  for (const [index, line] of text.split("\n").entries()) {
    pattern.lastIndex = 0;
    let match;
    while ((match = pattern.exec(line)) !== null) {
      findings.push({ line: index + 1, column: match.index + 1, match: match[0], rule });
    }
  }
  return findings;
}

/** Find hardcoded hex/rgb/hsl literals. No source-line bypasses are allowed. */
export function findRawColors(text) {
  return RAW_COLOR_PATTERNS.flatMap(({ name, re }) => findingsForPattern(text, re, name));
}

/** Collect CSS custom-property declarations from generated and local CSS. */
export function findTokenDefinitions(text) {
  const definitions = new Set();
  const re = /(^|[;{\s])(--[a-zA-Z0-9_-]+)\s*:/gm;
  let match;
  while ((match = re.exec(text)) !== null) definitions.add(match[2]);
  return definitions;
}

/** Find var(--token) references not present in the supplied declaration set. */
export function findUndefinedTokens(text, definedTokens) {
  const references = findingsForPattern(text, /var\(\s*(--[a-zA-Z0-9_-]+)/g, "token-reference");
  return references
    .map((finding) => ({ ...finding, match: finding.match.replace(/^var\(\s*/, "") }))
    .filter((finding) => !definedTokens.has(finding.match))
    .map((finding) => ({ ...finding, rule: "undefined-token" }));
}

function collectFiles(absDir) {
  if (!existsSync(absDir)) return [];
  const files = [];
  for (const entry of readdirSync(absDir, { withFileTypes: true })) {
    const full = join(absDir, entry.name);
    if (entry.isDirectory()) files.push(...collectFiles(full));
    else if (INCLUDE_EXTENSIONS.includes(extname(entry.name)) && !EXCLUDE_FILE.test(entry.name)) {
      files.push(full);
    }
  }
  return files;
}

export function lintTokenFiles(files) {
  const definitions = new Set();
  for (const file of files) {
    for (const token of findTokenDefinitions(readFileSync(file, "utf8"))) definitions.add(token);
  }

  const findings = [];
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    if (resolve(file) !== generatedTokens) {
      findings.push(...findRawColors(text).map((finding) => ({ file, ...finding })));
    }
    findings.push(...findUndefinedTokens(text, definitions).map((finding) => ({ file, ...finding })));
  }
  return findings;
}

function run() {
  const files = INCLUDE_DIRS.flatMap((directory) => collectFiles(resolve(uiRoot, directory)));
  // Generated CSS must exist: it is the authoritative design-token declaration set.
  if (!files.includes(generatedTokens)) files.push(generatedTokens);
  const findings = lintTokenFiles(files);

  for (const finding of findings) {
    const guidance = finding.rule === "undefined-token"
      ? "define it in tokens/base or tokens/themes"
      : "use a generated design token";
    console.error(
      `${relative(uiRoot, finding.file)}:${finding.line}:${finding.column}  ` +
      `${finding.rule} "${finding.match}" — ${guidance}`,
    );
  }

  if (findings.length > 0) {
    console.error(`\n✗ token-lint: ${findings.length} token purity violation(s).`);
    process.exitCode = 1;
    return;
  }
  console.log(`✓ token-lint: zero raw colors and zero undefined tokens in ${files.length} files.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) run();
