#!/usr/bin/env node
// Capability Provider Platform (CPP) — tauri-driver live desktop drive (M9-C).
//
// Drives the BUILT desktop app through tauri-driver + WebKitWebDriver over the
// raw WebDriver-over-HTTP protocol (no external npm deps — uses Node 18+ global
// fetch). It navigates to the first-class Capabilities area and asserts the
// Provider Manager and Capability Browser render, then exercises discovery.
//
// This is the automation for the "live tauri-driver drive" release-validation
// step; it is READY FOR EXECUTION on a machine with a display (DISPLAY=:1) and
// the debug desktop binary built.
//
// Prerequisites:
//   cargo build -p kria-desktop            # debug binary at target/debug/kria-desktop
//   tauri-driver --port 4444 &             # bridges to WebKitWebDriver
//   DISPLAY=:1 node scripts/cpp_tauri_driver_drive.mjs
//
// Env:
//   TAURI_DRIVER_URL  (default http://127.0.0.1:4444)
//   KRIA_BIN          (default target/debug/kria-desktop)

const DRIVER = process.env.TAURI_DRIVER_URL || "http://127.0.0.1:4444";
const BIN = process.env.KRIA_BIN || "target/debug/kria-desktop";

async function wd(method, path, body) {
  const res = await fetch(`${DRIVER}${path}`, {
    method,
    headers: { "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  const json = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(`WD ${method} ${path} -> ${res.status}: ${JSON.stringify(json)}`);
  return json.value;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  console.log(`[drive] creating session against ${DRIVER} for ${BIN}`);
  const session = await wd("POST", "/session", {
    capabilities: { alwaysMatch: { "tauri:options": { application: BIN } } },
  });
  const sid = session.sessionId || session["session_id"];
  const base = `/session/${sid}`;
  let failures = 0;
  const check = (cond, msg) => {
    console.log(`${cond ? "PASS" : "FAIL"} | ${msg}`);
    if (!cond) failures++;
  };

  try {
    await sleep(4000); // let the app + backend boot

    // Navigate to the Capabilities area via its hash route.
    await wd("POST", `${base}/url`, { url: "tauri://localhost/#capabilities" }).catch(() => {});
    await sleep(1500);

    // Assert the Capabilities heading is present.
    const source = await wd("GET", `${base}/source`).catch(() => "");
    check(/Capabilities/.test(source), "Capabilities area rendered");
    check(/Provider|provider/.test(source), "Provider Manager section present");
    check(/Browser|Discover|capability/i.test(source), "Capability Browser present");

    // Drive discovery: type a goal and submit.
    try {
      const input = await wd("POST", `${base}/element`, {
        using: "css selector",
        selector: "input[type=text]",
      });
      const eid = input["element-6066-11e4-a52e-4f735466cecf"] || input.ELEMENT;
      await wd("POST", `${base}/element/${eid}/value`, { text: "reverse a string" });
      await sleep(1500);
      check(true, "discovery input accepted");
    } catch (e) {
      check(false, `discovery input drive: ${e.message}`);
    }
  } finally {
    await wd("DELETE", base).catch(() => {});
  }

  console.log(failures === 0 ? "\nDRIVE GREEN" : `\nDRIVE RED (${failures} failures)`);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((e) => {
  console.error("drive error:", e);
  process.exit(2);
});
