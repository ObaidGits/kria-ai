import type {
  FocusOrbitSyntheticMessage,
  FocusOrbitSyntheticSamples,
  FocusOrbitSyntheticSummary,
} from "./focusOrbitSyntheticProtocol";

const CATEGORY_IDS = ["knowledge", "goals", "skills", "events", "ideas", "people", "conversations", "projects"] as const;
const CATEGORY_NAMES = ["Knowledge", "Goals", "Skills", "Events", "Ideas", "People", "Conversations", "Projects"] as const;
const CATEGORY_WEIGHTS = [412, 18, 64, 74, 96, 58, 132, 24] as const;
const SOURCE_NAMES = ["conversation", "document", "observation", "tool-result", "user-stated"] as const;
const TRUTH_NAMES = ["asserted", "corroborated", "disputed"] as const;
const WEIGHT_TOTAL = 878;
let dataset: Record<string, ArrayBufferView> | null = null;

function post(message: FocusOrbitSyntheticMessage): void {
  self.postMessage(message);
}

function mix32(value: number): number {
  let mixed = value >>> 0;
  mixed = Math.imul(mixed ^ mixed >>> 16, 0x45d9f3b) >>> 0;
  mixed = Math.imul(mixed ^ mixed >>> 16, 0x45d9f3b) >>> 0;
  return (mixed ^ mixed >>> 16) >>> 0;
}

function createRng(seed: number): () => number {
  let value = seed >>> 0 || 0x4b524941;
  return () => {
    value ^= value << 13;
    value ^= value >>> 17;
    value ^= value << 5;
    return value >>> 0;
  };
}

function sendProgress(phase: string, percent: number, detail: string): void {
  post({ type: "progress", phase, percent, detail });
}

function categoryShape(count: number, clusterCount: number) {
  const totals = new Uint32Array(8);
  const starts = new Uint32Array(8);
  const ends = new Uint32Array(8);
  let cumulative = 0;
  let weight = 0;
  for (let category = 0; category < 8; category += 1) {
    starts[category] = cumulative;
    weight += CATEGORY_WEIGHTS[category];
    const next = category === 7 ? count : Math.floor(count * weight / WEIGHT_TOTAL);
    totals[category] = next - cumulative;
    ends[category] = next;
    cumulative = next;
  }
  const clusterTotals = new Uint16Array(8);
  let assigned = 0;
  for (let category = 0; category < 8; category += 1) {
    clusterTotals[category] = Math.max(1, Math.floor(clusterCount * totals[category] / count));
    assigned += clusterTotals[category];
  }
  while (assigned < clusterCount) { clusterTotals[assigned % 8] += 1; assigned += 1; }
  let cursor = 0;
  while (assigned > clusterCount) {
    const category = cursor % 8;
    if (clusterTotals[category] > 1) { clusterTotals[category] -= 1; assigned -= 1; }
    cursor += 1;
  }
  const clusterStarts = new Uint16Array(8);
  let base = 0;
  for (let category = 0; category < 8; category += 1) { clusterStarts[category] = base; base += clusterTotals[category]; }
  return { totals, starts, ends, clusterTotals, clusterStarts };
}

