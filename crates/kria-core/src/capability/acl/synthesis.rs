//! `SynthesisProvider` — the Wave 9 keystone (spec R7.1): a synthesizing
//! [`CapabilityProvider`] whose `acquire` **generates a new capability** from a
//! goal, then is installed/verified/smoke-tested/benchmarked/activated through
//! the IDENTICAL neutral lifecycle as any other provider — **no special-case
//! Brain code**. The Brain treats "generate" as just another provider's acquire.
//!
//! # Safety (spec R7.2/R7.3/R11.4)
//! Synthesis is bounded to the audited [`primitives`] set: a synthesized
//! capability is a *declared composition of pure primitives*, never generated
//! host code. So there is no arbitrary code to sandbox — the "synthesized code
//! never runs unsandboxed on the host" invariant holds trivially, and the
//! capability defaults to the **lowest trust tier** (`synthesized`). When a goal
//! is not expressible from the audited set the provider **honestly declines**
//! (R7.4) rather than fabricating an unverifiable capability. Richer code-gen
//! stages (compile in the seccomp-bound Docker sandbox) are the documented later
//! stages that build on this same neutral acquire path.
//!
//! Provider-neutral: emits only neutral [`CapabilityDescriptor`]/
//! [`CapabilityOutcome`]; holds no cognition (the Brain decided to synthesize).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::capability::descriptor::{CapabilityDescriptor, Effects, Reversibility, TrustInfo};
use crate::capability::error::CapError;
use crate::capability::intelligence::capability_graph::{IR_SCHEMA_VERSION, PRIMITIVE_SET_VERSION};
use crate::capability::intelligence::synthesis::CapabilitySpecification;
use crate::capability::protocol::{
    ClientCapabilities, Feature, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
};
use crate::capability::provider::{
    AcquireRequest, CapabilityOutcome, CapabilityProvider, CapabilityRequest,
};
use crate::capability::ProviderId;

/// The persisted record of a synthesized capability (its spec on disk) plus a
/// **provenance manifest** (W9-R4): the exact goal hash, IR hash, and policy /
/// primitive-set / IR-schema versions that produced it, so a synthesized
/// capability is auditable and reproducible (spec R7/R16/R24) — "re-synthesizing
/// this goal under the same versions yields this same IR hash".
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SynthesizedRecord {
    spec: CapabilitySpecification,
    created_at: String,
    /// The exact source goal that produced this capability — enables
    /// auto-regeneration / repair (W9-R10 / BLOCKER 5): re-synthesizing from the
    /// stored goal self-heals a corrupted spec and bumps the version.
    #[serde(default)]
    source_goal: String,
    /// blake3 of the (trimmed, lowercased) source goal.
    #[serde(default)]
    source_goal_hash: String,
    /// blake3 of the Capability-Graph IR (content-addressed artifact id).
    #[serde(default)]
    ir_hash: String,
    #[serde(default)]
    primitive_set_version: u32,
    #[serde(default)]
    ir_schema_version: u32,
    /// Monotonic version of this synthesized capability (bumped on re-synthesis /
    /// repair; W9-R10). First generation = 1.
    #[serde(default = "one")]
    version: u32,
}

fn one() -> u32 {
    1
}

/// A synthesizing capability provider. `acquire` generates a capability from the
/// goal (or the Brain-selected `capability_id`) and persists it to a store;
/// `execute` runs the synthesized primitive; the standard lifecycle facet
/// (acquire/remove) is advertised so it plugs into the neutral platform.
pub struct SynthesisProvider {
    id: ProviderId,
    store_dir: PathBuf,
    /// In-flight generation lock keyed by `capability_id` (W9-R7): collapses
    /// concurrent identical-goal syntheses to a single generation so two callers
    /// never double-generate / double-persist the same capability.
    in_flight: Mutex<HashSet<String>>,
}

