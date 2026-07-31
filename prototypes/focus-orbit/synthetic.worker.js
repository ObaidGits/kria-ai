/* Throwaway Focus Orbit worker: compact deterministic synthetic KRIA-like graph. */
"use strict";
function syntheticWorkerRuntime(self) {
const CATEGORY_IDS = ["knowledge", "goals", "skills", "events", "ideas", "people", "conversations", "projects"];
const CATEGORY_NAMES = ["Knowledge", "Goals", "Skills", "Events", "Ideas", "People", "Conversations", "Projects"];
const CATEGORY_WEIGHTS = [412, 18, 64, 74, 96, 58, 132, 24];
const SOURCE_NAMES = ["conversation", "document", "observation", "tool-result", "user-stated"];
const TRUTH_NAMES = ["asserted", "corroborated", "disputed"];
const WEIGHT_TOTAL = 878;
let DATASET = null;

function mix32(value) {
  let x = value >>> 0;
  x = Math.imul(x ^ (x >>> 16), 0x45d9f3b) >>> 0;
  x = Math.imul(x ^ (x >>> 16), 0x45d9f3b) >>> 0;
  return (x ^ (x >>> 16)) >>> 0;
}

function makeRng(seed) {
  let x = seed >>> 0 || 0x4b524941;
  return () => {
    x ^= x << 13; x ^= x >>> 17; x ^= x << 5;
    return x >>> 0;
  };
}

function progress(phase, percent, detail) {
  self.postMessage({ type: "progress", phase, percent, detail });
}

function categoryShape(count, clusterCount) {
  const totals = new Uint32Array(8);
  const starts = new Uint32Array(8);
  const ends = new Uint32Array(8);
  let cumulative = 0;
  for (let c = 0; c < 8; c++) {
    starts[c] = cumulative;
    const next = c === 7 ? count : Math.floor(count * CATEGORY_WEIGHTS.slice(0, c + 1).reduce((a, b) => a + b, 0) / WEIGHT_TOTAL);
    totals[c] = next - cumulative; ends[c] = next; cumulative = next;
  }
  const clusterTotals = new Uint16Array(8);
  let assigned = 0;
  for (let c = 0; c < 8; c++) {
    clusterTotals[c] = Math.max(1, Math.floor(clusterCount * totals[c] / count));
    assigned += clusterTotals[c];
  }
  while (assigned < clusterCount) { clusterTotals[assigned % 8]++; assigned++; }
  let cursor = 0;
  while (assigned > clusterCount) {
    const c = cursor++ % 8;
    if (clusterTotals[c] > 1) { clusterTotals[c]--; assigned--; }
  }
  const clusterStarts = new Uint16Array(8);
  let clusterBase = 0;
  for (let c = 0; c < 8; c++) { clusterStarts[c] = clusterBase; clusterBase += clusterTotals[c]; }
  return { totals, starts, ends, clusterTotals, clusterStarts };
}

function generate(count, seed) {
  const started = performance.now();
  const clusterCount = Math.max(100, Math.min(500, 100 + Math.floor(count / 2500)));
  const shape = categoryShape(count, clusterCount);
  progress("allocating", 3, "Allocating compact per-node typed arrays");

  const category = new Uint8Array(count);
  const cluster = new Uint16Array(count);
  const ageDays = new Uint8Array(count);
  const confidence = new Uint8Array(count);
  const truthState = new Uint8Array(count);
  const source = new Uint8Array(count);
  const evidenceCount = new Uint8Array(count);
  const relationDegree = new Uint8Array(count);
  const relationOffsets = new Uint32Array(count + 1);
  let checksum = 0x811c9dc5;
  let edgeCount = 0;
  let cat = 0;
  const nodeStride = Math.max(10000, Math.floor(count / 12));

  for (let i = 0; i < count; i++) {
    while (i >= shape.ends[cat] && cat < 7) cat++;
    const h = mix32(seed ^ i);
    const local = i - shape.starts[cat];
    category[i] = cat;
    cluster[i] = shape.clusterStarts[cat] + (local % shape.clusterTotals[cat]);
    ageDays[i] = h % 211;
    confidence[i] = 140 + ((h >>> 8) % 116);
    truthState[i] = (h % 20 === 0) ? 2 : (h % 5 === 0 ? 1 : 0);
    source[i] = (h >>> 16) % 5;
    evidenceCount[i] = 1 + ((h >>> 24) % 64);
    relationDegree[i] = 3 + (mix32(h ^ 0xa5a5a5a5) % 6);
    relationOffsets[i] = edgeCount;
    edgeCount += relationDegree[i];
    checksum = Math.imul(checksum ^ h ^ cluster[i] ^ relationDegree[i], 0x01000193) >>> 0;
    if (i && i % nodeStride === 0) progress("nodes", 8 + Math.floor(30 * i / count), `Generated ${i.toLocaleString()} compact records`);
  }
  relationOffsets[count] = edgeCount;
  progress("relations", 40, `Allocating ${edgeCount.toLocaleString()} relation targets`);
  const relationTargets = new Uint32Array(edgeCount);
  const random = makeRng(seed ^ count ^ 0x9e3779b9);
  const edgeStride = Math.max(250000, Math.floor(edgeCount / 12));
  let edge = 0;
  cat = 0;

  for (let i = 0; i < count; i++) {
    while (i >= shape.ends[cat] && cat < 7) cat++;
    const catStart = shape.starts[cat];
    const catSize = shape.totals[cat];
    const localCluster = cluster[i] - shape.clusterStarts[cat];
    const clustersInCat = shape.clusterTotals[cat];
    const membersInCluster = Math.floor((catSize - 1 - localCluster) / clustersInCat) + 1;
    for (let d = 0; d < relationDegree[i]; d++, edge++) {
      const r = random();
      const bucket = r % 100;
      let target;
      if (bucket < 70) {
        target = catStart + localCluster + ((r >>> 8) % membersInCluster) * clustersInCat;
      } else if (bucket < 90) {
        target = catStart + ((r >>> 8) % catSize);
      } else {
        target = (r >>> 8) % count;
      }
      if (target === i) target = (target + 1) % count;
      relationTargets[edge] = target;
      checksum = Math.imul(checksum ^ target ^ edge, 0x01000193) >>> 0;
      if (edge && edge % edgeStride === 0) progress("relations", 42 + Math.floor(50 * edge / edgeCount), `Linked ${edge.toLocaleString()} relations`);
    }
  }

  progress("sampling", 94, "Building bounded representative UI samples");
  const samples = {};
  for (let c = 0; c < 8; c++) {
    const rows = [];
    const sampleCount = Math.min(24, shape.totals[c]);
    for (let j = 0; j < sampleCount; j++) {
      const index = shape.starts[c] + Math.min(shape.totals[c] - 1, Math.floor((j + 0.5) * shape.totals[c] / sampleCount));
      const clusterId = cluster[index];
      rows.push({
        id: `synthetic:${index}`,
        name: `Synthetic ${CATEGORY_NAMES[c]} · cluster ${String(clusterId).padStart(3, "0")} · record ${String(index).padStart(7, "0")}`,
        cat: CATEGORY_IDS[c], cluster: clusterId, days: ageDays[index],
        confidence: confidence[index] / 255, truth: TRUTH_NAMES[truthState[index]],
        source: SOURCE_NAMES[source[index]], evidence: evidenceCount[index],
        degree: relationDegree[index], relevance: 0.45 + (confidence[index] / 255) * 0.4 + (1 - ageDays[index] / 210) * 0.15
      });
    }
    samples[CATEGORY_IDS[c]] = rows.sort((a, b) => b.relevance - a.relevance);
  }

  DATASET = { category, cluster, ageDays, confidence, truthState, source, evidenceCount, relationDegree, relationOffsets, relationTargets };
  const bytes = Object.values(DATASET).reduce((sum, array) => sum + array.byteLength, 0);
  const durationMs = Math.round(performance.now() - started);
  const summary = {
    nodeCount: count, edgeCount, clusterCount, bytes, mib: +(bytes / 1048576).toFixed(2), durationMs,
    checksum: "0x" + checksum.toString(16).padStart(8, "0"), seed: "0x" + (seed >>> 0).toString(16).padStart(8, "0"),
    categoryTotals: Object.fromEntries(CATEGORY_IDS.map((id, c) => [id, shape.totals[c]])), workerHeld: true
  };
  progress("complete", 100, "Compact graph retained in worker memory");
  self.postMessage({ type: "complete", summary, samples });
}

self.onmessage = event => {
  if (event.data?.type !== "generate") return;
  try {
    DATASET = null;
    const count = Math.max(10000, Math.min(1000000, Number(event.data.count) || 10000));
    generate(count, Number(event.data.seed) >>> 0 || 0x4b524941);
  } catch (error) {
    DATASET = null;
    self.postMessage({ type: "error", message: error?.message || String(error) });
  }
};
}

if (typeof WorkerGlobalScope !== "undefined" && self instanceof WorkerGlobalScope) {
  syntheticWorkerRuntime(self);
}
