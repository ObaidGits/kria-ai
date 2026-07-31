import pw from "/media/obaid/SSD/KRIA/ui/node_modules/playwright-core/index.js";
const { chromium } = pw;
const root = "/media/obaid/SSD/KRIA/prototypes/focus-orbit";
const errors = [], shots = [];
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
page.on("pageerror", error => errors.push("page: " + error.message));
page.on("console", message => { if (message.type() === "error") errors.push("console: " + message.text()); });
await page.goto("file://" + root + "/index.html");
await page.waitForTimeout(500);
const snap = async name => { await page.screenshot({ path: root + "/preview/" + name + ".png" }); shots.push(name); };
const assert = (condition, message) => { if (!condition) throw new Error(message); };

const modes = {};
for (const mode of ["search", "ego", "path", "temporal", "grouped"]) {
  await page.evaluate(value => switchStrategy(value), mode);
  await page.waitForTimeout(100);
  modes[mode] = await page.evaluate(() => ({
    nodes: nodes.length, edges: edges.length,
    signature: nodes.map(node => [node.kind, Math.round(node.x3), Math.round(node.y3)])
  }));
}
const signatures = new Set(Object.values(modes).map(value => JSON.stringify(value.signature))).size;
assert(signatures === 5, "all five layout strategies must remain distinct");

async function generate(count) {
  await page.click(`[data-generate="${count}"]`);
  await page.waitForFunction(() => ["complete", "error"].includes(state.synthetic.status), null, { timeout: 120000 });
  const result = await page.evaluate(() => ({
    synthetic: state.synthetic,
    visible: nodes.filter(node => node.kind !== "more").length,
    cap: state.density,
    generatedSamples: Object.fromEntries(Object.entries(CORPUS).map(([key, rows]) => [key, rows.filter(row => row.id.startsWith("synthetic:")).length])),
    sceneUsesSample: nodes.some(node => node.item?.id?.startsWith("synthetic:")),
    progress: +document.getElementById("synthetic-progress").value,
    status: document.getElementById("synthetic-status").textContent,
    metrics: document.getElementById("synthetic-metrics").textContent,
    workerActive: syntheticController.workerActive()
  }));
  assert(result.synthetic.status === "complete", `${count} generation failed: ${result.status}`);
  return result;
}

const tenKFirst = await generate(10000);
const firstChecksum = tenKFirst.synthetic.summary.checksum;
const tenKSecond = await generate(10000);
assert(tenKSecond.synthetic.summary.checksum === firstChecksum, "same count/seed must produce the same checksum");
assert(tenKSecond.synthetic.summary.edgeCount >= 30000 && tenKSecond.synthetic.summary.edgeCount <= 80000, "10k graph must contain 3–8 relations per node");
assert(tenKSecond.synthetic.progressMessages >= 3 && tenKSecond.progress === 100, "worker must report phased progress through completion");
assert(Object.values(tenKSecond.generatedSamples).every(count => count >= 24), "each category must return enough representative records for density 24");

// Cancellation must terminate the active worker and ignore late fallback/load events.
await page.click('[data-generate="1000000"]');
await page.click("#synthetic-reset");
await page.waitForTimeout(150);
const cancelled = await page.evaluate(() => ({
  status: state.synthetic.status, active: syntheticController.workerActive(),
  generatedName: CORPUS.knowledge[0].name.startsWith("Synthetic ")
}));
assert(cancelled.status === "idle" && !cancelled.active && !cancelled.generatedName, "cancel/reset must terminate and release generated data");

const million = await generate(1000000);
const summary = million.synthetic.summary;
assert(summary.nodeCount === 1000000, "must generate one million actual records");
assert(summary.edgeCount >= 3000000 && summary.edgeCount <= 8000000, "1M graph must retain millions of real relations");
assert(summary.clusterCount >= 100 && summary.clusterCount <= 500, "cluster count must stay in the requested range");
assert(summary.bytes > 30000000 && summary.mib > 28, "worker metrics must describe retained typed-array storage");
assert(summary.workerHeld && million.workerActive && /worker-held/.test(million.status), "completed compact arrays must remain worker-held");
assert(million.sceneUsesSample && million.visible <= million.cap + 9, "scene must use generated samples while remaining bounded by density plus aggregate hubs");
assert(Object.values(summary.categoryTotals).reduce((sum, value) => sum + value, 0) === 1000000, "category totals must sum to node count");
assert(million.metrics.includes(summary.checksum) && million.metrics.includes("MiB"), "UI must report checksum and memory metrics");
await snap("lab-synthetic-1m");

// Preserve path/no-path, 3D, focus return, mirror, and state-preview behavior.
await page.evaluate(() => { switchStrategy("path"); state.pathNoResult = false; layout(); });
const pathEdges = await page.evaluate(() => edges.length);
await page.evaluate(() => { state.pathNoResult = true; layout(); });
const noPathEdges = await page.evaluate(() => edges.length);
assert(pathEdges > 0 && noPathEdges === 0, "path and honest no-path states must remain intact");
await page.evaluate(() => { switchStrategy("ego"); document.getElementById("btn-3d").click(); });
await page.waitForTimeout(1300);
const threeD = await page.evaluate(() => ({ mode: state.mode3d, lift: state.lift, z: nodes.some(node => node.z3 !== 0) }));
assert(threeD.mode && threeD.lift > 0.99 && threeD.z, "3D recency lift must remain intact");
await page.locator("#btn-command").focus();
await page.locator("#btn-command").click();
await page.locator("#cmd-input").fill("temporal");
await page.keyboard.press("Escape");
assert(await page.evaluate(() => document.activeElement?.id === "btn-command"), "command palette must return focus");
await page.locator("#mirror-toggle").click();
const mirror = await page.evaluate(() => ({
  visible: document.getElementById("graph-mirror").classList.contains("preview"),
  count: document.querySelectorAll("#sr-list button").length
}));
assert(mirror.visible && mirror.count > 0, "accessible DOM mirror must remain populated");
await page.evaluate(() => showPreview("degraded"));
await page.locator("[data-close-preview]").click();
assert(await page.evaluate(() => !document.getElementById("state-preview").classList.contains("show")), "state preview must close");

await page.click("#synthetic-reset");
const released = await page.evaluate(() => ({ status: state.synthetic.status, active: syntheticController.workerActive(), total: state.scaleTotal }));
assert(released.status === "idle" && !released.active && released.total === 878, "final reset must release worker-held arrays");
assert(errors.length === 0, "console/page errors: " + errors.join(" | "));
console.log(JSON.stringify({
  errors,
  modes: Object.fromEntries(Object.entries(modes).map(([key, value]) => [key, { nodes: value.nodes, edges: value.edges }])),
  deterministic10kChecksum: firstChecksum,
  million: summary,
  boundedVisible: million.visible,
  workerKind: million.synthetic.workerKind,
  cancelled,
  pathEdges, noPathEdges, threeD, mirror, released, shots
}, null, 2));
await browser.close();