impl SynthesisProvider {
    /// Create over a store directory for synthesized capability specs.
    pub fn new(id: impl Into<String>, store_dir: impl AsRef<Path>) -> Result<Self, CapError> {
        let store_dir = store_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&store_dir)
            .map_err(|e| CapError::Io(format!("synthesis store dir: {e}")))?;
        Ok(Self {
            id: id.into(),
            store_dir,
            in_flight: Mutex::new(HashSet::new()),
        })
    }

    fn spec_path(&self, capability_id: &str) -> PathBuf {
        self.store_dir.join(format!("{capability_id}.json"))
    }

    /// Persist a synthesized spec + its provenance manifest atomically (temp +
    /// rename) and return its descriptor. Split out of `acquire` so the in-flight
    /// lock (W9-R7) wraps exactly the generation/persist critical section.
    fn generate_and_persist(
        &self,
        goal: &str,
        spec: CapabilitySpecification,
    ) -> Result<CapabilityDescriptor, CapError> {
        let ir_hash = spec.ir_hash().unwrap_or_default();
        let source_goal_hash = blake3::hash(goal.trim().to_lowercase().as_bytes())
            .to_hex()
            .to_string();
        // Preserve a prior version number across re-synthesis (W9-R10): re-writing
        // an existing capability bumps its version; first generation is 1.
        let version = self
            .load(&spec.capability_id)
            .map(|r| r.version.saturating_add(1))
            .unwrap_or(1);
        let record = SynthesizedRecord {
            spec: spec.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            source_goal: goal.trim().to_string(),
            source_goal_hash,
            ir_hash,
            primitive_set_version: PRIMITIVE_SET_VERSION,
            ir_schema_version: IR_SCHEMA_VERSION,
            version,
        };
        let json = serde_json::to_string_pretty(&record)
            .map_err(|e| CapError::Acquire(format!("serialize spec: {e}")))?;
        // Atomic write: write to a temp file then rename, so a concurrent reader
        // never observes a partial spec (crash/concurrency safety).
        let final_path = self.spec_path(&spec.capability_id);
        let tmp_path = final_path.with_extension("json.tmp");
        std::fs::write(&tmp_path, json)
            .map_err(|e| CapError::Acquire(format!("synthesis store write: {e}")))?;
        std::fs::rename(&tmp_path, &final_path)
            .map_err(|e| CapError::Acquire(format!("synthesis store commit: {e}")))?;
        Ok(self.descriptor_from(&spec, true))
    }

    fn load(&self, capability_id: &str) -> Option<SynthesizedRecord> {
        let text = std::fs::read_to_string(self.spec_path(capability_id)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn descriptor_from(
        &self,
        spec: &CapabilitySpecification,
        installed: bool,
    ) -> CapabilityDescriptor {
        // Multi-input capabilities (W9-R9) declare a typed schema over their named
        // reducer inputs; single-input capabilities keep the `{text}` schema.
        let input_schema = if spec.reducer.is_some() && !spec.input_keys.is_empty() {
            let mut props = serde_json::Map::new();
            for k in &spec.input_keys {
                props.insert(k.clone(), serde_json::json!({ "type": "string" }));
            }
            serde_json::json!({
                "type": "object",
                "properties": props,
                "required": spec.input_keys.clone(),
            })
        } else {
            serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            })
        };
        let mut d = CapabilityDescriptor::minimal(
            self.id.clone(),
            &spec.capability_id,
            &spec.name,
            &spec.purpose,
            input_schema,
        );
        d.version = "1.0.0".into();
        // IO derived from the IR graph boundary (W9-R9 multi-input foundation):
        // a pure text pipeline is text→text; a multi-input reducer declares its
        // named inputs; a graph with non-text boundary nodes declares real
        // modalities. Falls back to text for pre-IR records.
        let graph = spec.normalized_graph();
        let (in_types, out_types) = if spec.reducer.is_some() && !spec.input_keys.is_empty() {
            (spec.input_keys.clone(), vec!["text".into()])
        } else {
            match &graph {
                Some(g) => (g.boundary_inputs(), g.boundary_outputs()),
                None => (vec!["text".into()], vec!["text".into()]),
            }
        };
        d.io_modality = vec!["text".into()];
        d.inputs = in_types;
        d.outputs = out_types;
        // Synthesized capabilities NEVER bypass permission (spec R7.2): declare a
        // conservative/elevated effect profile so the permission engine requires
        // explicit approval on first execution, even though pure primitives are
        // read-only. The effect set is `["synthesized"]` UNIONED with the IR's
        // per-node effect union at max risk (W9-R6, spec R11.1) — so a composed
        // capability that reaches an escalating capability node cannot launder
        // that effect away. Trust is earned via approval + benchmark, never
        // granted on creation.
        let mut classes = vec!["synthesized".to_string()];
        if let Some(g) = &graph {
            for c in g.effects_union() {
                if !classes.contains(&c) {
                    classes.push(c);
                }
            }
        }
        d.effects = Effects {
            classes,
            reversible: Reversibility::Unknown,
            idempotent: true,
            resource_class: Default::default(),
        };
        // Lowest trust — synthesized capabilities are never trusted on creation.
        d.trust = TrustInfo {
            publisher: Some(self.id.clone()),
            signed: false,
            tier: Some("synthesized".into()),
        };
        d.extensions.insert(
            "kind".into(),
            serde_json::Value::String("synthesized".into()),
        );
        d.extensions
            .insert("synthesized".into(), serde_json::Value::Bool(true));
        d.extensions
            .insert("installed".into(), serde_json::Value::Bool(installed));
        d.extensions.insert(
            "primitive".into(),
            serde_json::Value::String(spec.primitive.clone()),
        );
        // Declared smoke test = the golden case, so pre-activation smoke actually
        // EXECUTES the synthesized capability (real liveness gate, spec R21/R7.2),
        // not a no-op pass. Multi-input goldens are the full named-args object;
        // single-input goldens wrap the string as `{text}`.
        let smoke_args = if spec.reducer.is_some() {
            serde_json::from_str::<serde_json::Value>(&spec.golden_input)
                .unwrap_or_else(|_| serde_json::json!({ "text": spec.golden_input }))
        } else {
            serde_json::json!({ "text": spec.golden_input })
        };
        d.extensions
            .insert("smoke".into(), serde_json::json!({ "args": smoke_args }));
        // Declare the sandbox/host requirement for provenance (R11.4): a pure
        // primitive needs no host code, but future code-gen stages declare docker.
        let sandbox = match &graph {
            Some(g) if g.is_pure_primitive() => "pure-primitive",
            Some(_) => "composed-capabilities",
            None => "pure-primitive",
        };
        d.extensions
            .insert("sandbox".into(), serde_json::Value::String(sandbox.into()));
        // Provenance: the content-addressed IR hash so the descriptor itself
        // carries a reproducibility fingerprint (W9-R4, spec R7/R16/R24), plus
        // the serialized IR graph so the neutral platform can execute a composed
        // (capability-node) graph without reaching into the provider store
        // (W9-R8). Pure-primitive graphs still execute in-process in the provider.
        if let Some(g) = &graph {
            d.extensions
                .insert("ir_hash".into(), serde_json::Value::String(g.hash()));
            d.extensions.insert(
                "ir_schema_version".into(),
                serde_json::Value::from(IR_SCHEMA_VERSION),
            );
            if let Ok(gv) = serde_json::to_value(g) {
                d.extensions.insert("ir_graph".into(), gv);
            }
        }
        d
    }
}

