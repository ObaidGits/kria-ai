/**
 * GraphScene — the Three.js WebGL renderer for the Knowledge Graph lens
 * (task 6.4, Req 5.4 / 16.3). BROWSER-ONLY and isolated: it is constructed only
 * after the capability gate enables 3D (passing §11.3 G2 probe), so WebGL is
 * known-present. It is never imported by the pure logic modules and never runs
 * under jsdom (WebGL is unavailable there — the surrounding logic is what the
 * tests cover).
 *
 * Implements the §5.4 rendering hard rules that are GL-specific:
 *   • INSTANCED rendering — one InstancedMesh for all nodes, buffer-geometry
 *     line sets for edges (real + predicted).
 *   • Node size = centrality; per-instance color = community (accent reserved
 *     for the SELECTED node only).
 *   • A DAMPED, CONSTRAINED orbit camera (implemented here — no game-engine
 *     controls dependency) with a reset-view; single soft key light + ambient;
 *     matte materials.
 *   • LOD + label-set selection are delegated to graphModel and consumed for the
 *     HTML label overlay (labels only for focused/near set).
 *
 * All colors are resolved from DESIGN TOKENS at runtime (getComputedStyle), so
 * there are no raw color literals here (token-lint clean) and the scene tracks
 * the active dark/light theme.
 */
import {
  AmbientLight,
  BufferGeometry,
  Color,
  DirectionalLight,
  DynamicDrawUsage,
  Float32BufferAttribute,
  InstancedMesh,
  LineBasicMaterial,
  LineSegments,
  Matrix4,
  MeshStandardMaterial,
  PerspectiveCamera,
  Raycaster,
  Scene,
  SphereGeometry,
  Vector2,
  Vector3,
  WebGLRenderer,
} from "three";
import {
  communityColorToken,
  EDGE_COLOR_TOKEN,
  maxCentrality,
  nodeSizeForCentrality,
  PREDICTED_EDGE_COLOR_TOKEN,
  SELECTION_COLOR_TOKEN,
  selectLabelSet,
  type GraphEdge,
  type GraphNode,
  type LabelCandidate,
} from "./graphModel";
import type { PositionedNode } from "./layoutSettle";

/**
 * Resolve a design-token CSS variable to a THREE.Color from the live theme.
 * Falls back to a neutral mid-grey (constructed numerically — no raw color
 * literal) when the token is empty/unparseable.
 */
