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

const CYAN = 0x35e3ff;
const VIOLET = 0x9a7bff;
const BLUE = 0x3b82ff;

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
  const segments = 96;
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
      renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, powerPreference: "low-power" });
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

    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    renderer.setSize(width(), height());
    renderer.setClearColor(0x000000, 0);
    host.appendChild(renderer.domElement);
    renderer.domElement.style.width = "100%";
    renderer.domElement.style.height = "100%";
    renderer.domElement.style.display = "block";

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(45, width() / height(), 0.1, 100);
    camera.position.set(0, 0.5, 5.4);
    camera.lookAt(0, 0, 0);

    const root = new THREE.Group();
    scene.add(root);

    const disposables: Array<{ dispose(): void }> = [];
    const track = <T extends { dispose(): void }>(o: T): T => {
      disposables.push(o);
      return o;
    };

    // Wireframe sphere.
    const sphereGeo = track(new THREE.SphereGeometry(1.9, 38, 28));
    const wireGeo = track(new THREE.WireframeGeometry(sphereGeo));
    const wireMat = track(new THREE.LineBasicMaterial({ color: CYAN, transparent: true, opacity: 0.26 }));
    root.add(new THREE.LineSegments(wireGeo, wireMat));

    // Layered volumetric glow — violet core, electric-blue halo, cyan rim.
    const glowGeo = track(new THREE.SphereGeometry(1.66, 28, 22));
    const glowMat = track(new THREE.MeshBasicMaterial({ color: VIOLET, transparent: true, opacity: 0.26, blending: THREE.AdditiveBlending }));
    const glowMesh = new THREE.Mesh(glowGeo, glowMat);
    glowMesh.position.x = 0.35; // bias violet energy to the right hemisphere (reference)
    root.add(glowMesh);
    const haloGeo = track(new THREE.SphereGeometry(2.02, 28, 22));
    const haloMat = track(new THREE.MeshBasicMaterial({ color: BLUE, transparent: true, opacity: 0.12, blending: THREE.AdditiveBlending }));
    root.add(new THREE.Mesh(haloGeo, haloMat));
    const rimGeo = track(new THREE.SphereGeometry(2.24, 28, 22));
    const rimMat = track(new THREE.MeshBasicMaterial({ color: CYAN, transparent: true, opacity: 0.06, blending: THREE.AdditiveBlending }));
    root.add(new THREE.Mesh(rimGeo, rimMat));

    // Dense surface points.
    const ptsGeo = track(new THREE.BufferGeometry());
    ptsGeo.setAttribute("position", new THREE.BufferAttribute(spherePoints(820, 1.92), 3));
    const ptsMat = track(new THREE.PointsMaterial({ color: CYAN, size: 0.034, transparent: true, opacity: 0.95, sizeAttenuation: true }));
    root.add(new THREE.Points(ptsGeo, ptsMat));

    // Tilted orbit rings + travelling nodes (four bands for a richer field).
    const orbits: Array<{ group: THREE.Group; node: THREE.Mesh; speed: number; phase: number; radius: number }> = [];
    const ringDefs = [
      { r: 2.7, tiltX: 1.2, tiltZ: 0.2, color: CYAN, speed: 0.5 },
      { r: 3.25, tiltX: -0.6, tiltZ: 0.9, color: VIOLET, speed: -0.35 },
      { r: 2.4, tiltX: 0.4, tiltZ: -1.1, color: BLUE, speed: 0.7 },
      { r: 2.95, tiltX: 1.5, tiltZ: -0.5, color: CYAN, speed: -0.5 },
    ];
    for (const def of ringDefs) {
      const g = new THREE.Group();
      g.rotation.x = def.tiltX;
      g.rotation.z = def.tiltZ;
      g.add(makeOrbit(def.r, def.color));
      const nodeGeo = track(new THREE.SphereGeometry(0.07, 12, 12));
      const nodeMat = track(new THREE.MeshBasicMaterial({ color: def.color }));
      const node = new THREE.Mesh(nodeGeo, nodeMat);
      g.add(node);
      root.add(g);
      orbits.push({ group: g, node, speed: def.speed, phase: Math.random() * Math.PI * 2, radius: def.r });
    }

    const reduced = prefersReducedMotion();
    let raf = 0;
    let running = false;
    const clock = new THREE.Clock();

    const renderFrame = () => {
      const t = clock.getElapsedTime();
      root.rotation.y = t * 0.12;
      root.rotation.x = Math.sin(t * 0.15) * 0.08;
      for (const o of orbits) {
        const a = o.phase + t * o.speed;
        o.node.position.set(Math.cos(a) * o.radius, 0, Math.sin(a) * o.radius);
      }
      renderer.render(scene, camera);
    };

    const loop = () => {
      if (!running) return;
      renderFrame();
      raf = requestAnimationFrame(loop);
    };
    const start = () => {
      if (running || reduced) return;
      running = true;
      clock.getDelta();
      raf = requestAnimationFrame(loop);
    };
    const stop = () => {
      running = false;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
    };

    const onVisibility = () => (document.hidden ? stop() : start());
    window.addEventListener("blur", stop);
    window.addEventListener("focus", start);
    document.addEventListener("visibilitychange", onVisibility);

    const resize = () => {
      const w = width();
      const h = height();
      renderer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      if (!running) renderFrame();
    };
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(resize) : undefined;
    ro?.observe(host);

    // First paint, then animate (unless reduced-motion → single static frame).
    renderFrame();
    if (!reduced) start();

    onCleanup(() => {
      stop();
      window.removeEventListener("blur", stop);
      window.removeEventListener("focus", start);
      document.removeEventListener("visibilitychange", onVisibility);
      ro?.disconnect();
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
