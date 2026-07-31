/**
 * Globe — the Command Center's 3D AI-Core hero (three.js).
 *
 * A single WebGL surface: a wireframe sphere + surface points + a soft inner
 * glow + tilted orbit rings with travelling nodes, rotating slowly. It is:
 *   • capability-gated — if WebGL is unavailable it renders a CSS ring fallback;
 *   • paused on blur / tab-hidden (no wasted frames);
 *   • reduced-motion aware — renders a single static frame, no rAF loop;
 *   • fully disposed (geometries/materials/renderer/context) on unmount.
 *
 * Frontend-only demo: no data, no stores.
 */
import { onCleanup, onMount, createSignal, Show } from "solid-js";
import * as THREE from "three";
import { coreState, type CoreState } from "./context";

const CYAN = 0x35e3ff;
const VIOLET = 0x9a7bff;
const BLUE = 0x3b82ff;
const GREEN = 0x42d392;

const STATE_COLOR: Record<CoreState, number> = {
  idle: CYAN,
  listening: CYAN,
  thinking: VIOLET,
  retrieving: BLUE,
  executing: GREEN,
};
const STATE_ENERGY: Record<CoreState, number> = {
  idle: 0.42,
  listening: 0.72,
  thinking: 1,
  retrieving: 0.86,
  executing: 1.12,
};

function prefersReducedMotion(): boolean {
  if (typeof document !== "undefined" && document.documentElement?.dataset.reducedMotion === "on") return true;
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    try {
      return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    } catch {
      return false;
    }
  }
  return false;
}

/** Random point cloud on a sphere surface. */
function spherePoints(count: number, radius: number): Float32Array {
  const arr = new Float32Array(count * 3);
  for (let i = 0; i < count; i += 1) {
    const u = Math.random();
    const v = Math.random();
    const theta = 2 * Math.PI * u;
    const phi = Math.acos(2 * v - 1);
    arr[i * 3] = radius * Math.sin(phi) * Math.cos(theta);
    arr[i * 3 + 1] = radius * Math.sin(phi) * Math.sin(theta);
    arr[i * 3 + 2] = radius * Math.cos(phi);
  }
  return arr;
}

function makeOrbit(radius: number, color: number): THREE.Line {
  const points: THREE.Vector3[] = [];
  const segments = 64;
  for (let i = 0; i <= segments; i += 1) {
    const a = (i / segments) * Math.PI * 2;
    points.push(new THREE.Vector3(Math.cos(a) * radius, 0, Math.sin(a) * radius));
  }
  const geo = new THREE.BufferGeometry().setFromPoints(points);
  const mat = new THREE.LineBasicMaterial({ color, transparent: true, opacity: 0.5 });
  return new THREE.Line(geo, mat);
}

