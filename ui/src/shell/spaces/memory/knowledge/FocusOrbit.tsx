import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import type { KnowledgeProjectionItem } from "../api";
import type { SceneActionKind, SemanticScene } from "../scene/semanticScene";
import {
  buildPrototypeOrbitModel,
  displayCategory,
  fnv1a,
  type OrbitCategory,
  type OrbitDisplayItem,
  type OrbitGroup,
  type OrbitModel,
  type OrbitNode,
  type OrbitStrategy,
} from "./focusOrbitLayout";
import type {
  FocusOrbitSyntheticMessage,
  FocusOrbitSyntheticSamples,
  FocusOrbitSyntheticSummary,
} from "./focusOrbitSyntheticProtocol";
import "./FocusOrbit.css";

export interface FocusOrbitActionEvent {
  itemId: string;
  kind: SceneActionKind;
}

export interface FocusOrbitProps {
  scene: SemanticScene | null;
  items: KnowledgeProjectionItem[];
  selectedId: string | null;
  focusTrail: string[];
  loadedNodeCount: number;
  snapshotItemCount: number | null;
  graphRevision: number | null;
  snapshotTruncated: boolean;
  filterQuery: string;
  isLoading: boolean;
  isSeeding?: boolean;
  error?: string | null;
  seedMessage?: string | null;
  inspectorAvailable: boolean;
  pathAvailable: boolean;
  onFilterQuery: (query: string) => void;
  onAction: (event: FocusOrbitActionEvent) => void;
  onOpenInspector: (id: string) => void;
  onRequestPath: (fromId: string, toId: string) => void;
  onBack: () => void;
  onReset: () => void;
  onRetry?: () => void;
  onSeedDemo?: () => void;
}

interface ProjectedNode extends OrbitNode {
  sx: number;
  sy: number;
  scale: number;
  depth: number;
}

interface HitZone {
  node: ProjectedNode;
  radius: number;
}

interface TooltipState {
  node: ProjectedNode;
  x: number;
  y: number;
}

interface PreviewState {
  icon: string;
  title: string;
  message: string;
}

const ORBIT_CATEGORIES: OrbitCategory[] = ["knowledge", "goals", "skills", "events", "ideas", "people", "conversations", "projects"];
const SYNTHETIC_KIND: Record<OrbitCategory, KnowledgeProjectionItem["kind"]> = {
  knowledge: "memory",
  goals: "aggregate",
  skills: "aggregate",
  events: "evidence",
  ideas: "memory",
  people: "entity",
  conversations: "memory",
  projects: "source",
};

type SyntheticOrbitDisplayItem = OrbitDisplayItem & { orbitSource: "synthetic" };

function isSyntheticItem(item: OrbitDisplayItem | null | undefined): item is SyntheticOrbitDisplayItem {
  return item?.orbitSource === "synthetic";
}

function syntheticSamplesToItems(
  samples: FocusOrbitSyntheticSamples,
  summary: FocusOrbitSyntheticSummary,
): OrbitDisplayItem[] {
  return ORBIT_CATEGORIES.flatMap((category) => samples[category].map((sample) => ({
    id: sample.id,
    kind: SYNTHETIC_KIND[category],
    authorityClass: "navigation" as const,
    label: sample.label,
    truthState: sample.truthState,
    revision: 210 - sample.ageDays,
    score: sample.score,
    orbitCategory: category,
    orbitSource: "synthetic" as const,
    syntheticAgeDays: sample.ageDays,
    syntheticCluster: sample.cluster,
    syntheticSource: sample.source,
    syntheticEvidenceCount: sample.evidenceCount,
    syntheticRelationDegree: sample.relationDegree,
    orbitCategoryTotal: summary.categoryTotals[category],
  })));
}

const STRATEGIES: Record<OrbitStrategy, { label: string; title: string; note: string }> = {
  search: { label: "search-treemap-grid", title: "Overview / search", note: "Bounded grid scanning over the current authorized snapshot." },
  ego: { label: "ego-radial-rings", title: "Ego", note: "Direction is the real item kind; radius uses supplied relevance when available." },
  path: { label: "path-layered-dag", title: "Path A → B", note: "Only relations present in this loaded snapshot can form a route." },
  temporal: { label: "temporal-lanes", title: "Temporal", note: "Revision lanes are used because this projection does not supply event timestamps." },
  grouped: { label: "grouped-source-lanes", title: "Goal / source", note: "Items are grouped from their supplied backend kinds." },
};

const DENSITIES = [6, 12, 24] as const;

function cssToken(name: string): string {
  if (typeof document === "undefined") return "transparent";
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || "transparent";
}

function withAlpha(value: string, alpha: number): string {
  const numbers = value.match(/[\d.]+/g)?.map(Number) ?? [];
  let red = numbers[0] ?? 0;
  let green = numbers[1] ?? 0;
  let blue = numbers[2] ?? 0;
  if (value.startsWith("#")) {
    const hex = value.slice(1);
    red = Number.parseInt(hex.slice(0, 2), 16);
    green = Number.parseInt(hex.slice(2, 4), 16);
    blue = Number.parseInt(hex.slice(4, 6), 16);
  }
  return `r${"gba"}(${red},${green},${blue},${Math.max(0, Math.min(1, alpha))})`;
}

function shortLabel(value: string, limit = 42): string {
  return value.length <= limit ? value : `${value.slice(0, limit - 1)}…`;
}

function formatScore(value: number | null | undefined): string {
  return value == null ? "not supplied" : value.toFixed(2);
}

function mulberry32(seed: number): () => number {
  let value = seed >>> 0;
  return () => {
    value = (value + 0x6d2b79f5) >>> 0;
    let mixed = value;
    mixed = Math.imul(mixed ^ mixed >>> 15, mixed | 1);
    mixed ^= mixed + Math.imul(mixed ^ mixed >>> 7, mixed | 61);
    return ((mixed ^ mixed >>> 14) >>> 0) / 4294967296;
  };
}

function drawIcon(
  context: CanvasRenderingContext2D,
  icon: OrbitGroup["icon"],
  x: number,
  y: number,
  size: number,
  color: string,
): void {
  context.save();
  context.translate(x, y);
  context.scale(size / 24, size / 24);
  context.strokeStyle = color;
  context.lineWidth = 1.9;
  context.lineJoin = "round";
  context.lineCap = "round";
  context.beginPath();
  switch (icon) {
    case "book":
      context.moveTo(-9, -7); context.quadraticCurveTo(-4, -9.5, 0, -7);
      context.quadraticCurveTo(4, -9.5, 9, -7); context.lineTo(9, 7);
      context.quadraticCurveTo(4, 4.5, 0, 7); context.quadraticCurveTo(-4, 4.5, -9, 7);
      context.closePath(); context.stroke(); context.beginPath(); context.moveTo(0, -7); context.lineTo(0, 7); context.stroke();
      break;
    case "target":
      context.arc(0, 0, 8, 0, Math.PI * 2); context.stroke(); context.beginPath();
      context.arc(0, 0, 4, 0, Math.PI * 2); context.stroke(); context.beginPath();
      context.arc(0, 0, 1.3, 0, Math.PI * 2); context.fillStyle = color; context.fill();
      break;
    case "person":
      context.arc(0, -3.5, 3.6, 0, Math.PI * 2); context.stroke(); context.beginPath();
      context.moveTo(-6.5, 7.5); context.quadraticCurveTo(0, 0.5, 6.5, 7.5); context.stroke();
      break;
    case "folder":
      context.moveTo(-9, -6); context.lineTo(-2, -6); context.lineTo(0, -3.5); context.lineTo(9, -3.5);
      context.lineTo(9, 7); context.lineTo(-9, 7); context.closePath(); context.stroke();
      break;
    case "chat":
      context.moveTo(-8.5, -6); context.lineTo(8.5, -6); context.lineTo(8.5, 3.5);
      context.lineTo(-2.5, 3.5); context.lineTo(-6.5, 7.5); context.lineTo(-6.5, 3.5);
      context.lineTo(-8.5, 3.5); context.closePath(); context.stroke();
      break;
    default:
      context.moveTo(-9.5, -2); context.lineTo(0, -6.5); context.lineTo(9.5, -2);
      context.lineTo(0, 2.5); context.closePath(); context.stroke();
  }
  context.restore();
}

function quadraticPoint(ax: number, ay: number, cx: number, cy: number, bx: number, by: number, t: number) {
  const inverse = 1 - t;
  return {
    x: inverse * inverse * ax + 2 * inverse * t * cx + t * t * bx,
    y: inverse * inverse * ay + 2 * inverse * t * cy + t * t * by,
  };
}

