// KRIA homepage guardrail gate — Req 30.2 / 30.3 (release-blocking).
//
// Static counterpart to the runtime guardrails in
// `src/shell/spaces/home/guardrails.ts` and the published checklist in
// `.kiro/specs/homepage-presence-redesign/guardrails.md`. It enforces the two
// guardrails from task 0.5 that are cheap + reliable to check statically:
//
//   1. `coreHint` never written back → the Focus engine (and homeStore) must
//      NEVER call a coreStore mutator. `coreStore` is the sole authority for
//      Core state (Req 30.3, guardrails.md "Never … write back to coreStore").
//   2. No accent on the Room base → the emerald accent is reserved for the Core,
//      its pool, and meaningful state changes; it must never be spent on the
//      Room base surface (guardrails.md "Never … spend the emerald accent on
//      the Room base"; Req 16 accent discipline).
//
//   3. Awareness privacy model (Req 25.4/25.5) → the desktop-awareness modules
//      must declare no forbidden capture kind (keylogging, unconsented clipboard/
//      screen/file-history capture, scanning) as a source integration, and must
//      never perform network egress (awareness is processed all-local).
//
// The two runtime-only guardrails (single ACS, ≤3 chips) are enforced by
// `guardrails.ts` + its unit tests, since they depend on live FocusFrame values.
//
// Lightweight by design: it scans only the homepage store + component surface
// and passes vacuously while those files are still being built (additive gate).
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(here, "..");

// Files/dirs that must never write coreStore (Focus engine + homepage-local UI).
export const CORE_AUTHORITY_DIRS = ["src/shell/spaces/home"];
export const CORE_AUTHORITY_FILES = [
  "src/stores/homeStore.ts",
  "src/stores/homeFocusStore.ts",
];
// CSS surface scanned for accent-on-room-base.
export const ROOM_CSS_DIRS = ["src/shell/spaces/home"];
// Desktop-awareness surface scanned for privacy-model violations (Req 25.4/25.5):
// no forbidden capture kinds, no network egress (awareness stays all-local).
export const AWARENESS_FILES = [
  "src/stores/desktopAwarenessBridge.ts",
  "src/stores/awarenessPrivacy.ts",
  "src/shell/spaces/settings/AwarenessPanel.tsx",
];
// AI read-model surface (the Focus engine — "AI suggests, staged"). These files
// PRODUCE homepage AI outputs (Focus subjects, chips, starters, greeting,
// learned facts) and MUST stay suggestive: they may never override navigation
// (design §31, Req 29.1) nor auto-send/execute a suggestion (Req 29.3). Unlike
// presentation components (ContextualChips/PermissionSurface may route on an
// explicit user click), these read-model modules must not call `navigate`,
// `.send`, or `.execute` at all. Type-only `import type { Route }` is allowed.
export const AI_READMODEL_FILES = [
  "src/stores/homeFocusStore.ts",
  "src/stores/homeGreetingStore.ts",
  "src/stores/relationshipEvolution.ts",
];

const EXCLUDE_FILE = /\.(test|spec|stories)\.(ts|tsx)$/;

// ─── Detectors (pure; imported by the Vitest guard) ───────────────────────────

const CORE_MUTATORS = ["setState", "setBlocked", "setError", "goIdle", "reset", "ingest"];
const CORE_WRITE_RE = new RegExp(
  `\\bcoreStore\\s*\\.\\s*(${CORE_MUTATORS.join("|")})\\s*\\(`,
  "g",
);

/**
 * Find coreStore write-backs (any call to an authoritative mutator). Presence of
 * such a call in a Focus-engine / homepage-local file violates Req 30.3.
 */
export function findCoreStoreWrites(text) {
  const findings = [];
  for (const [index, line] of text.split("\n").entries()) {
    CORE_WRITE_RE.lastIndex = 0;
    let match;
    while ((match = CORE_WRITE_RE.exec(line)) !== null) {
      findings.push({ line: index + 1, column: match.index + 1, match: match[0], rule: "corehint-writeback" });
    }
  }
  return findings;
}

// A CSS rule whose selector references the Room base, e.g. `.kria-room__base`,
// `.kria-room` (bare base element), or a `--room-*` base custom property block.
const ROOM_BASE_SELECTOR_RE = /\.kria-room(__base|\b)[^{}]*\{([^{}]*)\}/g;
const ACCENT_TOKEN_RE = /--color-accent\b|--accent\b|--core-accent\b/;

/**
 * Find the emerald accent token used inside a Room-base CSS rule. The accent is
 * reserved for the Core/pool/state-change/focus — never the Room base surface.
 */