function generate(count: number, seed: number): void {
  const started = performance.now();
  const clusterCount = Math.max(100, Math.min(500, 100 + Math.floor(count / 2500)));
  const shape = categoryShape(count, clusterCount);
  sendProgress("allocating", 3, "Allocating compact per-node typed arrays");
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
  let categoryIndex = 0;
  const nodeStride = Math.max(10000, Math.floor(count / 12));

  for (let index = 0; index < count; index += 1) {
    while (index >= shape.ends[categoryIndex] && categoryIndex < 7) categoryIndex += 1;
    const hash = mix32(seed ^ index);
    const local = index - shape.starts[categoryIndex];
    category[index] = categoryIndex;
    cluster[index] = shape.clusterStarts[categoryIndex] + local % shape.clusterTotals[categoryIndex];
    ageDays[index] = hash % 211;
    confidence[index] = 140 + (hash >>> 8) % 116;
    truthState[index] = hash % 20 === 0 ? 2 : hash % 5 === 0 ? 1 : 0;
    source[index] = (hash >>> 16) % 5;
    evidenceCount[index] = 1 + (hash >>> 24) % 64;
    relationDegree[index] = 3 + mix32(hash ^ 0xa5a5a5a5) % 6;
    relationOffsets[index] = edgeCount;
    edgeCount += relationDegree[index];
    checksum = Math.imul(checksum ^ hash ^ cluster[index] ^ relationDegree[index], 0x01000193) >>> 0;
    if (index > 0 && index % nodeStride === 0) sendProgress("nodes", 8 + Math.floor(30 * index / count), `Generated ${index.toLocaleString()} compact records`);
  }
  relationOffsets[count] = edgeCount;
  sendProgress("relations", 40, `Allocating ${edgeCount.toLocaleString()} relation targets`);
  const relationTargets = new Uint32Array(edgeCount);
  const random = createRng(seed ^ count ^ 0x9e3779b9);
  const edgeStride = Math.max(250000, Math.floor(edgeCount / 12));
  let edge = 0;
  categoryIndex = 0;
  for (let index = 0; index < count; index += 1) {
    while (index >= shape.ends[categoryIndex] && categoryIndex < 7) categoryIndex += 1;
    const categoryStart = shape.starts[categoryIndex];
    const categorySize = shape.totals[categoryIndex];
    const localCluster = cluster[index] - shape.clusterStarts[categoryIndex];
    const clusters = shape.clusterTotals[categoryIndex];
    const clusterMembers = Math.floor((categorySize - 1 - localCluster) / clusters) + 1;
    for (let degree = 0; degree < relationDegree[index]; degree += 1, edge += 1) {
      const value = random();
      const bucket = value % 100;
      let target = bucket < 70
        ? categoryStart + localCluster + (value >>> 8) % clusterMembers * clusters
        : bucket < 90 ? categoryStart + (value >>> 8) % categorySize : (value >>> 8) % count;
      if (target === index) target = (target + 1) % count;
      relationTargets[edge] = target;
      checksum = Math.imul(checksum ^ target ^ edge, 0x01000193) >>> 0;
      if (edge > 0 && edge % edgeStride === 0) sendProgress("relations", 42 + Math.floor(50 * edge / edgeCount), `Linked ${edge.toLocaleString()} relations`);
    }
  }

  const samples = {} as FocusOrbitSyntheticSamples;
  for (let categoryIndex = 0; categoryIndex < CATEGORY_IDS.length; categoryIndex += 1) {
    const categoryId = CATEGORY_IDS[categoryIndex];
    const rows: FocusOrbitSyntheticSamples[typeof categoryId] = [];
    const sampleCount = Math.min(24, shape.totals[categoryIndex]);
    for (let sampleIndex = 0; sampleIndex < sampleCount; sampleIndex += 1) {
      const index = shape.starts[categoryIndex] + Math.min(
        shape.totals[categoryIndex] - 1,
        Math.floor((sampleIndex + 0.5) * shape.totals[categoryIndex] / sampleCount),
      );
      const clusterId = cluster[index];
      const normalizedConfidence = confidence[index] / 255;
      rows.push({
        id: `synthetic:${index}`,
        label: `Synthetic ${CATEGORY_NAMES[categoryIndex]} · cluster ${String(clusterId).padStart(3, "0")} · record ${String(index).padStart(7, "0")}`,
        category: categoryId,
        cluster: clusterId,
        ageDays: ageDays[index],
        confidence: normalizedConfidence,
        truthState: TRUTH_NAMES[truthState[index]],
        source: SOURCE_NAMES[source[index]],
        evidenceCount: evidenceCount[index],
        relationDegree: relationDegree[index],
        score: 0.45 + normalizedConfidence * 0.4 + (1 - ageDays[index] / 210) * 0.15,
      });
    }
    samples[categoryId] = rows.sort((left, right) => right.score - left.score);
  }

  dataset = { category, cluster, ageDays, confidence, truthState, source, evidenceCount, relationDegree, relationOffsets, relationTargets };
  const bytes = Object.values(dataset).reduce((sum, array) => sum + array.byteLength, 0);
  const categoryTotals = Object.fromEntries(
    CATEGORY_IDS.map((id, index) => [id, shape.totals[index]]),
  ) as FocusOrbitSyntheticSummary["categoryTotals"];
  const summary: FocusOrbitSyntheticSummary = {
    nodeCount: count,
    edgeCount,
    clusterCount,
    bytes,
    mib: Number((bytes / 1048576).toFixed(2)),
    durationMs: Math.round(performance.now() - started),
    checksum: `0x${checksum.toString(16).padStart(8, "0")}`,
    seed: `0x${(seed >>> 0).toString(16).padStart(8, "0")}`,
    categoryTotals,
    workerHeld: true,
  };
  sendProgress("complete", 100, "Compact graph retained in worker memory");
  post({ type: "complete", summary, samples });
}

self.onmessage = (event: MessageEvent<{ type: string; count?: number; seed?: number }>) => {
  if (event.data.type === "release") {
    dataset = null;
    post({ type: "released" });
    return;
  }
  if (event.data.type !== "generate") return;
  try {
    dataset = null;
    const count = Math.max(10000, Math.min(1000000, Number(event.data.count) || 10000));
    generate(count, Number(event.data.seed) >>> 0 || 0x4b524941);
  } catch (error) {
    dataset = null;
    post({ type: "error", message: error instanceof Error ? error.message : String(error) });
  }
};

export {};