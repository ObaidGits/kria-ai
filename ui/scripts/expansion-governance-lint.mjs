// UI future-expansion governance gate — Requirements 21.1-21.4.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { basename, dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(here, "..");

export const DOCK_LIMIT = 7;
export const CANONICAL_SPACES = [
  "converse", "memory", "automations", "capabilities",
  "machines", "observatory", "settings",
];
export const CORE_STATES = new Set([
  "idle", "listening", "thinking", "planning", "speaking", "acting",
  "running-automation", "watching", "remembering", "reflecting",
  "learning", "waiting", "blocked", "error", "recovering",
]);
const FEATURE_KINDS = new Set(["mode", "lens", "capability"]);

function unique(values) {
  return [...new Set(values)];
}

export function extractQuotedArray(source, declaration) {
  const match = source.match(new RegExp(`(?:const|type)\\s+${declaration}[^=]*=\\s*\\[([\\s\\S]*?)\\]`));
  if (!match) return [];
  return [...match[1].matchAll(/["']([a-z][a-z0-9-]*)["']/g)].map((item) => item[1]);
}

export function auditDock(spaces) {
  const issues = [];
  if (spaces.length > DOCK_LIMIT) issues.push(`Dock has ${spaces.length} Spaces; limit is ${DOCK_LIMIT}`);
  if (unique(spaces).length !== spaces.length) issues.push("Dock contains duplicate Spaces");
  const missing = CANONICAL_SPACES.filter((space) => !spaces.includes(space));
  const unknown = spaces.filter((space) => !CANONICAL_SPACES.includes(space));
  if (missing.length) issues.push(`missing canonical Spaces: ${missing.join(", ")}`);
  if (unknown.length) issues.push(`unapproved top-level Spaces: ${unknown.join(", ")}`);
  return issues;
}

function collectSource(directory) {
  if (!existsSync(directory)) return "";
  return readdirSync(directory, { withFileTypes: true }).map((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return collectSource(path);
    return [".ts", ".tsx"].includes(extname(entry.name)) ? readFileSync(path, "utf8") : "";
  }).join("\n");
}

export function validateFeatureDescriptor(descriptor, source, pathExists = existsSync, featureDir = "") {
  const issues = [];
  const name = descriptor?.id ?? "<unknown>";
  const fail = (message) => issues.push(`${name}: ${message}`);
  if (!/^[a-z][a-z0-9-]*$/.test(descriptor?.id ?? "")) fail("id must be kebab-case");
  if (!FEATURE_KINDS.has(descriptor?.kind)) fail("kind must be mode, lens, or capability");
  if (!CANONICAL_SPACES.includes(descriptor?.space)) fail("space must be an existing canonical Space");
  if (descriptor?.home !== "calm") fail('home must be "calm"');
  if (descriptor?.componentKit !== "shared") fail('componentKit must be "shared"');
  if (!Array.isArray(descriptor?.coreStates) || descriptor.coreStates.length === 0) {
    fail("coreStates must declare at least one canonical Core state");
  } else {
    for (const state of descriptor.coreStates) if (!CORE_STATES.has(state)) fail(`unknown Core state: ${state}`);
  }

  for (const field of ["entry", "paletteSource"]) {
    if (typeof descriptor?.[field] !== "string" || !descriptor[field]) fail(`${field} is required`);
    else if (!pathExists(resolve(featureDir, descriptor[field]))) fail(`${field} does not exist: ${descriptor[field]}`);
  }

  if (!/from\s+["'][^"']*\/kit(?:["'/])/.test(source)) fail("feature source must import shared kit components");
  if (!/\b(coreStore|CoreState)\b/.test(source)) fail("feature source must consume canonical Core state language");
  if (!/\b(registerSource|PaletteSource)\b/.test(source)) fail("feature must register a Command Palette source");

  if (descriptor?.approval === "approval-center") {
    if (!/\b(approvalStore|ApprovalCenter)\b/.test(source)) fail("approval-center policy lacks shared Approval Center evidence");
  } else if (descriptor?.approval === "not-required") {
    if (!descriptor.approvalReason?.trim()) fail("not-required approval needs approvalReason");
  } else fail("approval must be approval-center or not-required");

  if (descriptor?.inspector === "shared-inspector") {
    if (!/\b(registerInspectorRenderer|openInspector|InspectorHost)\b/.test(source)) fail("shared-inspector policy lacks shared Inspector evidence");
  } else if (descriptor?.inspector === "not-required") {
    if (!descriptor.inspectorReason?.trim()) fail("not-required inspector needs inspectorReason");
  } else fail("inspector must be shared-inspector or not-required");

  return issues;
}

export function auditRepository(root = uiRoot) {
  const issues = [];
  const routerPath = resolve(root, "src/shell/router.ts");
  const spacesPath = resolve(root, "src/shell/spaces/index.ts");
  const palettePath = resolve(root, "src/palette/sources.ts");
  const paletteTypesPath = resolve(root, "src/palette/types.ts");
  const router = existsSync(routerPath) ? readFileSync(routerPath, "utf8") : "";
  const spaces = extractQuotedArray(router, "ALL_SPACES");
  issues.push(...auditDock(spaces));

  const spaceRegistry = existsSync(spacesPath) ? readFileSync(spacesPath, "utf8") : "";
  for (const space of CANONICAL_SPACES) {
    const registrations = spaceRegistry.match(new RegExp(`\\b${space}\\s*:`, "g"))?.length ?? 0;
    if (registrations < 2) issues.push(`${space} must exist in SPACE_META and SPACE_COMPONENTS`);
  }

  const spaceFilesDir = resolve(root, "src/shell/spaces");
  const spaceFiles = existsSync(spaceFilesDir)
    ? readdirSync(spaceFilesDir).filter((name) => /^[A-Z][A-Za-z]+Space\.tsx$/.test(name))
    : [];
  const fileSpaces = spaceFiles.map((name) => basename(name, "Space.tsx").toLowerCase());
  for (const issue of auditDock(fileSpaces)) issues.push(`Space files: ${issue}`);

  const palette = existsSync(palettePath) ? readFileSync(palettePath, "utf8") : "";
  if (!/ALL_SPACES\.map\s*\(/.test(palette)) issues.push("palette must derive Space entries from ALL_SPACES");
  if (!/export function registerSource\s*\(/.test(palette)) issues.push("palette registerSource extension seam is missing");
  if (/\b(bridgeInvoke|tauriBridge|invoke|fetch)\s*\(/.test(palette)) issues.push("palette source performs direct execution; it may only navigate or submit intent");
  const paletteTypes = existsSync(paletteTypesPath) ? readFileSync(paletteTypesPath, "utf8") : "";
  const normalizedPaletteTypes = paletteTypes.replace(/\n\s*\*\s?/g, " ").replace(/\s+/g, " ");
  if (!normalizedPaletteTypes.includes("never a direct tool/capability execution")) issues.push("PaletteItem authority guard is missing");

  const seams = [
    "src/kit/index.ts", "src/stores/coreStore.ts", "src/stores/approvalStore.ts",
    "src/shell/approvals/ApprovalCenter.tsx", "src/shell/InspectorHost.tsx",
    "src/shell/inspectorRegistry.ts",
  ];
  for (const seam of seams) if (!existsSync(resolve(root, seam))) issues.push(`missing shared architecture seam: ${seam}`);

  const featuresRoot = resolve(root, "src/features");
  if (existsSync(featuresRoot)) {
    for (const entry of readdirSync(featuresRoot, { withFileTypes: true }).filter((item) => item.isDirectory())) {
      const featureDir = resolve(featuresRoot, entry.name);
      const descriptorPath = resolve(featureDir, "feature.governance.json");
      if (!existsSync(descriptorPath)) {
        issues.push(`${entry.name}: missing feature.governance.json`);
        continue;
      }
      try {
        const descriptor = JSON.parse(readFileSync(descriptorPath, "utf8"));
        if (descriptor.id !== entry.name) issues.push(`${entry.name}: descriptor id must match directory name`);
        const source = collectSource(featureDir);
        issues.push(...validateFeatureDescriptor(descriptor, source, existsSync, featureDir));
        const paletteSource = descriptor.paletteSource ? resolve(featureDir, descriptor.paletteSource) : "";
        if (paletteSource && existsSync(paletteSource)) {
          const paletteText = readFileSync(paletteSource, "utf8");
          if (/\b(bridgeInvoke|tauriBridge|invoke|fetch)\s*\(/.test(paletteText)) issues.push(`${entry.name}: palette source may not execute substrates/tools directly`);
        }
      } catch (error) {
        issues.push(`${entry.name}: invalid feature.governance.json (${error.message})`);
      }
    }
  }
  return issues;
}

function run() {
  const issues = auditRepository();
  for (const issue of issues) console.error(`expansion-governance-lint: ${issue}`);
  if (issues.length) {
    console.error(`\n✗ expansion-governance-lint: ${issues.length} violation(s).`);
    process.exitCode = 1;
  } else {
    console.log("✓ expansion-governance-lint: 7-Space cap, shared patterns, Calm home, palette reachability, and authority boundaries hold.");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) run();
