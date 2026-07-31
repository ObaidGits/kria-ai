# Offline/Pressure Traces — F5.5 (Deferred)

**Status:** `PENDING_EXECUTION`

**Why deferred:**
Per `dev-context.md`, the heavy offline/pressure fault-injection campaign (task 5.5.5 network-disconnect,
thermal, battery/power saver scenarios at hardware level) requires actual hardware runs.
This directory is the canonical evidence path per the F5.5 evidence schema.

**What exists:**
- Unit-level resource pressure evidence: `../../../performance/resource-trace.json` (V-RESOURCE-01 Pass)
- Quality-ladder degradation evidence: Section 9 model migration tests (S9.12 — Partial when vector unavailable)
- Burst queue overflow evidence: Section 4 outbox tests (S4.2 — dead-letter at RELAY_MAX_ATTEMPTS)
- Foreground preemption evidence: `../../../performance/resource-trace.json` (0.312ms vs 100ms budget)
- Full campaign report: `../../reports/resource-pressure-campaign.json`

**What is deferred:**
- Actual hardware thermal sensor / CPU governor / battery saver mode activation
- OS-level network interface disconnect with reconnect
- GPU/model compute pressure under concurrent local model load
- Trace captures from these hardware scenarios

**Acceptance criteria for completion (when hardware campaign runs):**
- Actual `perf stat` or equivalent CPU measurement confirming ≤1% overhead budget
- Network interface down/up sequence traces showing graceful offline→restore
- Battery/power-saver mode screenshot/log confirming quality ladder degradation
- Thermal throttle simulation log confirming scheduler degradation events fire