function tokenColor(root: HTMLElement, token: string): Color {
  const neutral = new Color(0.53, 0.53, 0.53);
  const raw = getComputedStyle(root).getPropertyValue(token).trim();
  if (!raw) return neutral;
  try {
    return new Color(raw);
  } catch {
    return neutral;
  }
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

export interface ScreenLabel {
  id: string;
  label: string;
  x: number;
  y: number;
}

/** Constrained orbit limits (§5.4 damped constrained orbit). */
const ORBIT = {
  minRadius: 8,
  maxRadius: 160,
  minPhi: Math.PI * 0.08,
  maxPhi: Math.PI * 0.92,
  rotateSpeed: 0.005,
  zoomSpeed: 0.0015,
  damping: 0.15,
  dragThresholdPx: 4,
};

export class GraphScene {
  private readonly renderer: WebGLRenderer;
  private readonly scene = new Scene();
  private readonly camera: PerspectiveCamera;
  private readonly raycaster = new Raycaster();
  private readonly themeRoot: HTMLElement;
  private readonly canvas: HTMLCanvasElement;

  private nodeMesh: InstancedMesh | null = null;
  private realEdges: LineSegments | null = null;
  private predictedEdges: LineSegments | null = null;

  private nodes: GraphNode[] = [];
  private edges: GraphEdge[] = [];
  private positions = new Map<string, Vector3>();
  private idToIndex = new Map<string, number>();
  private selectedId: string | null = null;
  private maxCent = 1;

  // Damped orbit state: `desired*` are targets, current values ease toward them.
  private readonly target = new Vector3(0, 0, 0);
  private radius = 60;
  private theta = 0; // azimuth
  private phi = Math.PI * 0.5; // polar
  private desiredRadius = 60;
  private desiredTheta = 0;
  private desiredPhi = Math.PI * 0.5;
  private readonly initial = { radius: 60, theta: 0, phi: Math.PI * 0.5 };

  // Pointer interaction bookkeeping.
  private dragging = false;
  private moved = false;
  private lastPointer = { x: 0, y: 0 };
  private onInteraction: (() => void) | null = null;
  private onPick: ((id: string) => void) | null = null;

  constructor(canvas: HTMLCanvasElement, themeRoot: HTMLElement = document.documentElement) {
    this.canvas = canvas;
    this.themeRoot = themeRoot;
    this.renderer = new WebGLRenderer({ canvas, antialias: true, alpha: true });
    this.renderer.setPixelRatio(Math.min(globalThis.devicePixelRatio || 1, 2));
    const w = canvas.clientWidth || 640;
    const h = canvas.clientHeight || 480;
    this.renderer.setSize(w, h, false);

    this.camera = new PerspectiveCamera(55, w / h, 0.1, 2000);
    this.updateCamera();

    // Single soft key light + ambient, matte materials (§5.4). White light is
    // constructed numerically (no raw color literal → token-lint clean).
    const white = new Color(1, 1, 1);
    const key = new DirectionalLight(white, 1.1);
    key.position.set(1, 1.4, 1.2);
    this.scene.add(key);
    this.scene.add(new AmbientLight(white, 0.55));

    this.attachPointer();
  }

  /** Register the "user interacted" hook (drives resume-on-interaction). */
  setOnInteraction(cb: () => void): void {
    this.onInteraction = cb;
  }

  /** Register the node-pick hook (a click that hits a node → focus/expand). */
  setOnPick(cb: (id: string) => void): void {
    this.onPick = cb;
  }

  /** Resize the drawing buffer + camera to the current canvas box. */
  resize(width: number, height: number): void {
    if (width <= 0 || height <= 0) return;
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
  }

  /**
   * (Re)build the instanced node mesh + edge line sets for a graph model. Old
   * GPU resources are released first (no leak across re-loads).
   */
  setGraph(nodes: GraphNode[], edges: GraphEdge[]): void {
    this.disposeMeshes();
    this.nodes = nodes;
    this.edges = edges;
    this.idToIndex = new Map(nodes.map((n, i) => [n.id, i]));
    this.maxCent = Math.max(1, maxCentrality(nodes));

    const geo = new SphereGeometry(1, 16, 12);
    const mat = new MeshStandardMaterial({ roughness: 0.85, metalness: 0.0 });
    const mesh = new InstancedMesh(geo, mat, Math.max(1, nodes.length));
    mesh.instanceMatrix.setUsage(DynamicDrawUsage);
    const matrix = new Matrix4();
    for (let i = 0; i < nodes.length; i++) {
      const n = nodes[i];
      const scale = nodeSizeForCentrality(n.centrality, this.maxCent);
      matrix.makeScale(scale, scale, scale);
      mesh.setMatrixAt(i, matrix);
      mesh.setColorAt(i, tokenColor(this.themeRoot, communityColorToken(n.community)));
    }
    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    this.scene.add(mesh);
    this.nodeMesh = mesh;

    this.realEdges = this.makeLineSegments(tokenColor(this.themeRoot, EDGE_COLOR_TOKEN), false);
    this.predictedEdges = this.makeLineSegments(
      tokenColor(this.themeRoot, PREDICTED_EDGE_COLOR_TOKEN),
      true,
    );
    this.scene.add(this.realEdges);
    this.scene.add(this.predictedEdges);
  }

  private makeLineSegments(color: Color, transparent: boolean): LineSegments {
    const geometry = new BufferGeometry();
    geometry.setAttribute("position", new Float32BufferAttribute([], 3));
    const material = new LineBasicMaterial({
      color,
      transparent,
      opacity: transparent ? 0.5 : 0.8,
    });
    const seg = new LineSegments(geometry, material);
    seg.frustumCulled = true;
    return seg;
  }

  /** Apply a batch of layout positions to node instances + rebuild edge lines. */
  applyPositions(batch: PositionedNode[]): void {
    for (const p of batch) {
      this.positions.set(p.id, new Vector3(p.x, p.y, p.z));
    }
    if (this.nodeMesh) {
      const matrix = new Matrix4();
      for (let i = 0; i < this.nodes.length; i++) {
        const n = this.nodes[i];
        const p = this.positions.get(n.id);
        if (!p) continue;
        const s = nodeSizeForCentrality(n.centrality, this.maxCent);
        matrix.makeScale(s, s, s);
        matrix.setPosition(p);
        this.nodeMesh.setMatrixAt(i, matrix);
      }
      this.nodeMesh.instanceMatrix.needsUpdate = true;
      this.nodeMesh.computeBoundingSphere();
    }
    this.rebuildEdges();
  }

  private rebuildEdges(): void {
    const real: number[] = [];
    const predicted: number[] = [];
    for (const e of this.edges) {
      const a = this.positions.get(e.source);
      const b = this.positions.get(e.target);
      if (!a || !b) continue;
      const bucket = e.predicted ? predicted : real;
      bucket.push(a.x, a.y, a.z, b.x, b.y, b.z);
    }
    if (this.realEdges) {
      this.realEdges.geometry.setAttribute("position", new Float32BufferAttribute(real, 3));
      this.realEdges.geometry.computeBoundingSphere();
    }
    if (this.predictedEdges) {
      this.predictedEdges.geometry.setAttribute(
        "position",
        new Float32BufferAttribute(predicted, 3),
      );
      this.predictedEdges.geometry.computeBoundingSphere();
    }
  }

  /** Highlight the selected node (accent = selection only, §5.4). */
  setSelected(id: string | null): void {
    if (!this.nodeMesh) return;
    const accent = tokenColor(this.themeRoot, SELECTION_COLOR_TOKEN);
    if (this.selectedId && this.idToIndex.has(this.selectedId)) {
      const prev = this.nodes[this.idToIndex.get(this.selectedId)!];
      this.nodeMesh.setColorAt(
        this.idToIndex.get(this.selectedId)!,
        tokenColor(this.themeRoot, communityColorToken(prev.community)),
      );
    }
    if (id && this.idToIndex.has(id)) {
      this.nodeMesh.setColorAt(this.idToIndex.get(id)!, accent);
    }
    if (this.nodeMesh.instanceColor) this.nodeMesh.instanceColor.needsUpdate = true;
    this.selectedId = id;
  }

  /**
   * Project the focused/near node set to screen coordinates for the HTML label
   * overlay (labels only for focused/near set, §5.4). Uses graphModel's
   * selectLabelSet so the selection rule is the tested, shared one.
   */
  computeLabels(focusedId: string | null, width: number, height: number): ScreenLabel[] {
    const camPos = this.camera.position;
    const candidates: LabelCandidate[] = [];
    for (const n of this.nodes) {
      const p = this.positions.get(n.id);
      if (!p) continue;
      candidates.push({ id: n.id, distance: p.distanceTo(camPos) });
    }
    const labelIds = selectLabelSet(candidates, focusedId);
    const labels: ScreenLabel[] = [];
    const v = new Vector3();
    for (const n of this.nodes) {
      if (!labelIds.has(n.id)) continue;
      const p = this.positions.get(n.id);
      if (!p) continue;
      v.copy(p).project(this.camera);
      if (v.z > 1) continue; // behind camera
      labels.push({
        id: n.id,
        label: n.label,
        x: (v.x * 0.5 + 0.5) * width,
        y: (-v.y * 0.5 + 0.5) * height,
      });
    }
    return labels;
  }

  /** Reset the orbit camera to its initial framing (§5.4 reset-view). */
  resetView(): void {
    this.desiredRadius = this.initial.radius;
    this.desiredTheta = this.initial.theta;
    this.desiredPhi = this.initial.phi;
    this.onInteraction?.();
  }

  /** Render exactly one frame (eases the damped orbit, then draws). */
  render(): void {
    const d = ORBIT.damping;
    this.radius += (this.desiredRadius - this.radius) * d;
    this.theta += (this.desiredTheta - this.theta) * d;
    this.phi += (this.desiredPhi - this.phi) * d;
    this.updateCamera();
    this.renderer.render(this.scene, this.camera);
  }

  private updateCamera(): void {
    const sinPhi = Math.sin(this.phi);
    this.camera.position.set(
      this.target.x + this.radius * sinPhi * Math.sin(this.theta),
      this.target.y + this.radius * Math.cos(this.phi),
      this.target.z + this.radius * sinPhi * Math.cos(this.theta),
    );
    this.camera.lookAt(this.target);
  }

  // ── Damped, constrained orbit input ──────────────────────────────────────
  private readonly onPointerDown = (e: PointerEvent) => {
    this.dragging = true;
    this.moved = false;
    this.lastPointer = { x: e.clientX, y: e.clientY };
    this.canvas.setPointerCapture?.(e.pointerId);
  };

  private readonly onPointerMove = (e: PointerEvent) => {
    if (!this.dragging) return;
    const dx = e.clientX - this.lastPointer.x;
    const dy = e.clientY - this.lastPointer.y;
    if (Math.abs(dx) + Math.abs(dy) > ORBIT.dragThresholdPx) this.moved = true;
    this.lastPointer = { x: e.clientX, y: e.clientY };
    this.desiredTheta -= dx * ORBIT.rotateSpeed;
    this.desiredPhi = clamp(this.desiredPhi - dy * ORBIT.rotateSpeed, ORBIT.minPhi, ORBIT.maxPhi);
    this.onInteraction?.();
  };

  private readonly onPointerUp = (e: PointerEvent) => {
    const wasDragging = this.dragging;
    this.dragging = false;
    this.canvas.releasePointerCapture?.(e.pointerId);
    // A click that didn't drag → pick a node for focus/expand.
    if (wasDragging && !this.moved) {
      const rect = this.canvas.getBoundingClientRect();
      const ndcX = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      const ndcY = -((e.clientY - rect.top) / rect.height) * 2 + 1;
      const id = this.pick(ndcX, ndcY);
      if (id) this.onPick?.(id);
    }
  };

  private readonly onWheel = (e: WheelEvent) => {
    e.preventDefault();
    this.desiredRadius = clamp(
      this.desiredRadius * (1 + e.deltaY * ORBIT.zoomSpeed),
      ORBIT.minRadius,
      ORBIT.maxRadius,
    );
    this.onInteraction?.();
  };

  private attachPointer(): void {
    this.canvas.addEventListener("pointerdown", this.onPointerDown);
    this.canvas.addEventListener("pointermove", this.onPointerMove);
    this.canvas.addEventListener("pointerup", this.onPointerUp);
    this.canvas.addEventListener("wheel", this.onWheel, { passive: false });
  }

  private detachPointer(): void {
    this.canvas.removeEventListener("pointerdown", this.onPointerDown);
    this.canvas.removeEventListener("pointermove", this.onPointerMove);
    this.canvas.removeEventListener("pointerup", this.onPointerUp);
    this.canvas.removeEventListener("wheel", this.onWheel);
  }

  /** Raycast a normalized-device coordinate to the nearest node id. */
  private pick(ndcX: number, ndcY: number): string | null {
    if (!this.nodeMesh) return null;
    this.raycaster.setFromCamera(new Vector2(ndcX, ndcY), this.camera);
    const hits = this.raycaster.intersectObject(this.nodeMesh);
    if (hits.length === 0) return null;
    const instanceId = hits[0].instanceId;
    if (instanceId == null) return null;
    return this.nodes[instanceId]?.id ?? null;
  }

  private disposeMeshes(): void {
    if (this.nodeMesh) {
      this.scene.remove(this.nodeMesh);
      this.nodeMesh.geometry.dispose();
      (this.nodeMesh.material as MeshStandardMaterial).dispose();
      this.nodeMesh.dispose();
      this.nodeMesh = null;
    }
    for (const seg of [this.realEdges, this.predictedEdges]) {
      if (!seg) continue;
      this.scene.remove(seg);
      seg.geometry.dispose();
      (seg.material as LineBasicMaterial).dispose();
    }
    this.realEdges = null;
    this.predictedEdges = null;
  }

  /** Release ALL GPU resources + listeners (§5.4 unload on Space exit). */
  dispose(): void {
    this.detachPointer();
    this.disposeMeshes();
    this.renderer.dispose();
    this.positions.clear();
    this.idToIndex.clear();
    this.nodes = [];
    this.edges = [];
  }
}
