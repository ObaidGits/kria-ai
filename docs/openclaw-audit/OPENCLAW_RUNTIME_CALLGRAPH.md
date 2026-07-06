# OpenClaw Runtime Call Graph

Traces the authoritative runtime paths after A9. Every path converges on the single
execution + lifecycle stack — there is no bypass.

## 1. Executing an installed skill

```
Agent / ExecutionEngine.execute(goal)
  └─ ExecutionPlanner::plan            (execution/planner.rs)
  └─ DependencyResolver::validate      (execution/dependency.rs)
  └─ GraphOptimizer::optimize          (execution/optimizer.rs)
  └─ ExecutionScheduler::run           (execution/scheduler.rs)
       └─ ExecutorRegistry.get(OpenClaw)
            └─ OpenClawExecutor.execute (execution/executors/openclaw.rs)
                 └─ SkillRuntime.execute (runtime/docker.rs → DockerRuntime)
                      └─ RuntimeManager.checkout_container  (runtime_manager.rs)
                           └─ warm pool → HRA admission → container
                      └─ bridge JSON-RPC call → skill handler
                      └─ RuntimeManager.checkin_container / cleanup
```

## 2. Installing a bundle (manual, marketplace, or generated — identical)

```
BundleInstaller.install(bundle_dir)     (bundle/installer.rs)
  └─ Bundle::open + Bundle::verify       (bundle/mod.rs, bundle/verify.rs)
       └─ verify_hashes + ed25519 signature check
  └─ deps::resolve                       (bundle/deps.rs)
  └─ version::relation                   (bundle/version.rs)
  └─ ProductionSkillRegistry.install_skill (registry.rs)
  └─ SkillActivation.activate + reindex   (hot reload: ToolRegistry + semantic index)
```

## 3. Platform install (download → install)

```
RepositoryManager.download(slug)        (platform/repository.rs)
  └─ priority-ordered repos → failover → offline cache
  └─ TrustFramework.verify_policy()      (platform/trust.rs) → bundle::verify::TrustPolicy
  └─ BundleInstaller.install(...)        (path 2)
```

## 4. Autonomous generation (A9) → same install path

```
GenerationPipeline.run(goal)            (generation/pipeline.rs)
  └─ SkillGenerator.extract_requirements (generation/llm_generator.rs)
  └─ DecisionEngine.decide               (generation/decision.rs)
       ├─ Reuse  → return existing skill (no generation)
       └─ Generate ↓
  └─ SkillGenerator.design_skill         (+ infer_capabilities, classify_risk)
  └─ SkillGenerator.generate_code
  └─ codegen::emit_bundle → .ocskill dir (generation/codegen.rs)
  └─ SkillValidator.validate             (wraps Bundle::open)
  └─ SandboxTester.test                  (generation/sandbox.rs)
  └─ [repair loop: SkillGenerator.repair_code] (budget-guarded)
  └─ QualityEvaluator.evaluate
  └─ ApprovalLayer.may_install           (high-risk → await approval)
  └─ InstallSink.install  ──────────────► path 2 (BundleInstaller, FROZEN)
```

## 5. Update / sync

```
SyncEngine.sync(prev_state)             (platform/sync.rs)
  └─ RepositoryManager.refresh (merge by priority + semver, persist cache)
UpdateEngine.detect(installed)          (platform/updates.rs)
  └─ version::relation / is_breaking_change / publisher revocation
  └─ (apply) → BundleInstaller.install   (path 2)
```

## Convergence guarantee

Paths 1–5 all terminate in the same three owners:
`BundleInstaller` (install), `ProductionSkillRegistry` (state),
`ExecutionEngine → OpenClawExecutor → RuntimeManager` (execution).
No path constructs its own runtime, registry, or installer.