export function Globe() {
  const [failed, setFailed] = createSignal(false);
  let host: HTMLDivElement | undefined;

  onMount(() => {
    if (!host) return;

    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, powerPreference: "high-performance" });
    } catch {
      setFailed(true);
      return;
    }
    if (!renderer.getContext()) {
      setFailed(true);
      return;
    }

    const width = () => host!.clientWidth || 480;
    const height = () => host!.clientHeight || 480;

    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.25));
    renderer.setSize(width(), height());
    renderer.setClearColor(0x000000, 0);
    host.appendChild(renderer.domElement);
    renderer.domElement.style.width = "100%";
    renderer.domElement.style.height = "100%";
    renderer.domElement.style.display = "block";

    const scene = new THREE.Scene();
    // A wider FOV and bounded ring radii keep every cognition orbit inside the
    // canvas at short laptop heights instead of clipping at the hero edges.
    const camera = new THREE.PerspectiveCamera(48, width() / height(), 0.1, 100);
    camera.position.set(0, 0.35, 6.6);
    camera.lookAt(0, 0, 0);

    const root = new THREE.Group();
    scene.add(root);

    const disposables: Array<{ dispose(): void }> = [];
    const track = <T extends { dispose(): void }>(o: T): T => {
      disposables.push(o);
      return o;
    };

    // Wireframe sphere. Geometry is intentionally bounded for a continuously
    // animated homepage surface; visual richness comes from layering, not load.
    const sphereGeo = track(new THREE.SphereGeometry(1.9, 30, 22));
    const wireGeo = track(new THREE.WireframeGeometry(sphereGeo));
    const wireMat = track(new THREE.LineBasicMaterial({ color: CYAN, transparent: true, opacity: 0.26 }));
    root.add(new THREE.LineSegments(wireGeo, wireMat));

    // Layered volumetric glow — violet core, electric-blue halo, cyan rim.
    const glowGeo = track(new THREE.SphereGeometry(1.66, 22, 18));
    const glowMat = track(new THREE.MeshBasicMaterial({ color: VIOLET, transparent: true, opacity: 0.26, blending: THREE.AdditiveBlending }));
    const glowMesh = new THREE.Mesh(glowGeo, glowMat);
    glowMesh.position.x = 0.35; // bias violet energy to the right hemisphere (reference)
    root.add(glowMesh);
    const haloGeo = track(new THREE.SphereGeometry(2.02, 22, 18));
    const haloMat = track(new THREE.MeshBasicMaterial({ color: BLUE, transparent: true, opacity: 0.12, blending: THREE.AdditiveBlending }));
    root.add(new THREE.Mesh(haloGeo, haloMat));
    const rimGeo = track(new THREE.SphereGeometry(2.24, 22, 18));
    const rimMat = track(new THREE.MeshBasicMaterial({ color: CYAN, transparent: true, opacity: 0.06, blending: THREE.AdditiveBlending }));
    root.add(new THREE.Mesh(rimGeo, rimMat));

    // Surface detail stays dense enough to read as neural activity at normal
    // scale while avoiding excess fragment work on integrated laptop GPUs.
    const ptsGeo = track(new THREE.BufferGeometry());
    ptsGeo.setAttribute("position", new THREE.BufferAttribute(spherePoints(560, 1.92), 3));
    const ptsMat = track(new THREE.PointsMaterial({ color: CYAN, size: 0.034, transparent: true, opacity: 0.95, sizeAttenuation: true }));
    root.add(new THREE.Points(ptsGeo, ptsMat));

    // Tilted orbit rings + travelling nodes. Radii stay within the camera's
    // fitted frame, so each ring remains visually complete at every breakpoint.
    const orbits: Array<{ group: THREE.Group; node: THREE.Mesh; speed: number; phase: number; radius: number }> = [];
    const ringDefs = [
      { r: 2.42, tiltX: 1.2, tiltZ: 0.2, color: CYAN, speed: 0.42 },
      { r: 2.62, tiltX: -0.6, tiltZ: 0.9, color: VIOLET, speed: -0.3 },
      { r: 2.3, tiltX: 0.4, tiltZ: -1.1, color: BLUE, speed: 0.52 },
      { r: 2.52, tiltX: 1.5, tiltZ: -0.5, color: CYAN, speed: -0.38 },
    ];
    for (const def of ringDefs) {
      const g = new THREE.Group();
      g.rotation.x = def.tiltX;
      g.rotation.z = def.tiltZ;
      g.add(makeOrbit(def.r, def.color));
      const nodeGeo = track(new THREE.SphereGeometry(0.07, 8, 8));
      const nodeMat = track(new THREE.MeshBasicMaterial({ color: def.color }));
      const node = new THREE.Mesh(nodeGeo, nodeMat);
      g.add(node);
      root.add(g);
      orbits.push({ group: g, node, speed: def.speed, phase: Math.random() * Math.PI * 2, radius: def.r });
    }

    const reduced = prefersReducedMotion();
    const frameInterval = 1000 / 30;
    let raf = 0;
    let running = false;
    let intersecting = true;
    let lastFrameAt = 0;
    const clock = new THREE.Clock();

    const renderFrame = () => {
      const t = clock.getElapsedTime();
      const state = coreState();
      const energy = STATE_ENERGY[state];
      const pulse = (Math.sin(t * (0.8 + energy * 0.7)) + 1) * 0.5;
      const stateColor = STATE_COLOR[state];

      root.rotation.y = t * (0.055 + energy * 0.055);
      root.rotation.x = Math.sin(t * 0.13) * (0.035 + energy * 0.035);
      root.scale.setScalar(1 + pulse * 0.008 * energy);
      wireMat.color.setHex(stateColor);
      wireMat.opacity = 0.14 + energy * 0.15;
      ptsMat.color.setHex(stateColor);
      ptsMat.opacity = 0.38 + energy * 0.5;
      ptsMat.size = 0.026 + energy * 0.009;
      glowMat.opacity = 0.16 + energy * 0.11 + pulse * 0.025;
      haloMat.opacity = 0.055 + energy * 0.07;
      rimMat.color.setHex(stateColor);
      rimMat.opacity = 0.025 + energy * 0.045;

      for (const o of orbits) {
        const a = o.phase + t * o.speed * (0.65 + energy * 0.45);
        o.node.position.set(Math.cos(a) * o.radius, 0, Math.sin(a) * o.radius);
        o.node.scale.setScalar(0.78 + pulse * 0.34 * energy);
      }
      renderer.render(scene, camera);
    };

    // Thirty frames per second is smooth for this deliberately slow ambient
    // motion and leaves more compositor time for immediate hover/touch feedback.
    const loop = (timestamp: number) => {
      if (!running) return;
      const elapsed = timestamp - lastFrameAt;
      if (elapsed >= frameInterval) {
        lastFrameAt = timestamp - (elapsed % frameInterval);
        renderFrame();
      }
      raf = requestAnimationFrame(loop);
    };
    const start = () => {
      if (running || reduced || !intersecting || document.hidden) return;
      running = true;
      lastFrameAt = 0;
      clock.getDelta();
      raf = requestAnimationFrame(loop);
    };
    const stop = () => {
      running = false;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
    };
    const syncRunning = () => {
      if (!document.hidden && intersecting) start();
      else stop();
    };

    const onVisibility = () => syncRunning();
    window.addEventListener("blur", stop);
    window.addEventListener("focus", syncRunning);
    document.addEventListener("visibilitychange", onVisibility);

    const io = typeof IntersectionObserver !== "undefined"
      ? new IntersectionObserver(([entry]) => {
          intersecting = entry?.isIntersecting ?? true;
          syncRunning();
        }, { threshold: 0.05 })
      : undefined;
    io?.observe(host);

    let resizeTimer: number | undefined;
    const resize = () => {
      const w = width();
      const h = height();
      renderer.setSize(w, h, false);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      if (!running) renderFrame();
    };
    // CSS layout transitions visually scale the existing canvas. Debouncing the
    // drawing-buffer resize avoids reallocating WebGL surfaces on every frame.
    const scheduleResize = () => {
      if (resizeTimer !== undefined) window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(() => {
        resizeTimer = undefined;
        resize();
      }, 72);
    };
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(scheduleResize) : undefined;
    ro?.observe(host);

    // First paint, then animate (unless reduced-motion → single static frame).
    renderFrame();
    if (!reduced) start();

    onCleanup(() => {
      stop();
      window.removeEventListener("blur", stop);
      window.removeEventListener("focus", syncRunning);
      document.removeEventListener("visibilitychange", onVisibility);
      io?.disconnect();
      ro?.disconnect();
      if (resizeTimer !== undefined) window.clearTimeout(resizeTimer);
      for (const d of disposables) {
        try {
          d.dispose();
        } catch {
          /* best-effort */
        }
      }
      renderer.dispose();
      renderer.domElement.remove();
    });
  });

  return (
    <div class="cc-globe" ref={host}>
      <Show when={failed()}>
        <div class="cc-globe-fallback" aria-hidden="true">
          <span class="cc-globe-fallback__ring" />
          <span class="cc-globe-fallback__ring cc-globe-fallback__ring--2" />
          <span class="cc-globe-fallback__ring cc-globe-fallback__ring--3" />
        </div>
      </Show>
    </div>
  );
}

export default Globe;
