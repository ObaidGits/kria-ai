import type { OrbitCategory } from "./focusOrbitLayout";

export interface FocusOrbitSyntheticSample {
  id: string;
  label: string;
  category: OrbitCategory;
  cluster: number;
  ageDays: number;
  confidence: number;
  truthState: string;
  source: string;
  evidenceCount: number;
  relationDegree: number;
  score: number;
}

export interface FocusOrbitSyntheticSummary {
  nodeCount: number;
  edgeCount: number;
  clusterCount: number;
  bytes: number;
  mib: number;
  durationMs: number;
  checksum: string;
  seed: string;
  categoryTotals: Record<OrbitCategory, number>;
  workerHeld: true;
}

export type FocusOrbitSyntheticSamples = Record<OrbitCategory, FocusOrbitSyntheticSample[]>;

export type FocusOrbitSyntheticMessage =
  | { type: "progress"; phase: string; percent: number; detail: string }
  | { type: "complete"; summary: FocusOrbitSyntheticSummary; samples: FocusOrbitSyntheticSamples }
  | { type: "error"; message: string }
  | { type: "released" };
