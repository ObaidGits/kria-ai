# OpenClaw — Performance Review

> Cold start, warm pool, container/image reuse, caching, parallelism, dependency install,
> workspace reuse, streaming latency, bottlenecks.

## 1. As-built performance model

- **Warm pool:** `warm_per_class = 2` (config), pre-warmed at boot for Light/Medium/Heavy;
  background loop keeps **Light** at target every 5s. Adopt-on-boot reuses survivors.
- **Per invocation:** checkout warm container → `mkdir` workspace → **spawn `docker attach`
  CLI process** → MCP handshake → `tools/call` → **destroy container** → async prewarm one.
- **Concurrency:** semaphore `max_concurrent_invocations = 4`, `try_acquire` (no queue).

## 2. Findings

### PERF-1 (High) — Destroy-per-invocation defeats the warm pool
Every call force-removes the container and prewarms a replacement asynchronously. Under bursty
load the pool drains faster than it refills (only Light is background-refilled), so tail calls
pay **cold create+start** (~hundreds of ms–seconds) instead of warm reuse. Isolation is
excellent but throughput/latency suffer.
**Options:** (a) keep destroy-per-invocation for Untrusted only; for Verified reuse a container
across N invocations with workspace wipe; (b) refill all classes in the background loop, not
just Light; (c) size warm pool to `max_concurrent + headroom`.

### PERF-2 (High) — `docker attach` CLI subprocess per call
Spawning the `docker` binary per invocation adds process-fork + Docker CLI startup overhead and
a second failure surface, versus using the already-present **bollard** attach/exec API in-process.
(Also the source of the `--no-stdin` correctness bug — see pipeline Defect 7.)
**Fix:** use bollard `attach_container`/`exec` streams directly; removes the subprocess and the
bug in one change.

### PERF-3 (Medium) — No streaming → perceived latency
`call_tool` is request→single response with a hard timeout. Long skills show nothing until done
(or until the 30s loop timeout). No partial output to UI.
**Fix:** stream MCP content blocks / container stdout incrementally to `StreamEvent`s.

### PERF-4 (Medium) — Fixed 30s loop timeout throttles Heavy skills
Loop wraps dispatch at 30s while Heavy/media budget is 120s (pipeline Defect 3). Heavy skills
can't complete.
**Fix:** pass `resource_profile.timeout_secs` through as the dispatch timeout.

### PERF-5 (Medium) — No image/layer caching strategy for skills
With SKL-1/SKL-7, any real dependency story risks per-run installs. Dependencies must be
resolved at **install** time into a cached bundle/layer, never per invocation, to keep runtime
fast and air-gapped.

### PERF-6 (Low) — Health check is an inspect round-trip per checkout
`get_or_create_warm` inspects each popped container. Fine at low scale; with event-driven
recycling (SBX-3) most inspects become unnecessary.

### PERF-7 (Low) — `mkdir` via exec adds a round-trip
Per-invocation workspace `mkdir` is an extra Docker exec. Could be created by the bridge on
first call or via a known path.

## 3. Latency budget (target, once fixed)

| Phase | Now (est.) | Target |
|-------|-----------|--------|
| Warm checkout | ~10–50ms (or cold seconds on drain) | <20ms warm, cold only on cold-miss |
| Attach + MCP handshake | subprocess + handshake (~50–150ms) | <30ms in-process bollard |
| First byte to UI | none until done | <500ms via streaming |
| Light skill round-trip | broken today | <1.5s p95 |

## 4. Bottleneck ranking

1. Destroy-per-invocation + Light-only background refill (throughput/tail latency).
2. `docker attach` CLI subprocess (overhead + correctness).
3. No streaming (perceived latency).
4. Timeout mismatch (Heavy skills).

None are fundamental; the warm-pool concept is right. The wins are: bollard in-process exec,
tiered reuse, background refill for all classes, streaming, and correct timeout propagation.