export function findAccentOnRoomBase(cssText) {
  const findings = [];
  ROOM_BASE_SELECTOR_RE.lastIndex = 0;
  let match;
  while ((match = ROOM_BASE_SELECTOR_RE.exec(cssText)) !== null) {
    const body = match[2] ?? "";
    if (ACCENT_TOKEN_RE.test(body)) {
      const upto = cssText.slice(0, match.index);
      const line = upto.split("\n").length;
      findings.push({ line, column: 1, match: match[0].split("{")[0].trim(), rule: "accent-on-room-base" });
    }
  }
  return findings;
}

// Forbidden desktop-awareness capture kinds (design §25.2 / Req 25.4). These must
// NEVER appear as a source `integration` value — KRIA does not keylog, capture the
// clipboard/screen-content/file-history/browsing-history without consent, record
// app usage, or scan. The privacy module allowlists local integrations; the lint
// catches a forbidden kind being smuggled in as an integration literal.
const FORBIDDEN_CAPTURE_KINDS = [
  "keylog",
  "keylogger",
  "keylogging",
  "clipboard-capture",
  "clipboard-scan",
  "screen-capture-content",
  "screen-content-capture",
  "screen-scrape",
  "screenshot-capture",
  "file-history",
  "file-history-capture",
  "browsing-history",
  "browser-history",
  "app-usage-history",
  "app-usage-recording",
  "usage-recording",
  "window-scan",
  "process-scan",
  "network-egress",
];
const FORBIDDEN_INTEGRATION_RE = new RegExp(
  `\\bintegration\\s*:\\s*["'\`](${FORBIDDEN_CAPTURE_KINDS.join("|")})["'\`]`,
  "g",
);

/**
 * Find a forbidden capture kind used as a source integration. Presence is a
 * privacy-model violation (Req 25.4): the desktop-awareness registry may only
 * register local allowlisted integrations, never a surveillance capture kind.
 */
export function findForbiddenCaptureKinds(text) {
  const findings = [];
  for (const [index, line] of text.split("\n").entries()) {
    FORBIDDEN_INTEGRATION_RE.lastIndex = 0;
    let match;
    while ((match = FORBIDDEN_INTEGRATION_RE.exec(line)) !== null) {
      findings.push({ line: index + 1, column: match.index + 1, match: match[0], rule: "forbidden-capture" });
    }
  }
  return findings;
}