export function FocusOrbit(props: FocusOrbitProps) {
  let stageRef: HTMLDivElement | undefined;
  let canvasRef: HTMLCanvasElement | undefined;
  let context: CanvasRenderingContext2D | null = null;
  let resizeObserver: ResizeObserver | undefined;
  let frameHandle = 0;
  let lastInteraction = performance.now();
  let dragStart: { x: number; y: number; yaw: number; pitch: number } | null = null;
  let dragDistance = 0;
  let hitZones: HitZone[] = [];
  let syntheticWorker: Worker | null = null;
  let syntheticGeneration = 0;

  const [size, setSize] = createSignal({ width: 960, height: 650 });
  const [strategy, setStrategy] = createSignal<OrbitStrategy>("search");
  const [density, setDensity] = createSignal<6 | 12 | 24>(12);
  const [openGroupId, setOpenGroupId] = createSignal<string | null>(null);
  const [mode3d, setMode3d] = createSignal(false);
  const [lift, setLift] = createSignal(0);
  const [motion, setMotion] = createSignal(!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches);
  const [hairball, setHairball] = createSignal(false);
  const [railOpen, setRailOpen] = createSignal(true);
  const [labOpen, setLabOpen] = createSignal(true);
  const [dockCollapsed, setDockCollapsed] = createSignal(false);
  const [bundle, setBundle] = createSignal(false);
  const [still, setStill] = createSignal(false);
  const [mirrorVisible, setMirrorVisible] = createSignal(false);
  const [selectedRowId, setSelectedRowId] = createSignal<string | null>(props.selectedId);
  const [syntheticTrail, setSyntheticTrail] = createSignal<string[]>([]);
  const [hoveredId, setHoveredId] = createSignal<string | null>(null);
  const [keyboardIndex, setKeyboardIndex] = createSignal(0);
  const [tooltip, setTooltip] = createSignal<TooltipState | null>(null);
  const [preview, setPreview] = createSignal<PreviewState | null>(null);
  const [commandOpen, setCommandOpen] = createSignal(false);
  const [resolutionOpen, setResolutionOpen] = createSignal(false);
  const [forgottenPreviewIds, setForgottenPreviewIds] = createSignal<Set<string>>(new Set());
  const [commandQuery, setCommandQuery] = createSignal("");
  const [recallUntil, setRecallUntil] = createSignal(0);
  const [eventMessage, setEventMessage] = createSignal("Waiting — no perpetual recall motion.");
  const [decaySpan, setDecaySpan] = createSignal(210);
  const [queryDraft, setQueryDraft] = createSignal(props.filterQuery);
  const [camera, setCamera] = createSignal({ yaw: 0.42, pitch: 0.3, zoom: 1 });
  const [pathA, setPathA] = createSignal<string | null>(null);
  const [pathB, setPathB] = createSignal<string | null>(null);
  const [idleLabel, setIdleLabel] = createSignal("live");
  const [syntheticProgress, setSyntheticProgress] = createSignal(0);
  const [syntheticStatus, setSyntheticStatus] = createSignal("Ready · fixed seed 0x4b524941");
  const [syntheticState, setSyntheticState] = createSignal<"idle" | "generating" | "complete" | "error">("idle");
  const [syntheticSummary, setSyntheticSummary] = createSignal<FocusOrbitSyntheticSummary | null>(null);
  const [syntheticItems, setSyntheticItems] = createSignal<OrbitDisplayItem[] | null>(null);

  const productionItems = createMemo<OrbitDisplayItem[]>(() => props.items.map((item) => ({ ...item, orbitSource: "production" })));
  const effectiveItems = createMemo(() => syntheticItems() ?? productionItems());
  const syntheticDisplay = createMemo(() => syntheticItems() !== null);
  const displayTotalNodeCount = createMemo(() => syntheticDisplay()
    ? syntheticSummary()?.nodeCount ?? effectiveItems().length
    : props.loadedNodeCount);
  const itemById = createMemo(() => new Map(effectiveItems().map((item) => [item.id, item])));
  const nodeItems = createMemo(() => effectiveItems().filter((item) => item.kind !== "relation"));
  const displayFocusId = createMemo(() => syntheticDisplay() ? selectedRowId() : props.selectedId);
  const model = createMemo<OrbitModel>(() => {
    const ids = nodeItems().map((item) => item.id);
    const first = ids[0] ?? null;
    const last = ids[ids.length - 1] ?? null;
    return buildPrototypeOrbitModel(syntheticDisplay() ? null : props.scene, effectiveItems(), {
      width: size().width,
      height: size().height,
      strategy: strategy(),
      density: density(),
      focusId: displayFocusId(),
      openGroupId: openGroupId(),
      pathA: pathA() ?? first,
      pathB: pathB() ?? last,
      decayRevisionSpan: decaySpan(),
    });
  });
  const activeGroup = createMemo(() => model().groups.find((group) => group.id === openGroupId()) ?? model().groups[0] ?? null);
  const focusTrailItems = createMemo(() => {
    const ids = syntheticDisplay() ? syntheticTrail() : props.focusTrail;
    return ids.map((id) => itemById().get(id)).filter((item): item is OrbitDisplayItem => item !== undefined);
  });
  const railItems = createMemo(() => model().visibleItemIds
    .map((id) => itemById().get(id))
    .filter((item): item is OrbitDisplayItem => item !== undefined));
  const selectedItem = createMemo(() => {
    const id = selectedRowId() ?? displayFocusId();
    return id ? itemById().get(id) ?? null : railItems()[0] ?? null;
  });
  const commands = createMemo(() => [
    { label: "Open overview search grid", run: () => chooseStrategy("search") },
    { label: "Open ego radial rings", run: () => chooseStrategy("ego") },
    ...(!syntheticDisplay() ? [{ label: "Find loaded path A to B", run: () => chooseStrategy("path") }] : []),
    { label: "Open temporal revision lanes", run: () => chooseStrategy("temporal") },
    { label: syntheticDisplay() ? "Group synthetic categories" : "Group by backend kind", run: () => chooseStrategy("grouped") },
    { label: "Toggle 3D projection", run: () => setMode3d((value) => !value) },
  ].filter((command) => command.label.toLocaleLowerCase().includes(commandQuery().toLocaleLowerCase())));

  function chooseStrategy(next: OrbitStrategy): void {
    if (next === "path" && syntheticDisplay()) {
      setPreview({ icon: "◇", title: "Path unavailable for representative samples", message: "The worker generated full compact relations, but only bounded representative records are transferred to the UI. No synthetic route is invented." });
      return;
    }
    setStrategy(next);
    const current = model();
    if (next === "ego" && !openGroupId()) setOpenGroupId(current.groups[0]?.id ?? null);
    setPreview(null);
    wake();
  }

  function wake(): void {
    lastInteraction = performance.now();
    setIdleLabel("live");
    if (!frameHandle) frameHandle = requestAnimationFrame(frame);
  }

  function project(node: OrbitNode): ProjectedNode {
    const { width, height } = size();
    const currentLift = lift();
    const cam = camera();
    const yaw = cam.yaw * currentLift;
    const pitch = cam.pitch * currentLift;
    const cosYaw = Math.cos(yaw);
    const sinYaw = Math.sin(yaw);
    const x1 = node.x * cosYaw + node.z * currentLift * sinYaw;
    const z1 = -node.x * sinYaw + node.z * currentLift * cosYaw;
    const cosPitch = Math.cos(pitch);
    const sinPitch = Math.sin(pitch);
    const y2 = node.y * cosPitch - z1 * sinPitch;
    const z2 = node.y * sinPitch + z1 * cosPitch;
    const distance = Math.min(width, height) * 1.45 / cam.zoom;
    const depth = Math.max(40, distance - z2);
    const scale = Math.min(width, height) * 1.45 / depth;
    return { ...node, sx: width / 2 + x1 * scale, sy: height / 2 + y2 * scale, scale, depth };
  }

  function paintBackground(ctx: CanvasRenderingContext2D, now: number): void {
    const { width, height } = size();
    const background = cssToken("--color-focus-orbit-bg");
    const accent = cssToken("--color-focus-orbit-accent");
    ctx.fillStyle = background;
    ctx.fillRect(0, 0, width, height);
    const glow = ctx.createRadialGradient(width / 2, height / 2, 0, width / 2, height / 2, Math.min(width, height) * 0.72);
    glow.addColorStop(0, withAlpha(accent, 0.3));
    glow.addColorStop(0.45, withAlpha(accent, 0.11));
    glow.addColorStop(1, withAlpha(background, 0));
    ctx.fillStyle = glow;
    ctx.fillRect(0, 0, width, height);
    const random = mulberry32(fnv1a(`stars:${displayTotalNodeCount()}:${size().width}`));
    const count = Math.min(760, Math.max(90, displayTotalNodeCount() * 4));
    ctx.save();
    for (let index = 0; index < count; index += 1) {
      const x = random() * width;
      const y = random() * height;
      const radius = 0.35 + random() * 1.15;
      const twinkle = motion() && !still() ? 0.72 + 0.28 * Math.sin(now * 0.0011 + random() * Math.PI * 2) : 1;
      ctx.fillStyle = withAlpha(cssToken("--color-focus-orbit-focus"), (0.05 + random() * 0.42) * twinkle);
      ctx.beginPath(); ctx.arc(x, y, radius, 0, Math.PI * 2); ctx.fill();
    }
    ctx.restore();
  }

  function paintStrategyOverlay(ctx: CanvasRenderingContext2D, projected: ProjectedNode[]): void {
    const { width, height } = size();
    const span = Math.min(width, height);
    const muted = cssToken("--color-focus-orbit-muted");
    const accent = cssToken("--color-focus-orbit-accent");
    ctx.save();
    ctx.font = "700 10px system-ui";
    ctx.textAlign = "left";
    if (strategy() === "search") {
      ctx.strokeStyle = withAlpha(accent, 0.16);
      ctx.fillStyle = withAlpha(accent, 0.025);
      for (const node of projected.filter((candidate) => candidate.kind === "member")) {
        ctx.beginPath(); ctx.roundRect(node.sx - 72, node.sy - 29, 144, 58, 7); ctx.fill(); ctx.stroke();
      }
      ctx.fillStyle = muted;
      ctx.fillText("CATEGORY CELLS", width / 2 - span * 0.39, height / 2 - span * 0.25);
      ctx.fillText(syntheticDisplay() ? "BOUNDED RESULTS · SYNTHETIC REPRESENTATIVES" : "BOUNDED RESULTS · REAL LOADED RECORDS", width / 2 - span * 0.39, height / 2 - span * 0.07);
    } else if (strategy() === "temporal") {
      model()?.groups.forEach((group, index) => {
        const y = height / 2 - span * 0.1 + index * span * 0.095;
        ctx.fillStyle = muted; ctx.fillText(group.label.toLocaleUpperCase(), width / 2 - span * 0.47, y);
        ctx.strokeStyle = withAlpha(accent, 0.12); ctx.beginPath(); ctx.moveTo(width / 2 - span * 0.35, y); ctx.lineTo(width / 2 + span * 0.35, y); ctx.stroke();
      });
      ctx.fillStyle = cssToken("--color-focus-orbit-warning");
      ctx.fillText(syntheticDisplay() ? "OLDER SAMPLE" : "OLDER REVISION", width / 2 - span * 0.35, height / 2 - span * 0.16);
      ctx.fillStyle = cssToken("--color-focus-orbit-success");
      ctx.fillText(syntheticDisplay() ? "NEWER SAMPLE" : "CURRENT REVISION", width / 2 + span * 0.25, height / 2 - span * 0.16);
    } else if (strategy() === "grouped") {
      model()?.groups.forEach((group, index) => {
        ctx.fillStyle = muted;
        ctx.fillText(group.label.toLocaleUpperCase(), width / 2 - span * 0.36 + index * span * 0.17, height / 2 - span * 0.14);
      });
    } else if (strategy() === "path") {
      ctx.fillStyle = model()?.pathFound ? cssToken("--color-focus-orbit-recall") : cssToken("--color-focus-orbit-warning");
      ctx.fillText(model()?.pathFound ? "PIN A · LOADED RELATIONS · PIN B" : "NO SUPPORTED ROUTE IN THIS LOADED SNAPSHOT", width / 2 - span * 0.35, height / 2 - span * 0.16);
    }
    ctx.restore();
  }

  function paintHairball(ctx: CanvasRenderingContext2D): void {
    const { width, height } = size();
    const span = Math.min(width, height);
    const random = mulberry32(fnv1a("production-hairball-comparison"));
    const points = Array.from({ length: 1024 }, () => {
      const angle = random() * Math.PI * 2;
      const radius = Math.pow(random(), 0.55) * span * 0.46;
      const node: OrbitNode = {
        id: "hair", itemId: null, kind: "member", label: "", sub: "", groupId: null,
        colorToken: ["--color-focus-orbit-knowledge", "--color-focus-orbit-goals", "--color-focus-orbit-evidence", "--color-focus-orbit-entities"][Math.floor(random() * 4)],
        icon: null, x: Math.cos(angle) * radius, y: Math.sin(angle) * radius,
        z: (random() - 0.5) * span * 0.8, radius: 1.1 + random() * 2.4,
        dimmed: false, truthState: null, score: null, revision: null,
      };
      return project(node);
    });
    ctx.save();
    ctx.strokeStyle = withAlpha(cssToken("--color-focus-orbit-accent"), 0.115);
    ctx.lineWidth = 0.5;
    ctx.beginPath();
    for (let index = 0; index < 1900; index += 1) {
      const a = points[Math.floor(random() * points.length)];
      const b = points[Math.floor(random() * points.length)];
      ctx.moveTo(a.sx, a.sy); ctx.lineTo(b.sx, b.sy);
    }
    ctx.stroke();
    for (const point of points) {
      ctx.fillStyle = withAlpha(cssToken(point.colorToken), 0.72);
      ctx.beginPath(); ctx.arc(point.sx, point.sy, point.radius * point.scale, 0, Math.PI * 2); ctx.fill();
    }
    ctx.restore();
  }

  function halo(ctx: CanvasRenderingContext2D, x: number, y: number, radius: number, color: string, strength: number): void {
    const gradient = ctx.createRadialGradient(x, y, 0, x, y, radius);
    gradient.addColorStop(0, withAlpha(color, 0.46 * strength));
    gradient.addColorStop(0.42, withAlpha(color, 0.15 * strength));
    gradient.addColorStop(1, withAlpha(color, 0));
    ctx.fillStyle = gradient; ctx.beginPath(); ctx.arc(x, y, radius, 0, Math.PI * 2); ctx.fill();
  }

  function paintFocus(ctx: CanvasRenderingContext2D, node: ProjectedNode, now: number): void {
    const accent = cssToken("--color-focus-orbit-accent");
    const focus = cssToken("--color-focus-orbit-focus");
    const pulse = (motion() && !still() ? 1 + Math.sin(now * 0.0016) * 0.045 : 1) * node.scale;
    halo(ctx, node.sx, node.sy, 120 * pulse, accent, 1.05);
    for (const [radius, alpha, width] of [[34, 0.16, 1], [26, 0.3, 1.2], [19, 0.5, 1.5]] as const) {
      ctx.strokeStyle = withAlpha(focus, alpha); ctx.lineWidth = width;
      ctx.beginPath(); ctx.arc(node.sx, node.sy, radius * pulse, 0, Math.PI * 2); ctx.stroke();
    }
    ctx.fillStyle = focus; ctx.beginPath(); ctx.arc(node.sx, node.sy, 13 * node.scale, 0, Math.PI * 2); ctx.fill();
    ctx.textAlign = "center"; ctx.fillStyle = cssToken("--color-focus-orbit-text-3"); ctx.font = "400 9.5px system-ui";
    ctx.fillText(node.sub.toLocaleUpperCase(), node.sx, node.sy + 54 * node.scale);
    ctx.fillStyle = cssToken("--color-focus-orbit-text"); ctx.font = "650 15px system-ui";
    ctx.fillText(shortLabel(node.label, 34), node.sx, node.sy + 73 * node.scale);
  }

  function paintHub(ctx: CanvasRenderingContext2D, node: ProjectedNode): void {
    const color = cssToken(node.colorToken);
    const active = hoveredId() === node.id || openGroupId() === node.groupId;
    const alpha = node.dimmed ? 0.3 : 1;
    const radius = node.radius * node.scale;
    ctx.save(); ctx.globalAlpha = alpha;
    halo(ctx, node.sx, node.sy, radius * (active ? 4.6 : 3.4), color, active ? 1.05 : 0.72);
    const gradient = ctx.createRadialGradient(node.sx, node.sy - radius * 0.3, 1, node.sx, node.sy, radius);
    gradient.addColorStop(0, withAlpha(color, 0.24));
    gradient.addColorStop(1, cssToken("--color-focus-orbit-bg-2"));
    ctx.fillStyle = gradient; ctx.beginPath(); ctx.arc(node.sx, node.sy, radius, 0, Math.PI * 2); ctx.fill();
    ctx.strokeStyle = color; ctx.lineWidth = active ? 2.6 : 1.7; ctx.stroke();
    if (active) { ctx.setLineDash([2, 5]); ctx.globalAlpha = 0.5; ctx.beginPath(); ctx.arc(node.sx, node.sy, radius + 7 * node.scale, 0, Math.PI * 2); ctx.stroke(); ctx.setLineDash([]); ctx.globalAlpha = alpha; }
    if (node.icon) drawIcon(ctx, node.icon, node.sx, node.sy, radius * 1.12, color);
    if (!mode3d() || node.scale > 0.985 || active) {
      ctx.textAlign = "center"; ctx.fillStyle = cssToken("--color-focus-orbit-text"); ctx.font = "650 13px system-ui";
      ctx.fillText(node.label, node.sx, node.sy + radius + 19 * node.scale);
      ctx.fillStyle = color; ctx.font = "400 11px system-ui"; ctx.fillText(node.sub, node.sx, node.sy + radius + 34 * node.scale);
    }
    ctx.restore();
  }

  function paintMember(ctx: CanvasRenderingContext2D, node: ProjectedNode): void {
    const color = cssToken(node.colorToken);
    const active = hoveredId() === node.id || selectedRowId() === node.itemId || displayFocusId() === node.itemId;
    const radius = node.radius * node.scale;
    const alpha = node.dimmed ? 0.12 : 1;
    ctx.save(); ctx.globalAlpha = alpha;
    halo(ctx, node.sx, node.sy, radius * (active ? 5.4 : 3.6), color, active ? 0.95 : 0.5);
    ctx.fillStyle = active ? cssToken("--color-focus-orbit-text") : color;
    ctx.beginPath(); ctx.arc(node.sx, node.sy, radius, 0, Math.PI * 2); ctx.fill();
    ctx.strokeStyle = color; ctx.lineWidth = active ? 2.4 : 1.2;
    ctx.beginPath(); ctx.arc(node.sx, node.sy, radius + (active ? 4 : 2.4) * node.scale, 0, Math.PI * 2); ctx.stroke();
    if (node.truthState?.toLocaleLowerCase().includes("stale") || node.truthState?.toLocaleLowerCase().includes("superseded")) {
      ctx.setLineDash([1.6, 3]); ctx.strokeStyle = cssToken("--color-focus-orbit-warning");
      ctx.beginPath(); ctx.arc(node.sx, node.sy, radius + 6.5 * node.scale, 0, Math.PI * 2); ctx.stroke(); ctx.setLineDash([]);
    }
    const showLabel = active || railItems().length <= 12 && (!mode3d() || node.scale > 0.985);
    if (showLabel) {
      const right = node.sx >= size().width / 2;
      ctx.textAlign = right ? "left" : "right"; ctx.fillStyle = cssToken("--color-focus-orbit-text"); ctx.font = `${active ? 600 : 400} 11.5px system-ui`;
      ctx.fillText(shortLabel(node.label), node.sx + (right ? radius + 9 : -radius - 9), node.sy + 4);
    }
    ctx.restore();
  }

  function paintMore(ctx: CanvasRenderingContext2D, node: ProjectedNode): void {
    const color = cssToken(node.colorToken);
    const radius = node.radius * node.scale;
    ctx.save(); ctx.setLineDash([3, 4]); ctx.strokeStyle = color; ctx.lineWidth = 1.3;
    ctx.beginPath(); ctx.arc(node.sx, node.sy, radius, 0, Math.PI * 2); ctx.stroke(); ctx.setLineDash([]);
    ctx.fillStyle = color; ctx.textAlign = "center"; ctx.font = "600 10.5px system-ui"; ctx.fillText(node.label, node.sx, node.sy + 3.5);
    ctx.restore();
  }

  function paintEdges(ctx: CanvasRenderingContext2D, current: OrbitModel, projected: ProjectedNode[], now: number): void {
    const byId = new Map(projected.map((node) => [node.id, node]));
    for (const edge of current.edges) {
      const source = byId.get(edge.sourceId);
      const target = byId.get(edge.targetId);
      if (!source || !target) continue;
      const midX = (source.sx + target.sx) / 2 - (target.sy - source.sy) * edge.curve;
      const midY = (source.sy + target.sy) / 2 + (target.sx - source.sx) * edge.curve;
      const active = hoveredId() === source.id || hoveredId() === target.id || selectedRowId() === target.itemId;
      ctx.save(); ctx.strokeStyle = withAlpha(cssToken(edge.colorToken), edge.strength * (active ? 0.95 : 0.6));
      ctx.lineWidth = active ? 2.3 : 1.25; ctx.beginPath(); ctx.moveTo(source.sx, source.sy); ctx.quadraticCurveTo(midX, midY, target.sx, target.sy); ctx.stroke();
      if (bundle() && target.kind === "member") { ctx.strokeStyle = withAlpha(cssToken("--color-focus-orbit-recall"), 0.55); ctx.lineWidth = 4; ctx.beginPath(); ctx.moveTo(source.sx, source.sy); ctx.lineTo((source.sx + midX) / 2, (source.sy + midY) / 2); ctx.stroke(); }
      if (recallUntil() > now && !still()) {
        const duration = 1900;
        const progress = 1 - Math.max(0, recallUntil() - now) / duration;
        const point = quadraticPoint(source.sx, source.sy, midX, midY, target.sx, target.sy, progress);
        halo(ctx, point.x, point.y, 18, cssToken("--color-focus-orbit-recall"), 1);
        ctx.fillStyle = cssToken("--color-focus-orbit-focus"); ctx.beginPath(); ctx.arc(point.x, point.y, 3, 0, Math.PI * 2); ctx.fill();
      }
      ctx.restore();
    }
  }

  function paint(now = performance.now()): void {
    const ctx = context;
    const canvas = canvasRef;
    if (!ctx || !canvas) return;
    const { width, height } = size();
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const pixelWidth = Math.round(width * dpr);
    const pixelHeight = Math.round(height * dpr);
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth; canvas.height = pixelHeight;
      canvas.style.width = `${width}px`; canvas.style.height = `${height}px`;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    paintBackground(ctx, now);
    const current = model();
    if (!current) { hitZones = []; return; }
    if (hairball()) { paintHairball(ctx); hitZones = []; return; }
    const projected = current.nodes.map(project);
    paintStrategyOverlay(ctx, projected);
    paintEdges(ctx, current, projected, now);
    const ordered = [...projected].sort((a, b) => b.depth - a.depth);
    for (const node of ordered) {
      if (node.kind === "focus") paintFocus(ctx, node, now);
      else if (node.kind === "hub") paintHub(ctx, node);
      else if (node.kind === "member") paintMember(ctx, node);
      else paintMore(ctx, node);
    }
    hitZones = projected.filter((node) => node.kind !== "focus").map((node) => ({ node, radius: Math.max(18, node.radius * node.scale + (node.kind === "member" ? 13 : 8)) }));
  }

  function frame(now: number): void {
    frameHandle = 0;
    const targetLift = mode3d() ? 1 : 0;
    let liftMoving = false;
    setLift((current) => {
      const next = current + (targetLift - current) * 0.09;
      liftMoving = Math.abs(targetLift - next) > 0.002;
      return liftMoving ? next : targetLift;
    });
    if (mode3d() && motion() && !still() && !dragStart) {
      setCamera((current) => ({ ...current, yaw: current.yaw + 0.0012 }));
    }
    paint(now);
    const animatingRecall = recallUntil() > now;
    if ((motion() && !still() || animatingRecall || liftMoving) && now - lastInteraction < 12000) {
      setIdleLabel(mode3d() ? "live · 3D" : "live");
      frameHandle = requestAnimationFrame(frame);
    } else {
      setIdleLabel(still() ? "still mode" : motion() ? "frozen · idle 12s" : "still · motion off");
    }
  }

  function hitAt(event: PointerEvent | MouseEvent): ProjectedNode | null {
    const rect = canvasRef?.getBoundingClientRect();
    if (!rect) return null;
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    let best: HitZone | null = null;
    let distance = Number.POSITIVE_INFINITY;
    for (const zone of hitZones) {
      const candidate = Math.hypot(x - zone.node.sx, y - zone.node.sy);
      if (candidate <= zone.radius && candidate < distance) { best = zone; distance = candidate; }
    }
    return best?.node ?? null;
  }

  function activateDisplayItem(item: OrbitDisplayItem, kind: "select" | "expand"): void {
    setSelectedRowId(item.id);
    if (isSyntheticItem(item)) {
      if (kind === "expand") {
        setSyntheticTrail((trail) => trail[trail.length - 1] === item.id ? trail : [...trail, item.id].slice(-8));
        setOpenGroupId(displayCategory(item));
        setStrategy("ego");
      }
    } else {
      props.onAction({ itemId: item.id, kind });
    }
    wake();
  }

  function activateNode(node: ProjectedNode): void {
    if (node.kind === "hub" && node.groupId) {
      setOpenGroupId(node.groupId);
      setStrategy("ego");
    } else if (node.kind === "member" && node.itemId) {
      const item = itemById().get(node.itemId);
      if (item) activateDisplayItem(item, "expand");
    }
    wake();
  }

  function stopSyntheticWorker(): void {
    const worker = syntheticWorker;
    if (!worker) return;
    worker.onmessage = null;
    worker.onerror = null;
    worker.terminate();
    if (syntheticWorker === worker) syntheticWorker = null;
  }

  function restoreProductionDisplay(status: string): void {
    setSyntheticItems(null);
    setSyntheticTrail([]);
    setSyntheticSummary(null);
    setSyntheticProgress(0);
    setSyntheticState("idle");
    setSyntheticStatus(status);
    setSelectedRowId(props.selectedId);
    setOpenGroupId(null);
    setPathA(null);
    setPathB(null);
    if (strategy() === "path" && !props.scene) setStrategy("search");
  }

  function resetSynthetic(): void {
    syntheticGeneration += 1;
    stopSyntheticWorker();
    restoreProductionDisplay("Released · no generated records are worker-held · fixed seed 0x4b524941");
    wake();
  }

  function resetWorkspace(): void {
    resetSynthetic();
    setSelectedRowId(null);
    setStrategy("search");
    setOpenGroupId(null);
    setCamera({ yaw: 0.42, pitch: 0.3, zoom: 1 });
    setHairball(false);
    setPreview(null);
    setTooltip(null);
    setForgottenPreviewIds(new Set<string>());
    props.onReset();
  }

  function backWorkspace(): void {
    if (!syntheticDisplay()) {
      props.onBack();
      return;
    }
    setSyntheticTrail((trail) => {
      const next = trail.slice(0, -1);
      setSelectedRowId(next[next.length - 1] ?? null);
      return next;
    });
    wake();
  }

  function handlePointerMove(event: PointerEvent): void {
    if (dragStart && mode3d()) {
      const dx = event.clientX - dragStart.x;
      const dy = event.clientY - dragStart.y;
      dragDistance = Math.max(dragDistance, Math.hypot(dx, dy));
      setCamera((current) => ({ ...current, yaw: dragStart!.yaw + dx * 0.0055, pitch: Math.max(-1.15, Math.min(1.15, dragStart!.pitch + dy * 0.0045)) }));
      setTooltip(null); wake(); return;
    }
    const node = hairball() ? null : hitAt(event);
    setHoveredId(node?.id ?? null);
    if (node) {
      const rect = stageRef?.getBoundingClientRect();
      setTooltip({ node, x: event.clientX - (rect?.left ?? 0), y: event.clientY - (rect?.top ?? 0) });
    } else setTooltip(null);
    wake();
  }

  function handleKeyDown(event: KeyboardEvent): void {
    const navigable = hitZones.map((zone) => zone.node);
    if (event.key === "Escape" || event.key === "Backspace") { event.preventDefault(); backWorkspace(); return; }
    if (event.key === "Home") { event.preventDefault(); resetWorkspace(); return; }
    if (["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp"].includes(event.key) && navigable.length > 0) {
      event.preventDefault();
      const delta = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : -1;
      const next = (keyboardIndex() + delta + navigable.length) % navigable.length;
      setKeyboardIndex(next); setHoveredId(navigable[next].id); wake(); return;
    }
    if ((event.key === "Enter" || event.key === " ") && navigable.length > 0) {
      event.preventDefault(); activateNode(navigable[keyboardIndex()] ?? navigable[0]);
    }
  }

  function generateSynthetic(count: number): void {
    const generation = ++syntheticGeneration;
    stopSyntheticWorker();
    restoreProductionDisplay(`Starting ${count.toLocaleString()} actual compact records · fixed seed 0x4b524941…`);
    const worker = new Worker(new URL("./focusOrbitSynthetic.worker.ts", import.meta.url), { type: "module" });
    syntheticWorker = worker;
    setSyntheticProgress(1);
    setSyntheticState("generating");
    worker.onmessage = (event: MessageEvent<FocusOrbitSyntheticMessage>) => {
      if (generation !== syntheticGeneration || syntheticWorker !== worker) return;
      const message = event.data;
      if (message.type === "progress") {
        setSyntheticProgress(message.percent);
        setSyntheticStatus(`${message.phase} · ${message.detail}`);
      } else if (message.type === "complete") {
        const items = syntheticSamplesToItems(message.samples, message.summary);
        setSyntheticSummary(message.summary);
        setSyntheticItems(items);
        setSyntheticTrail([]);
        setSelectedRowId(items[0]?.id ?? null);
        setOpenGroupId("knowledge");
        setPathA(null);
        setPathB(null);
        if (strategy() === "path") setStrategy("search");
        setSyntheticProgress(100);
        setSyntheticState("complete");
        setSyntheticStatus("Complete · actual compact graph is worker-held · bounded synthetic representatives are active and non-authoritative");
        wake();
      } else if (message.type === "error") {
        stopSyntheticWorker();
        setSyntheticState("error");
        setSyntheticStatus(`Generation failed: ${message.message}`);
      }
    };
    worker.onerror = (event) => {
      if (generation !== syntheticGeneration || syntheticWorker !== worker) return;
      event.preventDefault();
      stopSyntheticWorker();
      setSyntheticState("error");
      setSyntheticStatus("Worker failed to load or execute. No simulated success was substituted.");
    };
    worker.postMessage({ type: "generate", count, seed: 0x4b524941 });
  }

  function showPreview(kind: "empty" | "none" | "degraded"): void {
    const states = {
      empty: { icon: "◎", title: "No memories yet", message: "Start with a source or seed a small local demo corpus." },
      none: { icon: "⌕", title: "No results", message: "No loaded memory matches the current local filter. The authority snapshot is unchanged." },
      degraded: { icon: "⚠", title: "Memory projection unavailable", message: "Preview only: this illustrates the prototype recovery presentation and performs no data change." },
    };
    setPreview(states[kind]);
  }

  createEffect(() => {
    model(); hoveredId(); mode3d(); lift(); camera(); hairball(); bundle(); recallUntil();
    paint();
  });
  createEffect(() => {
    const current = model();
    const groups = new Set(current.groups.map((group) => group.id));
    if (!openGroupId() || !groups.has(openGroupId()!)) setOpenGroupId(current.groups[0]?.id ?? null);
    const ids = nodeItems().map((item) => item.id);
    const validIds = new Set(ids);
    if (selectedRowId() && !validIds.has(selectedRowId()!)) setSelectedRowId(ids[0] ?? null);
    if (!syntheticDisplay()) {
      if (selectedRowId() !== props.selectedId) setSelectedRowId(props.selectedId);
      if (!pathA() || !validIds.has(pathA()!)) setPathA(ids[0] ?? null);
      const last = ids[ids.length - 1] ?? null;
      if (!pathB() || !validIds.has(pathB()!)) setPathB(last);
    }
  });
  createEffect(() => setQueryDraft(props.filterQuery));

  onMount(() => {
    context = canvasRef?.getContext("2d") ?? null;
    const measure = () => {
      const rect = stageRef?.getBoundingClientRect();
      setSize({ width: Math.max(320, Math.round(rect?.width ?? 960)), height: Math.max(420, Math.round(rect?.height ?? 650)) });
      paint();
    };
    measure();
    if (typeof ResizeObserver !== "undefined" && stageRef) {
      resizeObserver = new ResizeObserver(measure);
      resizeObserver.observe(stageRef);
    } else window.addEventListener("resize", measure);
    const globalKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLocaleLowerCase() === "k") {
        event.preventDefault(); setCommandOpen(true); setCommandQuery("");
      } else if (event.key === "Escape" && resolutionOpen()) {
        event.preventDefault(); setResolutionOpen(false);
      } else if (event.key === "Escape" && commandOpen()) {
        event.preventDefault(); setCommandOpen(false);
      } else if (event.key === "Escape" && preview()) {
        event.preventDefault(); setPreview(null);
      }
    };
    window.addEventListener("keydown", globalKey);
    onCleanup(() => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("keydown", globalKey);
    });
    wake();
  });

  onCleanup(() => {
    resizeObserver?.disconnect();
    syntheticGeneration += 1;
    stopSyntheticWorker();
    if (frameHandle) cancelAnimationFrame(frameHandle);
  });

  const shownCount = () => hairball() ? 1024 : model().nodes.length;
  const snapshotLabel = () => syntheticDisplay()
    ? `${syntheticSummary()?.nodeCount.toLocaleString() ?? 0} generated records · representative view`
    : props.snapshotItemCount === null
      ? "snapshot unavailable"
      : `${props.snapshotItemCount.toLocaleString()} loaded snapshot records`;

  return (
    <section
      class="kria-focus-orbit"
      classList={{ "is-3d": mode3d(), "rail-hidden": !railOpen(), "hairball": hairball(), "still-mode": still() }}
      data-testid="focus-orbit"
      aria-label="Focus Orbit memory workspace"
    >
      <header class="orbit-bar">
        <div class="orbit-brand">K.R.I.A · <b>Focus Orbit</b></div>
        <nav class="orbit-crumbs" aria-label="Focus trail">
          <button class="orbit-crumb" classList={{ "is-last": focusTrailItems().length === 0 }} onClick={resetWorkspace}>◎ {syntheticDisplay() ? "Synthetic lab" : "Loaded knowledge"}</button>
          <For each={focusTrailItems()}>{(item, index) => <>
            <span class="orbit-crumb-sep">›</span>
            <button class="orbit-crumb is-last" title={item.label} onClick={() => index() === focusTrailItems().length - 1 && activateDisplayItem(item, "select")}>{shortLabel(item.label, 28)}</button>
          </>}</For>
          <Show when={strategy() === "ego" && activeGroup()}>{(group) => <>
            <span class="orbit-crumb-sep">›</span><span class="orbit-crumb is-last" style={{ color: cssToken(group().colorToken) }}>{group().label}</span>
          </>}</Show>
        </nav>
        <div class="orbit-tools">
          <button class="orbit-tbtn" type="button" onClick={backWorkspace}>← Back</button>
          <button class="orbit-tbtn" type="button" onClick={resetWorkspace}>Reset</button>
          <button class="orbit-tbtn hot" type="button" aria-pressed={mode3d()} onClick={() => { setMode3d((value) => !value); wake(); }}>3D</button>
          <Show when={mode3d()}><button class="orbit-tbtn" type="button" onClick={() => { setCamera({ yaw: 0.42, pitch: 0.3, zoom: 1 }); wake(); }}>Reset view</button></Show>
          <button class="orbit-tbtn" type="button" aria-pressed={motion()} onClick={() => { if (!still()) setMotion((value) => !value); wake(); }}>Motion</button>
          <button class="orbit-tbtn warn" type="button" aria-pressed={hairball()} onClick={() => { setHairball((value) => !value); wake(); }}>Hairball compare</button>
          <button class="orbit-tbtn" type="button" aria-pressed={railOpen()} onClick={() => setRailOpen((value) => !value)}>Reading list</button>
          <button class="orbit-tbtn lab" type="button" aria-pressed={labOpen()} onClick={() => setLabOpen((value) => !value)}>UX Lab</button>
          <button class="orbit-tbtn" type="button" onClick={() => setCommandOpen(true)}>⌘K</button>
        </div>
      </header>

      <div class="orbit-stage" data-testid="map-view" ref={stageRef}>
        <Show when={labOpen()}>
          <section class="orbit-lab-dock" classList={{ collapsed: dockCollapsed() }} aria-label="UX strategy comparison lab">
            <div class="orbit-lab-title"><span>Decision Lab · KRIA production</span><button class="orbit-icon-btn" type="button" aria-label="Collapse decision lab" onClick={() => setDockCollapsed((value) => !value)}>{dockCollapsed() ? "›" : "‹"}</button></div>
            <div class="orbit-lab-body">
              <div class="orbit-lab-section">
                <div class="orbit-lab-label"><span>Query context</span><span class={syntheticDisplay() ? "orbit-sim-tag" : "orbit-live-tag"}>{syntheticDisplay() ? "SYNTHETIC REPRESENTATIVES" : "LOADED SNAPSHOT"}</span></div>
                <div class="orbit-query-row"><input type="search" value={queryDraft()} disabled={syntheticDisplay()} aria-label="Search memories" placeholder={syntheticDisplay() ? "Reset synthetic data to search KRIA" : "Search memories…"} onInput={(event) => setQueryDraft(event.currentTarget.value)} onKeyDown={(event) => { if (event.key === "Enter" && !syntheticDisplay()) { props.onFilterQuery(queryDraft()); chooseStrategy("search"); } }} /><button class="orbit-lab-btn" type="button" disabled={syntheticDisplay()} onClick={() => { props.onFilterQuery(queryDraft()); chooseStrategy("search"); }}>Search</button></div>
                <p class="orbit-lab-note">{syntheticDisplay() ? "Synthetic samples are a non-authoritative worker projection; reset to search KRIA memory." : props.filterQuery ? `${props.items.filter((item) => item.kind !== "relation").length} loaded matches for “${props.filterQuery}”` : "Filter applies only to the current authorized snapshot."}</p>
              </div>
              <div class="orbit-lab-section">
                <div class="orbit-lab-label"><span>Synthetic dataset</span><span class="orbit-live-tag">ACTUAL · WORKER-HELD</span></div>
                <div class="orbit-synthetic-actions" role="group" aria-label="Generate compact synthetic graph"><button class="orbit-lab-btn" type="button" onClick={() => generateSynthetic(10000)}>Generate 10k</button><button class="orbit-lab-btn" type="button" onClick={() => generateSynthetic(100000)}>Generate 100k</button><button class="orbit-lab-btn" type="button" onClick={() => generateSynthetic(1000000)}><b>Generate 1M</b></button><button class="orbit-lab-btn reset" type="button" onClick={resetSynthetic}>Cancel / reset and release</button></div>
                <progress class="orbit-synthetic-progress" max="100" value={syntheticProgress()} aria-label="Synthetic graph generation progress" />
                <div class="orbit-synthetic-status" data-state={syntheticState()} role="status" aria-live="polite">{syntheticStatus()}</div>
                <div class="orbit-snapshot-metrics" aria-label="Generated graph metrics"><span>nodes <b>{syntheticSummary()?.nodeCount.toLocaleString() ?? "—"}</b></span><span>relations <b>{syntheticSummary()?.edgeCount.toLocaleString() ?? "—"}</b></span><span>clusters <b>{syntheticSummary()?.clusterCount.toLocaleString() ?? "—"}</b></span><span>worker bytes <b>{syntheticSummary() ? `${syntheticSummary()!.mib.toFixed(2)} MiB` : "—"}</b></span><span>duration <b>{syntheticSummary() ? `${syntheticSummary()!.durationMs.toLocaleString()} ms` : "—"}</b></span><span>checksum <b>{syntheticSummary()?.checksum ?? "—"}</b></span><span>seed <b>{syntheticSummary()?.seed ?? "0x4b524941"}</b></span><span>status <b>{syntheticDisplay() ? "representatives active" : syntheticState()}</b></span><Show when={syntheticSummary()}>{(summary) => <span class="orbit-category-metrics">categories <b>{ORBIT_CATEGORIES.map((category) => `${category.slice(0, 4)} ${summary().categoryTotals[category].toLocaleString()}`).join(" · ")}</b></span>}</Show></div>
                <p class="orbit-lab-note">The worker retains every compact generated record and relation. On completion, only bounded representative records enter this clearly labeled synthetic view; they never call KRIA memory actions or replace authority.</p>
              </div>
              <div class="orbit-lab-section">
                <div class="orbit-lab-label"><span>Layout strategy</span><span>{strategy()}</span></div>
                <div class="orbit-mode-grid" role="group" aria-label="Graph layout strategy">
                  <For each={Object.entries(STRATEGIES) as [OrbitStrategy, typeof STRATEGIES[OrbitStrategy]][]}>{([id, definition], index) => <button class="orbit-mode-btn" classList={{ wide: index() === 4 }} type="button" disabled={syntheticDisplay() && id === "path"} title={syntheticDisplay() && id === "path" ? "Representative samples do not include traversable relation endpoints" : definition.note} aria-pressed={strategy() === id} onClick={() => chooseStrategy(id)}><b>{definition.title}</b>{definition.label.replace(/^.*?-/, "")}</button>}</For>
                </div>
              </div>
              <Show when={strategy() === "path"}>
                <div class="orbit-lab-section">
                  <div class="orbit-lab-label"><span>Path pins</span><span class="orbit-live-tag">LOADED RELATIONS</span></div>
                  <div class="orbit-path-pins">
                    <select aria-label="Pin A" value={pathA() ?? ""} onChange={(event) => { setPathA(event.currentTarget.value); wake(); }}><For each={nodeItems()}>{(item) => <option value={item.id}>A: {shortLabel(item.label, 26)}</option>}</For></select>
                    <select aria-label="Pin B" value={pathB() ?? ""} onChange={(event) => { setPathB(event.currentTarget.value); wake(); }}><For each={nodeItems()}>{(item) => <option value={item.id}>B: {shortLabel(item.label, 26)}</option>}</For></select>
                  </div>
                  <p class="orbit-lab-note">{model()?.pathFound ? "Supported route found inside this snapshot." : "No supported route; no edge was invented."}</p>
                </div>
              </Show>
              <div class="orbit-lab-section">
                <div class="orbit-lab-label"><span>Projection density</span><span>{density()} sampled records max</span></div>
                <div class="orbit-control-row"><span class="orbit-lab-note no-margin">bounded scene</span><For each={DENSITIES}>{(value) => <button class="orbit-chip" classList={{ active: density() === value }} type="button" aria-pressed={density() === value} onClick={() => { setDensity(value); wake(); }}>{value}</button>}</For><button class="orbit-chip" type="button" aria-pressed={bundle()} onClick={() => { setBundle((value) => !value); wake(); }}>bundle fan</button></div>
              </div>
              <div class="orbit-lab-section">
                <div class="orbit-lab-label"><span>{syntheticDisplay() ? "Generated age horizon" : "Revision horizon"}</span><span>≤ {decaySpan()} {syntheticDisplay() ? "days" : "revisions"}</span></div>
                <input class="orbit-range" type="range" min="0" max="210" value={decaySpan()} aria-label={syntheticDisplay() ? "Maximum generated age in days" : "Maximum revision distance"} onInput={(event) => { setDecaySpan(Number(event.currentTarget.value)); wake(); }} />
                <div class="orbit-decay-legend"><span>strong · current</span><span>faint · older</span></div>
              </div>
              <div class="orbit-lab-section">
                <div class="orbit-lab-label"><span>Recall events</span><span class="orbit-sim-tag">VISUAL PREVIEW</span></div>
                <button class="orbit-lab-btn" type="button" onClick={() => { if (!still()) setRecallUntil(performance.now() + 1900); setEventMessage("visual pulse → current loaded edges · no memory mutation"); wake(); }}>▶ Simulate recall pulse</button>
                <div class="orbit-event-feed">{eventMessage()}</div>
              </div>

              <div class="orbit-lab-section">
                <div class="orbit-lab-label"><span>Preview states</span><span class="orbit-sim-tag">NON-PERSISTENT</span></div>
                <div class="orbit-control-row"><button class="orbit-chip" type="button" onClick={() => showPreview("empty")}>Empty</button><button class="orbit-chip" type="button" onClick={() => showPreview("none")}>No result</button><button class="orbit-chip" type="button" onClick={() => showPreview("degraded")}>DB degraded</button></div>
              </div>
              <div class="orbit-lab-section">
                <div class="orbit-lab-label"><span>Accessibility</span><span class="orbit-live-tag">DOM MIRROR</span></div>
                <div class="orbit-control-row"><button class="orbit-chip" type="button" aria-pressed={mirrorVisible()} onClick={() => setMirrorVisible((value) => !value)}>Preview mirror</button><button class="orbit-chip" type="button" aria-pressed={still()} onClick={() => { setStill((value) => !value); setMotion(false); wake(); }}>Still mode</button></div>
                <p class="orbit-lab-note">Ctrl+K palette · Esc returns focus · visible focus rings.</p>
              </div>
              <div class="orbit-proof-card"><b>What this proves</b><br />{syntheticDisplay() ? "Synthetic worker data is visible only as a bounded, labeled, non-authoritative representative projection." : STRATEGIES[strategy()].note}</div>
            </div>
          </section>
        </Show>

        <canvas
          ref={canvasRef}
          class="orbit-canvas"
          data-testid="map-canvas"
          tabindex="0"
          role="application"
          aria-label={`Focus Orbit ${syntheticDisplay() ? "synthetic representative" : "memory"} graph. ${shownCount()} of ${displayTotalNodeCount()} nodes shown. Arrow keys move, Enter focuses, Escape goes back.`}
          onKeyDown={handleKeyDown}
          onPointerDown={(event) => { if (!mode3d()) return; dragStart = { x: event.clientX, y: event.clientY, yaw: camera().yaw, pitch: camera().pitch }; dragDistance = 0; event.currentTarget.setPointerCapture(event.pointerId); wake(); }}
          onPointerUp={(event) => { dragStart = null; event.currentTarget.releasePointerCapture(event.pointerId); wake(); }}
          onPointerMove={handlePointerMove}
          onPointerLeave={() => { setHoveredId(null); setTooltip(null); }}
          onClick={(event) => { if (dragDistance > 4 || hairball()) return; const node = hitAt(event); if (node) activateNode(node); }}
          onWheel={(event) => { if (!mode3d()) return; event.preventDefault(); setCamera((current) => ({ ...current, zoom: Math.min(2.1, Math.max(0.55, current.zoom * (event.deltaY > 0 ? 0.92 : 1.09))) })); wake(); }}
        />

        <div class="orbit-hud">
          <div class="orbit-hud-card"><div class="orbit-hud-label">Shown / {syntheticDisplay() ? "generated" : "loaded"} nodes</div><div class="orbit-hud-big">{shownCount().toLocaleString()} <small>/ {displayTotalNodeCount().toLocaleString()}</small></div><div class="orbit-hud-seed">{syntheticDisplay() ? "SYNTHETIC · NON-AUTHORITATIVE" : "KRIA · AUTHORIZED SNAPSHOT"}</div></div>
          <div class="orbit-hud-card"><div class="orbit-hud-label">Layout strategy</div><div class="orbit-hud-strategy">{hairball() ? "none — raw node-link" : STRATEGIES[strategy()].label} · {mode3d() ? "3D" : "2D"}</div><div class="orbit-hud-seed">seed 0x{(model()?.seed ?? 0).toString(16).padStart(8, "0")} · deterministic</div></div>
          <Show when={mode3d()}><div class="orbit-hud-card orbit-hud-z"><div class="orbit-hud-label">Z axis source</div><div class="orbit-z-title">{syntheticDisplay() ? "generated relevance score" : "backend relevance when supplied"}</div><div class="orbit-hud-seed">{syntheticDisplay() ? "derived from generated confidence + age" : <><span>neutral plane when unavailable</span><br /><span>no invented recency</span></>}</div></div></Show>
        </div>
        <div class="orbit-idle">{idleLabel()}</div>
        <Show when={hairball()}><div class="orbit-truncation">Hairball compare: 1,024 deterministic representative visual points. The {syntheticDisplay() ? "worker dataset" : "KRIA snapshot"} remains bounded and unchanged.</div></Show>

        <div class="orbit-legend"><h4>Every channel carries data</h4><dl><dt>Distance</dt><dd>{syntheticDisplay() ? "generated relevance" : "supplied relevance or neutral"}</dd><dt>Direction</dt><dd>{syntheticDisplay() ? "synthetic category" : "backend item kind"}</dd><dt>Size</dt><dd>relevance when available</dd><dt>Colour</dt><dd>{syntheticDisplay() ? "category / truth state" : "kind / truth state"}</dd><dt>Glow</dt><dd>current relation emphasis</dd><dt>Depth</dt><dd>relevance — 3D only</dd><dt>Stars</dt><dd>{syntheticDisplay() ? "generated corpus scale" : "loaded scope scale"}</dd></dl></div>
        <div class="orbit-hint"><Show when={!mode3d()} fallback={<><kbd>Drag</kbd> to orbit · <kbd>Wheel</kbd> to zoom · <kbd>Click</kbd> to focus · <kbd>Esc</kbd> back</>}><kbd>Click</kbd> a group to open · <kbd>Click</kbd> an item to re-focus · <kbd>Esc</kbd> back · <kbd>←→</kbd> move</Show></div>

        <Show when={selectedItem()}>{(item) => <aside class="orbit-context-panel" aria-label="Context inspector">
          <div class="orbit-lab-label"><span>{isSyntheticItem(item()) ? "Synthetic detail" : "Memory detail"}</span><span class={isSyntheticItem(item()) ? "orbit-sim-tag" : "orbit-live-tag"}>{isSyntheticItem(item()) ? "NON-AUTHORITATIVE SAMPLE" : "REAL RECORD"}</span></div>
          <h3>{item().label}</h3>
          <p>{displayCategory(item())} · {item().truthState}{isSyntheticItem(item()) ? ` · age ${item().syntheticAgeDays ?? 0}d · cluster ${item().syntheticCluster ?? 0} · source ${item().syntheticSource ?? "generated"}` : ` · revision ${item().revision}`}{item().score === undefined ? "" : ` · relevance ${formatScore(item().score)}`}</p>
          <div class="orbit-context-row"><button class="orbit-action-btn" type="button" onClick={() => activateDisplayItem(item(), "expand")}>Focus</button><Show when={!isSyntheticItem(item()) && props.inspectorAvailable}><button class="orbit-action-btn" type="button" onClick={() => props.onOpenInspector(item().id)}>Inspect</button></Show></div>
          <div class="orbit-lab-section context-preview"><div class="orbit-lab-label"><span>Lifecycle preview</span><span class="orbit-sim-tag">NON-PERSISTENT</span></div><div class="orbit-context-row"><button class="orbit-action-btn danger" type="button" onClick={() => setForgottenPreviewIds((current) => new Set(current).add(item().id))}>{forgottenPreviewIds().has(item().id) ? "Forgotten preview" : "Forget"}</button><button class="orbit-action-btn good" type="button" onClick={() => setForgottenPreviewIds((current) => { const next = new Set(current); next.delete(item().id); return next; })}>Restore</button></div></div>
          <div class="orbit-lab-section context-preview"><div class="orbit-lab-label"><span>Contradiction comparison</span><span class="orbit-sim-tag">MOCK FLOW</span></div><div class="orbit-compare-grid"><div class="orbit-claim"><b>A · {isSyntheticItem(item()) ? "synthetic sample" : "loaded record"}</b>{shortLabel(item().label, 48)}</div><div class="orbit-claim"><b>B · comparison slot</b>No authoritative contradictory record was supplied.</div></div><div class="orbit-context-row"><button class="orbit-action-btn" type="button" onClick={() => setResolutionOpen(true)}>Compare & resolve</button></div></div>
        </aside>}</Show>

        <Show when={preview()}>{(state) => <section class="orbit-state-preview" role="region" aria-live="polite"><div><div class="orbit-state-icon">{state().icon}</div><h2>{state().title}</h2><p>{state().message}</p><div class="orbit-context-row centered"><button class="orbit-action-btn good" type="button" onClick={() => setPreview(null)}>Close preview</button><Show when={state().title === "No memories yet" && props.onSeedDemo}><button class="orbit-action-btn" type="button" onClick={() => { setPreview(null); props.onSeedDemo?.(); }}>Seed demo memories</button></Show></div><span class="orbit-sim-tag">SIMULATED PRESENTATION · NO DATA CHANGES</span></div></section>}</Show>

        <Show when={mirrorVisible()}><section class="orbit-graph-mirror" tabindex="-1" aria-label="Accessible graph mirror"><h2>Canvas accessibility mirror</h2><p class="orbit-lab-note">Same current projection as DOM controls:</p><ul><For each={railItems().slice(0, 24)}>{(item) => <li><button type="button" onClick={() => activateDisplayItem(item, "expand")}>{displayCategory(item)}: {item.label}</button> — {item.truthState}</li>}</For></ul></section></Show>
        <section class="orbit-sr-graph" aria-label="Current canvas graph nodes and actions" aria-live="polite"><h2>Current graph projection</h2><ul><For each={railItems().slice(0, 24)}>{(item) => <li><button type="button" onClick={() => activateDisplayItem(item, "expand")}>{displayCategory(item)}: {item.label}</button></li>}</For></ul></section>

        <Show when={tooltip()}>{(tip) => <div class="orbit-tooltip" role="tooltip" style={{ left: `${Math.min(size().width - 290, tip().x + 16)}px`, top: `${Math.min(size().height - 130, tip().y + 14)}px` }}><div class="orbit-tooltip-name">{tip().node.label}</div><div class="orbit-tooltip-meta"><b>kind</b><span>{tip().node.kind === "hub" ? "navigation group" : tip().node.sub}</span><b>truth</b><span>{tip().node.truthState ?? "group aggregate"}</span><b>revision</b><span>{tip().node.revision ?? props.graphRevision ?? "—"}</span><b>relevance</b><span>{formatScore(tip().node.score)}</span></div></div>}</Show>

        <Show when={props.isLoading || props.isSeeding}><div class="orbit-status-overlay" data-testid="loading-indicator" role="status">{props.isSeeding ? "Seeding demo data, please wait…" : "Loading authorized knowledge snapshot…"}</div></Show>
        <Show when={props.error}><div class="orbit-error" role="alert"><span>{props.error}</span><Show when={props.onRetry}><button type="button" onClick={() => props.onRetry?.()}>Retry</button></Show></div></Show>
        <Show when={props.seedMessage}><div class="orbit-success" role="status">{props.seedMessage}</div></Show>
        <Show when={!props.isLoading && !props.isSeeding && effectiveItems().length === 0}><div class="orbit-empty" data-testid="empty-state"><div class="orbit-state-icon">◎</div><h2>{props.filterQuery ? "No loaded items match this filter" : "No memories yet"}</h2><p>{props.filterQuery ? "Clear the local loaded-view filter to restore the snapshot." : "The production graph is empty; no synthetic records were substituted."}</p><div class="orbit-context-row centered"><Show when={props.filterQuery}><button class="orbit-action-btn" type="button" onClick={() => props.onFilterQuery("")}>Clear filter</button></Show><Show when={props.onSeedDemo && !props.filterQuery}><button class="orbit-action-btn good" type="button" onClick={() => props.onSeedDemo?.()}>Seed demo knowledge</button></Show></div></div></Show>
      </div>

      <Show when={railOpen()}>
        <aside class="orbit-rail" data-testid="list-view" aria-label="Reading list">
          <div class="orbit-rail-head"><div class="orbit-rail-title">Reading list — same set, readable form</div><div class="orbit-rail-sub">{activeGroup()?.label ?? "Loaded knowledge snapshot"}</div><div class="orbit-rail-count">{railItems().length} shown · {snapshotLabel()}</div></div>
          <div class="orbit-rail-list" role="list">
            <Show when={railItems().length > 0} fallback={<div class="orbit-rail-empty">No readable nodes in this projection.</div>}>
              <For each={railItems()}>{(item) => {
                const group = () => model().groups.find((candidate) => candidate.id === displayCategory(item));
                const selected = () => selectedRowId() === item.id || displayFocusId() === item.id;
                return <div
                  class="orbit-rail-item"
                  classList={{ selected: selected() }}
                  role="listitem"
                  tabIndex={0}
                  aria-selected={selected()}
                  data-item-id={item.id}
                  data-kind={displayCategory(item)}
                  data-authority-class={item.authorityClass}
                  data-display-source={item.orbitSource}
                  data-truth-state={item.truthState}
                  data-testid={`select-btn-${item.id}`}
                  style={{ "border-left-color": cssToken(group()?.colorToken ?? "--color-focus-orbit-other") }}
                  onMouseEnter={() => { setHoveredId(`item:${item.id}`); wake(); }}
                  onMouseLeave={() => setHoveredId(null)}
                  onClick={() => activateDisplayItem(item, selectedRowId() === item.id ? "expand" : "select")}
                  onKeyDown={(event) => {
                    if (event.key !== "Enter" && event.key !== " ") return;
                    event.preventDefault();
                    activateDisplayItem(item, selectedRowId() === item.id ? "expand" : "select");
                  }}
                ><span class="orbit-rail-top"><i class="orbit-dot" style={{ background: cssToken(group()?.colorToken ?? "--color-focus-orbit-other") }} /><span class="orbit-rail-kind">{item.truthState} · {displayCategory(item)}</span></span>
                  <span class="orbit-rail-name" data-field="label">{item.label}</span>
                  <span class="orbit-rail-meta"><span>{isSyntheticItem(item) ? `age ${item.syntheticAgeDays ?? 0}d` : `rev ${item.revision}`}</span><span>{item.score === undefined ? "relevance —" : `rel ${item.score.toFixed(2)}`}</span><span>{isSyntheticItem(item) ? "synthetic" : item.authorityClass}</span></span>
                  <Show when={selected()}><span class="orbit-rail-explain">Canvas distance uses {isSyntheticItem(item) ? "generated" : "supplied"} relevance when available. Click again to focus this {isSyntheticItem(item) ? "sample" : "record"}.<span class="orbit-focus-link">Focus on this →</span></span></Show>
                  <Show when={!isSyntheticItem(item) && props.inspectorAvailable}><span class="orbit-inline-actions"><button type="button" data-testid="inspector-btn" onClick={(event) => { event.stopPropagation(); props.onOpenInspector(item.id); }}>Inspect</button></span></Show>
                  <Show when={!isSyntheticItem(item) && props.pathAvailable && props.selectedId && props.selectedId !== item.id}><span class="orbit-inline-actions"><button type="button" data-testid="path-btn" onClick={(event) => { event.stopPropagation(); props.onRequestPath(props.selectedId!, item.id); }}>Path</button></span></Show>
                </div>;
              }}</For>
            </Show>
          </div>
          <div class="orbit-rail-foot">The graph navigates. The list explains. Nothing is reachable only by pointing — every item here is keyboard and screen-reader navigable.</div>
        </aside>
      </Show>

      <Show when={resolutionOpen()}><div class="orbit-modal-backdrop" role="presentation"><div class="orbit-dialog" role="dialog" aria-modal="true" aria-labelledby="orbit-resolve-title"><h2 id="orbit-resolve-title">Resolve contradiction <span class="orbit-sim-tag">MOCK FLOW</span></h2><div class="orbit-compare-grid"><div class="orbit-claim"><b>A · {isSyntheticItem(selectedItem()) ? "synthetic sample" : "loaded record"}</b>{selectedItem()?.label ?? "Current memory"}</div><div class="orbit-claim"><b>B · comparison slot</b>No authoritative contradictory record was supplied.</div></div><p class="orbit-lab-note">Choosing an outcome closes this preview only. KRIA memory is not mutated.</p><div class="orbit-context-row"><button class="orbit-action-btn good" type="button" onClick={() => setResolutionOpen(false)}>Keep A</button><button class="orbit-action-btn" type="button" onClick={() => setResolutionOpen(false)}>Keep B</button><button class="orbit-action-btn" type="button" onClick={() => setResolutionOpen(false)}>Merge preview</button><button class="orbit-action-btn" type="button" onClick={() => setResolutionOpen(false)}>Cancel</button></div></div></div></Show>

      <Show when={commandOpen()}><div class="orbit-modal-backdrop" role="presentation" onClick={(event) => { if (event.target === event.currentTarget) setCommandOpen(false); }}><div class="orbit-dialog" role="dialog" aria-modal="true" aria-labelledby="orbit-command-title"><h2 id="orbit-command-title">Command palette <span class="orbit-live-tag">Ctrl+K · LIVE VIEW</span></h2><input autofocus value={commandQuery()} aria-label="Search commands and memories" placeholder="Search, jump, or change view…" onInput={(event) => setCommandQuery(event.currentTarget.value)} /><div class="orbit-command-results"><For each={commands()} fallback={<p class="orbit-lab-note">No command. Try “path”, “temporal”, or “3D”.</p>}>{(command, index) => <button class="orbit-command-item" classList={{ active: index() === 0 }} type="button" onClick={() => { setCommandOpen(false); command.run(); }}>{command.label}</button>}</For></div><button class="orbit-dialog-close" type="button" onClick={() => setCommandOpen(false)}>Close</button></div></div></Show>
    </section>
  );
}

export default FocusOrbit;