#[async_trait]
impl CapabilityProvider for SynthesisProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }

    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        // Advertise the LIFECYCLE facet: acquire = generate, remove = delete.
        Ok(client.negotiate(
            self.id.clone(),
            ProtocolVersion::CURRENT,
            FeatureSet::mandatory().with(Feature::Lifecycle),
            serde_json::Map::new(),
        ))
    }

    async fn describe(&self, _s: &ProtocolSession) -> Result<Vec<CapabilityDescriptor>, CapError> {
        // Installed = previously-synthesized capabilities persisted in the store.
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.store_dir) {
            for e in entries.flatten() {
                if e.path().extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(text) = std::fs::read_to_string(e.path()) {
                        if let Ok(rec) = serde_json::from_str::<SynthesizedRecord>(&text) {
                            out.push(self.descriptor_from(&rec.spec, true));
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    async fn acquire(&self, req: &AcquireRequest) -> Result<CapabilityDescriptor, CapError> {
        // GENERATE a capability from the goal (spec R7.1). If the Brain already
        // synthesized this exact id (idempotent re-acquire), return it.
        if let Some(chosen) = req.capability_id.as_deref() {
            if let Some(rec) = self.load(chosen) {
                // Re-acquire by id = **repair / auto-regeneration** (W9-R10 /
                // BLOCKER 5): re-synthesize deterministically from the stored
                // source goal (self-heals a corrupted spec, bumps the version).
                // Honors a Brain-proposed graph when supplied (e.g. an improved
                // LLM proposal at repair time). Falls back to returning the
                // existing spec unchanged if regeneration no longer matches the id.
                let goal = req
                    .hint
                    .clone()
                    .filter(|h| !h.trim().is_empty())
                    .unwrap_or_else(|| rec.source_goal.clone());
                let regenerated = req
                    .proposed_graph
                    .as_ref()
                    .and_then(|v| {
                        serde_json::from_value::<crate::capability::intelligence::CapabilityGraph>(
                            v.clone(),
                        )
                        .ok()
                    })
                    .and_then(|g| CapabilitySpecification::from_graph(&goal, g))
                    .or_else(|| CapabilitySpecification::from_goal(&goal));
                if let Some(spec) = regenerated {
                    if spec.capability_id == chosen {
                        return self.generate_and_persist(&goal, spec);
                    }
                }
                return Ok(self.descriptor_from(&rec.spec, true));
            }
        }
        let goal = req
            .hint
            .clone()
            .unwrap_or_else(|| req.capability_tag.clone());
        // Prefer a Brain-proposed IR (deterministic or LLM-assisted, W9-R11): the
        // provider RE-VALIDATES it (safety, not cognition) via `from_graph` and
        // persists it. Falls back to deterministic goal→IR derivation when no
        // proposal is supplied (parity). Honest-decline if neither is expressible.
        let spec = match req
            .proposed_graph
            .as_ref()
            .and_then(|v| {
                serde_json::from_value::<crate::capability::intelligence::CapabilityGraph>(
                    v.clone(),
                )
                .ok()
            })
            .and_then(|g| CapabilitySpecification::from_graph(&goal, g))
        {
            Some(s) => s,
            None => CapabilitySpecification::from_goal(&goal).ok_or_else(|| {
                CapError::Acquire(format!(
                    "cannot synthesize a capability for '{goal}': not expressible from the \
                     audited primitive set (honest decline — no fabricated capability)"
                ))
            })?,
        };

        // W9-R7 in-flight lock: collapse concurrent identical-goal syntheses. If
        // another call is already generating this exact capability, or it already
        // exists on disk, return the existing artifact (idempotent, no double
        // generation / double persist).
        {
            let mut guard = self
                .in_flight
                .lock()
                .map_err(|_| CapError::Acquire("synthesis lock poisoned".into()))?;
            if guard.contains(&spec.capability_id) {
                drop(guard);
                if let Some(rec) = self.load(&spec.capability_id) {
                    return Ok(self.descriptor_from(&rec.spec, true));
                }
                return Err(CapError::Acquire(format!(
                    "capability '{}' is already being synthesized concurrently",
                    spec.capability_id
                )));
            }
            if self.spec_path(&spec.capability_id).exists() {
                drop(guard);
                if let Some(rec) = self.load(&spec.capability_id) {
                    return Ok(self.descriptor_from(&rec.spec, true));
                }
            } else {
                guard.insert(spec.capability_id.clone());
            }
        }
        // From here we own the in-flight slot; ensure it is always released.
        let result = self.generate_and_persist(&goal, spec.clone());
        if let Ok(mut guard) = self.in_flight.lock() {
            guard.remove(&spec.capability_id);
        }
        result
    }

    async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        // Resource-exhaustion guard (BLOCKER 9): pure primitives run in-process,
        // so bound the total input size to avoid OOM on a hostile/huge payload.
        const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
        let input_bytes: usize = req
            .args
            .as_object()
            .map(|o| o.values().filter_map(|v| v.as_str().map(|s| s.len())).sum())
            .unwrap_or(0);
        if input_bytes > MAX_INPUT_BYTES {
            return Err(CapError::Execute(format!(
                "synthesis input too large ({input_bytes} > {MAX_INPUT_BYTES} bytes)"
            )));
        }
        let rec = self.load(&req.capability_id).ok_or_else(|| {
            CapError::Execute(format!(
                "synthesized capability '{}' not found",
                req.capability_id
            ))
        })?;
        // Multi-input reducer path (W9-R9): gather the declared named args, apply
        // the audited reducer to get the initial text, then run the trailing
        // primitive pipeline (if any).
        if let Some(reducer) = &rec.spec.reducer {
            let args_map = req.args.as_object().cloned().unwrap_or_default();
            let reduced =
                crate::capability::intelligence::primitives::apply_reducer(reducer, &args_map)
                    .map_err(CapError::Execute)?
                    .ok_or_else(|| CapError::Execute(format!("unknown reducer '{reducer}'")))?;
            let result = if rec.spec.pipeline.is_empty() {
                reduced
            } else {
                crate::capability::intelligence::primitives::apply_pipeline(
                    &rec.spec.pipeline,
                    &reduced,
                )
                .map_err(CapError::Execute)?
            };
            return Ok(CapabilityOutcome::Value(
                serde_json::json!({ "result": result }),
            ));
        }

        let text = req
            .args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CapError::Execute("synthesis: missing required 'text' argument".into())
            })?;
        // Execute the authoritative Capability-Graph IR (W9-R2). A pure-primitive
        // graph runs in-process; a graph with capability nodes needs a
        // NodeExecutor (W9-R8 — routed through the platform, not the provider).
        let graph = rec.spec.normalized_graph().ok_or_else(|| {
            CapError::Execute(format!(
                "synthesized capability '{}' has no runnable IR",
                req.capability_id
            ))
        })?;
        if !graph.is_pure_primitive() {
            // Composed (capability/code node) graphs execute through the neutral
            // platform's graph executor (it owns cross-provider routing + the
            // code sandbox). Signal that with `Unsupported` so the platform
            // transparently reroutes — the provider is pure Hands.
            return Err(CapError::Unsupported(format!(
                "synthesized capability '{}' composes installed capabilities; route via the \
                 platform graph executor",
                req.capability_id
            )));
        }
        match graph.execute_pure(text) {
            Ok(result) => Ok(CapabilityOutcome::Value(
                serde_json::json!({ "result": result }),
            )),
            Err(e) => Err(CapError::Execute(e)),
        }
    }

    async fn remove(&self, capability_id: &str) -> Result<(), CapError> {
        let path = self.spec_path(capability_id);
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| CapError::Acquire(format!("synthesis remove: {e}")))?;
        }
        Ok(())
    }

    async fn health(&self) -> ProviderHealth {
        if self.store_dir.is_dir() {
            ProviderHealth::Ready
        } else {
            ProviderHealth::Offline
        }
    }
}
