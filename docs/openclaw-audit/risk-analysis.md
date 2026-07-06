# OpenClaw — Risk Analysis

> What breaks (and how badly) if each gap is shipped or deferred. Likelihood × Impact.

## 1. If shipped "as-is" (current state)

| Risk | Likelihood | Impact | Notes |
|------|-----------|--------|-------|
| Feature appears in UI but every skill fails | Certain | High | P0 Defects 7/8/9 — user tries OpenClaw, it never works → trust damage |
| "85% functional" label misleads planning | Certain | High | Real number ~35% wired, 0% e2e — roadmap decisions built on a false premise |
| Audit ledger provides false assurance | High | High | Dev HMAC key → tamper-evident claim is false (SEC-1) |
| Permission modal provides false assurance | High | High | approved_capabilities ignored (SEC-3) |

## 2. Per-gap deferral risk

### If A1 (stdin/exec) deferred
- **Impact: total.** No skill can run. Everything downstream is untestable. **Do not defer.**

### If A2 (real skills) deferred
- Impact: high. Substrate is a demo with nothing to demo; can't validate the pipeline.

### If B1 (vault key) deferred
- Impact: high, security. Any audit/trust claim is void; blocks a truthful security posture.
- Likelihood of exploit: low locally, but the *claim* of tamper-evidence is a liability.

### If B3 (capability materialization) deferred
- Impact: high, functional + trust. Either skills silently can't work, or a later hasty
  "just enable network/writes" bypasses the whole review model. This is where a **future
  security incident** is most likely to originate.

### If B4 (seccomp/pids/events) deferred
- Impact: medium-high. Fork-bomb/DoS within mem budget; container leaks on crash; larger
  escape surface for Community/Untrusted skills on a shared kernel.

### If B5 (HRA admission) deferred
- Impact: medium-high on low-tier hosts. A Heavy skill can starve realtime voice/vision —
  regressing the working GPU orchestrator the user is proud of. **Ties directly to the user's
  existing hardware/GPU work.**

### If B6 (signing) deferred
- Impact: medium now (catalog is empty), high later. Supply-chain compromise once a real
  ClawHub catalog exists. Safe to defer only while the catalog is bundled-only.

### If C1 (router unification) deferred
- Impact: medium now, high at scale. With many skills, tool soup returns and **directly
  worsens the "auto tool" misfire** the user already reports. Deferring past ~50 skills is
  the rework trap the user was worried about.

### If C4 (tiered runtimes) deferred
- Impact: medium. Acceptable while only Verified/reviewed-Community skills exist; **blocking**
  for prompt-generated/Untrusted skills (D3).

## 3. Rework-risk map (the user's core concern)

The changes that, if skipped now, force **re-touching every skill later**:

1. **Skill package contract + provisioning (SKL-1)** — defining bundle format late means
   re-authoring/repackaging all skills. **Decide the format before writing skills (A2).**
2. **Capability grant model (B3)** — retrofitting grants means re-reviewing every skill's
   permissions. Define once in Phase B.
3. **Router unification (C1)** — leaving two routers means editing routing per skill forever.
   Fix the contract once; skills self-describe via descriptor.
4. **Descriptor↔schema single source (SKL-6)** — if params live in two places, every skill
   drifts. Make the bundle schema authoritative.

Everything else (streaming, UI, telemetry, tiered runtimes) is **additive** and can land later
without touching existing skills.

## 4. Highest-priority risks (do first)

1. A1 — or the feature is dead. 
2. B1 + B3 — or "safe/private" is untrue.
3. A2 with the **final** skill-package format — or you repackage everything later.
4. B5 — or OpenClaw can regress the working voice/GPU stack.

## 5. Reversibility

All fixes are local and reversible (config-gated, `enabled=false` by default). The one
**hard-to-reverse** decision is the **skill-package format + capability grant model** — get
that right before authoring a catalog, because it is the contract every skill depends on.
