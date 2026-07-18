// One-component-per-concept audit — Req 14.4.
import { existsSync, readdirSync } from "node:fs";
import { dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(here, "..");

export const COMPONENT_CONCEPTS = [
  ["Button", "src/kit/Button.tsx"],
  ["Input", "src/kit/Input.tsx"],
  ["Card", "src/kit/Card.tsx"],
  ["Chip", "src/kit/Chip.tsx"],
  ["Badge", "src/kit/Badge.tsx"],
  ["StatusDot", "src/kit/StatusDot.tsx"],
  ["Row", "src/kit/Row.tsx"],
  ["Segment", "src/kit/SegmentBar.tsx"],
  ["Table", "src/kit/Table.tsx"],
  ["Inspector", "src/shell/InspectorHost.tsx"],
  ["ApprovalCard", "src/shell/approvals/ApprovalCard.tsx"],
  ["WorkBlock", "src/shell/spaces/converse/WorkBlock.tsx"],
  ["GraphNodeEdge", "src/shell/spaces/memory/graph/graphModel.ts"],
  ["Progress", "src/kit/Progress.tsx"],
  ["EmptyState", "src/kit/EmptyState.tsx"],
  ["Notification", "src/shell/notifications/NotificationCenter.tsx"],
  ["ModalConfirm", "src/kit/Confirm.tsx"],
  ["Wizard", "src/shell/spaces/machines/EnrollWizard.tsx"],
  ["ProvenanceCue", "src/kit/ProvenanceCue.tsx"],
];

function collectFiles(directory) {
  if (!existsSync(directory)) return [];
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const full = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...collectFiles(full));
    else if ([".ts", ".tsx"].includes(extname(entry.name)) && !/\.(test|stories)\.tsx?$/.test(entry.name)) files.push(full);
  }
  return files;
}

export function auditConcepts(manifest, options = {}) {
  const root = options.root ?? uiRoot;
  const fileExists = options.fileExists ?? existsSync;
  const candidateFiles = options.candidateFiles ?? [
    ...collectFiles(resolve(root, "src/kit")),
    ...collectFiles(resolve(root, "src/shell")),
  ];
  const issues = [];
  const seenConcepts = new Set();
  const seenPaths = new Set();

  for (const [concept, relativePath] of manifest) {
    if (seenConcepts.has(concept)) issues.push(`duplicate concept: ${concept}`);
    if (seenPaths.has(relativePath)) issues.push(`shared canonical path: ${relativePath}`);
    seenConcepts.add(concept);
    seenPaths.add(relativePath);

    const canonical = resolve(root, relativePath);
    if (!fileExists(canonical)) issues.push(`missing canonical component: ${concept} -> ${relativePath}`);
    const basename = relativePath.split("/").at(-1);
    for (const candidate of candidateFiles) {
      if (candidate.endsWith(`/${basename}`) && resolve(candidate) !== canonical) {
        issues.push(`duplicate ${concept} implementation: ${relative(root, candidate)}`);
      }
    }
  }

  return issues;
}

function run() {
  const issues = auditConcepts(COMPONENT_CONCEPTS);
  for (const issue of issues) console.error(`component-concept-audit: ${issue}`);
  if (issues.length > 0) {
    console.error(`\n✗ component-concept-audit: ${issues.length} violation(s).`);
    process.exitCode = 1;
    return;
  }
  console.log(`✓ component-concept-audit: ${COMPONENT_CONCEPTS.length} canonical concepts; no duplicates.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) run();