// Outbound network primitives. Awareness is processed all-local (Req 25.5); the
// bridge maps local signals only and must never transmit them off the device.
const NETWORK_EGRESS_RE =
  /\b(fetch\s*\(|new\s+WebSocket\b|new\s+XMLHttpRequest\b|navigator\s*\.\s*sendBeacon\b|EventSource\b)/g;

/**
 * Find outbound-network primitives in a desktop-awareness module. Any hit breaks
 * the all-local invariant (Req 25.5): awareness data must never leave the device.
 */
export function findAwarenessNetworkEgress(text) {
  const findings = [];
  for (const [index, line] of text.split("\n").entries()) {
    NETWORK_EGRESS_RE.lastIndex = 0;
    let match;
    while ((match = NETWORK_EGRESS_RE.exec(line)) !== null) {
      findings.push({ line: index + 1, column: match.index + 1, match: match[0].trim(), rule: "awareness-network-egress" });
    }
  }
  return findings;
}

// Navigation override in the AI read-model (design §31, Req 29.1). The Focus
// engine may declare a routing target (a `route` chip carries a `Route`), but it
// must never EXECUTE navigation — that is the user's authority (palette/dock).
// A direct `navigate(` call in a read-model file is the override regression.
const NAV_OVERRIDE_RE = /\bnavigate\s*\(/g;

/**
 * Find navigation calls in an AI read-model module. Any hit means the Focus
 * engine is overriding navigation instead of leaving it to the user (Req 29.1).
 */
export function findNavOverrides(text) {
  const findings = [];
  for (const [index, line] of text.split("\n").entries()) {
    NAV_OVERRIDE_RE.lastIndex = 0;
    let match;
    while ((match = NAV_OVERRIDE_RE.exec(line)) !== null) {
      findings.push({ line: index + 1, column: match.index + 1, match: match[0].trim(), rule: "ai-nav-override" });
    }
  }
  return findings;
}

// AI auto-send/execute in the read-model (design §31, Req 29.3). Homepage AI
// outputs are staged/suggestive — the read-model must never send a message or
// execute a tool. Catches `<x>.send(` / `<x>.execute(` / `runTool(`/`invokeTool(`
// /`dispatchTool(` call sites that would auto-act without user review.
const AI_SEND_EXECUTE_RE =
  /\.\s*(send|execute|sendMessage|executeTool|runTool)\s*\(|\b(runTool|invokeTool|dispatchTool)\s*\(/g;

/**
 * Find auto-send/execute call sites in an AI read-model module. Any hit means a
 * home suggestion could act without explicit user review (Req 29.3 violation).
 */
export function findAiSendExecute(text) {
  const findings = [];
  for (const [index, line] of text.split("\n").entries()) {
    AI_SEND_EXECUTE_RE.lastIndex = 0;
    let match;
    while ((match = AI_SEND_EXECUTE_RE.exec(line)) !== null) {
      findings.push({ line: index + 1, column: match.index + 1, match: match[0].trim(), rule: "ai-send-execute" });
    }
  }
  return findings;
}

// ─── File collection ───────────────────────────────────────────────────────

function collectFiles(absDir, extensions) {
  if (!existsSync(absDir)) return [];
  const files = [];
  for (const entry of readdirSync(absDir, { withFileTypes: true })) {
    const full = join(absDir, entry.name);
    if (entry.isDirectory()) files.push(...collectFiles(full, extensions));
    else if (extensions.includes(extname(entry.name)) && !EXCLUDE_FILE.test(entry.name)) {
      files.push(full);
    }
  }
  return files;
}

export function lintGuardrailFiles({ authorityFiles, cssFiles, awarenessFiles = [], aiReadModelFiles = [] }) {
  const findings = [];
  for (const file of authorityFiles) {
    if (!existsSync(file)) continue;
    findings.push(...findCoreStoreWrites(readFileSync(file, "utf8")).map((f) => ({ file, ...f })));
  }
  for (const file of cssFiles) {
    findings.push(...findAccentOnRoomBase(readFileSync(file, "utf8")).map((f) => ({ file, ...f })));
  }
  for (const file of awarenessFiles) {
    if (!existsSync(file)) continue;
    const text = readFileSync(file, "utf8");
    findings.push(...findForbiddenCaptureKinds(text).map((f) => ({ file, ...f })));
    findings.push(...findAwarenessNetworkEgress(text).map((f) => ({ file, ...f })));
  }
  for (const file of aiReadModelFiles) {
    if (!existsSync(file)) continue;
    const text = readFileSync(file, "utf8");
    findings.push(...findNavOverrides(text).map((f) => ({ file, ...f })));
    findings.push(...findAiSendExecute(text).map((f) => ({ file, ...f })));
  }
  return findings;
}

function run() {
  const authorityFiles = [
    ...CORE_AUTHORITY_DIRS.flatMap((d) => collectFiles(resolve(uiRoot, d), [".ts", ".tsx"])),
    ...CORE_AUTHORITY_FILES.map((f) => resolve(uiRoot, f)),
  ];
  const cssFiles = ROOM_CSS_DIRS.flatMap((d) => collectFiles(resolve(uiRoot, d), [".css"]));
  const awarenessFiles = AWARENESS_FILES.map((f) => resolve(uiRoot, f));
  const aiReadModelFiles = AI_READMODEL_FILES.map((f) => resolve(uiRoot, f));

  const findings = lintGuardrailFiles({ authorityFiles, cssFiles, awarenessFiles, aiReadModelFiles });

  const GUIDANCE = {
    "corehint-writeback":
      "coreStore is the sole authority for Core state (Req 30.3); coreHint is advisory only — remove this write",
    "accent-on-room-base":
      "the emerald accent is reserved for the Core/pool/state-change/focus — never the Room base (guardrails.md)",
    "forbidden-capture":
      "desktop-awareness may only use local allowlisted integrations — never keylogging/clipboard/screen/file-history capture or scanning (Req 25.4)",
    "awareness-network-egress":
      "awareness is processed all-local and must never leave the device — remove this network call (Req 25.5)",
    "ai-nav-override":
      "the AI read-model (Focus engine) must never override navigation — navigation is the user's authority via palette/dock (design §31, Req 29.1)",
    "ai-send-execute":
      "homepage AI outputs are staged/suggestive — the read-model must never send/execute without explicit user review (design §31, Req 29.3)",
  };
  for (const finding of findings) {
    const guidance = GUIDANCE[finding.rule] ?? "homepage guardrail violation";
    console.error(
      `${relative(uiRoot, finding.file)}:${finding.line}:${finding.column}  ` +
        `${finding.rule} "${finding.match}" — ${guidance}`,
    );
  }

  if (findings.length > 0) {
    console.error(`\n✗ guardrail-lint: ${findings.length} homepage guardrail violation(s) — release blocker.`);
    process.exitCode = 1;
    return;
  }
  const scanned =
    authorityFiles.filter(existsSync).length +
    cssFiles.length +
    awarenessFiles.filter(existsSync).length +
    aiReadModelFiles.filter(existsSync).length;
  console.log(
    `✓ guardrail-lint: no coreStore write-backs, no accent-on-room-base, no ` +
      `awareness privacy violations (forbidden capture / network egress), and no ` +
      `AI authority violations (nav override / auto-send) in ${scanned} file(s).`,
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) run();
