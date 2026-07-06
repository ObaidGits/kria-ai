//! `CapabilityRanker` — multi-signal ranking with **config** weights (task 5.2,
//! design §8.4, R1.4 / R4.2 / R12.2).
//!
//! Discovery (task 3.3) hands the ranker a set of [`CapabilityCandidate`]s with
//! only the `semantic` and `lexical` signals populated. This module fills the
//! remaining signals it can compute — chiefly **compatibility** — and produces
//! the final deterministic ordering by combining every signal with the
//! configured [`RankWeights`].
//!
//! # Weights are data, not code (R1.4, no-hardcoding)
//!
//! The final score is a plain weighted sum
//! `w.semantic·semantic + w.lexical·lexical + w.compatibility·compatibility +
//!  w.trust·trust + w.quality·quality + w.popularity·popularity + w.success·success`.
//! The weights come entirely from [`RankWeights`] (loaded from
//! `[openclaw.cil.weights]`). There is **no per-skill and no per-category
//! branch** anywhere: every [`CapabilityTag`] id is an open string, matched
//! structurally, so a never-before-seen capability is ranked by its signals —
//! never by its name.
//!
//! # Compatibility (R12.2) — three generic sub-signals, each `0.0..=1.0`
//!
//! 1. **I/O type fit** — structural overlap of the candidate's *provided*
//!    capability tags + declared `outputs`/`inputs` against the goal's `required`
//!    capability tags (id match + qualifier-subset), weighted by each
//!    requirement's confidence.
//! 2. **Runtime fit** — the candidate's runtime capability requirements checked
//!    against a pluggable [`RuntimeAvailability`] abstraction. See the note below
//!    on why this is an abstraction rather than a live `RuntimeManager`.
//! 3. **Dependency satisfiability** — the candidate's `consumes` tags resolved
//!    against the pool of capabilities *provided* by the candidate set (what is
//!    actually available to compose with).
//!
//! `compatibility` is the mean of the three (a candidate with no profile is not
//! verifiable → `0.0`). All three are pure structural computations over open
//! strings, satisfying R12.2's "compatibility via generic I/O tags + runtime +
//! deps".
//!
//! # Why a `RuntimeAvailability` abstraction (not a live `RuntimeManager`)
//!
//! Design §8.4 phrases runtime fit as "runtime requirements vs `RuntimeManager`
//! availability". The frozen [`RuntimeManager`] is a heavyweight Docker
//! container-lifecycle authority (`crate::openclaw::runtime_manager`) and is
//! costly to construct in unit tests. Rather than fork or embed it, the ranker
//! depends on the small [`RuntimeAvailability`] trait: an open-string predicate
//! `is_available(requirement) -> bool`. A production wiring adapts the frozen
//! `RuntimeManager` behind this trait; tests use [`AllRuntimesAvailable`] or
//! [`RuntimeCapabilitySet`]. This keeps the ranker generic, testable, and honest
//! about what it reads — reusing the frozen runtime info rather than forking it.
//!
//! # Signals the ranker *reads* (tasks 8 / 15)
//!
//! `trust` and `quality` are enriched by later phases (marketplace/trust in task
//! 8). The ranker **reads** whatever the candidate already carries for these and
//! folds them into the weighted sum; it does not invent values.
//!
//! `popularity` and `success` are the **learned** signals. On every goal the
//! ranker reads them from a [`StatisticsSource`] — the seam over the frozen
//! [`SkillStatistics`](crate::openclaw::registry::SkillStatistics) that the
//! [`FeedbackLearner`](super::learn::FeedbackLearner) keeps current (task 15.1).
//! For any candidate with a `skill_ref`, the ranker looks up its stats and sets
//! `popularity` (usage-derived, saturating) and `success` (historical success
//! rate). This closes the **discover → execute → learn** loop (R4.3): a prior
//! run's outcome shifts the stats, and the next goal's ranking reflects it. When
//! a skill has no stats yet the source returns `None` and the candidate's signals
//! are left untouched (honest: absence of data is not a signal). The default
//! [`NoStatistics`] source reads nothing, preserving pre-learning-loop behavior
//! exactly.
//!
//! # Determinism (task 5.5 / R for 5.5)
//!
//! For fixed inputs + weights the ordering is reproducible: candidates are sorted
//! by descending final score with a **stable tie-break by `skill_ref`** (skill
//! id, ascending). Equal scores therefore always resolve to the same order.
//!
//! [`RuntimeManager`]: crate::openclaw::runtime_manager::RuntimeManager

use std::collections::HashSet;
use std::sync::Arc;

use super::config::RankWeights;
use super::index::CapabilityCandidate;
use super::intent::GoalIntent;
use super::profile::{CapabilityProfile, CapabilityTag};

/// Combines a candidate's signals into a final score and orders the candidates
/// in place (design §8.4).
///
/// Weights come from config ([`RankWeights`]), not code. Implementations MUST
/// NOT branch on a specific skill or capability category — all capability ids
/// are open strings, treated uniformly.
pub trait CapabilityRanker: Send + Sync {
    /// Fill any signals the ranker is responsible for (compatibility) and sort
    /// `candidates` best-first for the given `intent` and `w`eights.
    fn rank(&self, intent: &GoalIntent, candidates: &mut [CapabilityCandidate], w: &RankWeights);
}

/// A pluggable, testable view of which runtime requirements the environment can
/// satisfy — the seam standing in for the frozen `RuntimeManager` (see module
/// docs). Keyed on an **open string** requirement token so no capability is
/// hardcoded.
pub trait RuntimeAvailability: Send + Sync {
    /// Whether the runtime can satisfy the given requirement token (e.g. a
    /// capability-kind token like `"filesystem"`/`"network"`, or a runtime tag).
    fn is_available(&self, requirement: &str) -> bool;
}

/// Optimistic availability: every runtime requirement is satisfiable. Suitable
/// as a default when no runtime introspection is wired (runtime fit then never
/// penalizes a candidate).
#[derive(Debug, Clone, Copy, Default)]
pub struct AllRuntimesAvailable;

impl RuntimeAvailability for AllRuntimesAvailable {
    fn is_available(&self, _requirement: &str) -> bool {
        true
    }
}

/// Availability backed by an explicit set of satisfiable requirement tokens.
/// Anything not in the set is treated as unavailable. Useful for tests and for
/// adapting a concrete runtime's capability list.
#[derive(Debug, Clone, Default)]
pub struct RuntimeCapabilitySet {
    available: HashSet<String>,
}

impl RuntimeCapabilitySet {
    /// Build from an iterator of available requirement tokens (case-normalized).
    pub fn new(tokens: impl IntoIterator<Item = String>) -> Self {
        Self {
            available: tokens.into_iter().map(|t| t.to_lowercase()).collect(),
        }
    }
}

impl RuntimeAvailability for RuntimeCapabilitySet {
    fn is_available(&self, requirement: &str) -> bool {
        self.available.contains(&requirement.to_lowercase())
    }
}

/// The two learned signals a candidate skill carries from its execution history,
/// each normalized to `0.0..=1.0` (design §7.3 / §8.4):
///
/// - `popularity` — how much the skill has been used (install/usage counts).
/// - `success`    — its historical success rate.
///
/// Both are *derived* from the frozen
/// [`SkillStatistics`](crate::openclaw::registry::SkillStatistics), which the
/// [`FeedbackLearner`](super::learn::FeedbackLearner) keeps current on every node
/// completion (task 15.1). The ranker only ever *reads* these; it never writes
/// stats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkillSignals {
    /// Usage-derived popularity, saturating in `0.0..=1.0`.
    pub popularity: f32,
    /// Historical success rate, clamped to `0.0..=1.0`.
    pub success: f32,
}

impl SkillSignals {
    /// Derive the learned signals from a frozen
    /// [`SkillStatistics`](crate::openclaw::registry::SkillStatistics) row.
    ///
    /// - `success` is the recorded `success_rate` (already a `0.0..=1.0` rate),
    ///   clamped for safety.
    /// - `popularity` maps the unbounded `usage_count` onto `0.0..=1.0` with a
    ///   monotonic **saturating** curve `n / (n + half_saturation)`: zero uses →
    ///   `0.0`, `half_saturation` uses → `0.5`, and it approaches `1.0` as usage
    ///   grows. The curve is applied *uniformly* to every skill (no per-skill or
    ///   per-category term), so the transform introduces no hardcoding — only a
    ///   single, explicit scale parameter.
    pub fn from_statistics(
        stats: &crate::openclaw::registry::SkillStatistics,
        half_saturation: f32,
    ) -> Self {
        let n = stats.usage_count as f32;
        let k = half_saturation.max(f32::MIN_POSITIVE);
        let popularity = (n / (n + k)).clamp(0.0, 1.0);
        let success = (stats.success_rate as f32).clamp(0.0, 1.0);
        Self {
            popularity,
            success,
        }
    }
}

/// A pluggable, testable source of a skill's learned [`SkillSignals`] — the seam
/// through which the ranker READS the frozen
/// [`SkillStatistics`](crate::openclaw::registry::SkillStatistics) updated by the
/// learning loop (task 15.1). Keyed on the **open-string** `skill_id` so no skill
/// is hardcoded.
///
/// Returning `None` means "no statistics for this skill yet" — the ranker then
/// leaves the candidate's `popularity`/`success` untouched (honest: absence of
/// data is not a signal), so a skill with no history is neither rewarded nor
/// penalized beyond its other signals.
///
/// This closes the **discover → execute → learn** loop (R4.3): a run updates
/// `SkillStatistics` via the frozen `record_feedback`, and the very next goal's
/// ranking reads those updated stats through this source.
pub trait StatisticsSource: Send + Sync {
    /// The learned signals for `skill_id`, or `None` when no stats exist yet.
    fn signals(&self, skill_id: &str) -> Option<SkillSignals>;
}

/// The default statistics source: no history is available, so no candidate's
/// learned signals are altered. Using this preserves the pre-learning-loop
/// behavior exactly (flag-off / not-yet-wired parity).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStatistics;

impl StatisticsSource for NoStatistics {
    fn signals(&self, _skill_id: &str) -> Option<SkillSignals> {
        None
    }
}

/// The default `usage_count` at which [`SkillSignals`] popularity reaches `0.5`.
///
/// A single, uniform scale for the saturating popularity curve (see
/// [`SkillSignals::from_statistics`]). It is a scale parameter applied identically
/// to every skill — not a per-skill or per-category value — and can be overridden
/// via [`RegistryStatistics::with_half_saturation`].
pub const DEFAULT_POPULARITY_HALF_SATURATION: f32 = 10.0;

/// A [`StatisticsSource`] backed by the frozen
/// [`ProductionSkillRegistry`](crate::openclaw::registry::ProductionSkillRegistry).
///
/// It reads each skill's [`SkillStatistics`](crate::openclaw::registry::SkillStatistics)
/// (the single source of truth for usage/success, updated by the frozen
/// `record_feedback` path) and derives normalized [`SkillSignals`]. It performs no
/// writes and owns no statistics of its own — the registry remains authoritative.
///
/// A skill with no stats row (never executed) yields `None`, leaving the
/// candidate's provided signals untouched.
#[derive(Clone)]
pub struct RegistryStatistics {
    registry: Arc<crate::openclaw::registry::ProductionSkillRegistry>,
    half_saturation: f32,
}

impl RegistryStatistics {
    /// Read learned signals from the frozen registry using the default popularity
    /// scale ([`DEFAULT_POPULARITY_HALF_SATURATION`]).
    pub fn new(registry: Arc<crate::openclaw::registry::ProductionSkillRegistry>) -> Self {
        Self {
            registry,
            half_saturation: DEFAULT_POPULARITY_HALF_SATURATION,
        }
    }

    /// Override the popularity saturation scale (the `usage_count` at which
    /// popularity reaches `0.5`).
    pub fn with_half_saturation(mut self, half_saturation: f32) -> Self {
        self.half_saturation = half_saturation;
        self
    }
}

impl StatisticsSource for RegistryStatistics {
    fn signals(&self, skill_id: &str) -> Option<SkillSignals> {
        // A missing stats row (never executed) is honestly reported as `None`;
        // only a real error path would also yield `None` here — either way the
        // ranker simply keeps the candidate's existing signals.
        self.registry
            .get_skill_statistics(skill_id)
            .ok()
            .map(|stats| SkillSignals::from_statistics(&stats, self.half_saturation))
    }
}

/// The default multi-signal ranker.
///
/// Holds only the [`RuntimeAvailability`] seam; all tuning lives in the
/// [`RankWeights`] passed to [`rank`](CapabilityRanker::rank), so the ranker
/// carries no hardcoded weights.
pub struct DefaultCapabilityRanker<
    R: RuntimeAvailability = AllRuntimesAvailable,
    S: StatisticsSource = NoStatistics,
> {
    runtime: R,
    statistics: S,
}

impl Default for DefaultCapabilityRanker<AllRuntimesAvailable, NoStatistics> {
    fn default() -> Self {
        Self {
            runtime: AllRuntimesAvailable,
            statistics: NoStatistics,
        }
    }
}

impl DefaultCapabilityRanker<AllRuntimesAvailable, NoStatistics> {
    /// A ranker that assumes every runtime requirement is available and reads no
    /// learned statistics (learned signals stay at whatever the candidate carries).
    pub fn new() -> Self {
        Self::default()
    }
}

impl<R: RuntimeAvailability> DefaultCapabilityRanker<R, NoStatistics> {
    /// A ranker wired to a specific [`RuntimeAvailability`] backend, reading no
    /// learned statistics.
    pub fn with_runtime(runtime: R) -> Self {
        Self {
            runtime,
            statistics: NoStatistics,
        }
    }
}

impl<S: StatisticsSource> DefaultCapabilityRanker<AllRuntimesAvailable, S> {
    /// A ranker that reads learned `popularity`/`success` signals from the given
    /// [`StatisticsSource`] (e.g. [`RegistryStatistics`]) while assuming every
    /// runtime requirement is available. This is the wiring that closes the
    /// discover→execute→learn loop for the common case (task 15.2, R4.3).
    pub fn with_statistics(statistics: S) -> Self {
        Self {
            runtime: AllRuntimesAvailable,
            statistics,
        }
    }
}

impl<R: RuntimeAvailability, S: StatisticsSource> DefaultCapabilityRanker<R, S> {
    /// A ranker wired to both a [`RuntimeAvailability`] backend and a
    /// [`StatisticsSource`].
    pub fn with_runtime_and_statistics(runtime: R, statistics: S) -> Self {
        Self {
            runtime,
            statistics,
        }
    }

    /// The final weighted score for a candidate (pure function of its signals +
    /// the config weights). No per-skill/per-category term.
    fn weighted_score(c: &CapabilityCandidate, w: &RankWeights) -> f32 {
        w.semantic * c.semantic
            + w.lexical * c.lexical
            + w.compatibility * c.compatibility
            + w.trust * c.trust
            + w.quality * c.quality
            + w.popularity * c.popularity
            + w.success * c.success
    }

    /// Compatibility = mean(I/O fit, runtime fit, dependency satisfiability),
    /// each in `0.0..=1.0`. A candidate without a profile is not verifiable → 0.
    fn compatibility(
        &self,
        c: &CapabilityCandidate,
        intent: &GoalIntent,
        provided_universe: &HashSet<String>,
    ) -> f32 {
        let Some(profile) = &c.profile else {
            return 0.0;
        };
        let io = io_type_fit(profile, intent);
        let runtime = self.runtime_fit(profile);
        let deps = dependency_satisfiability(profile, provided_universe);
        ((io + runtime + deps) / 3.0).clamp(0.0, 1.0)
    }

    /// Runtime fit: fraction of the candidate's runtime capability requirements
    /// the [`RuntimeAvailability`] backend can satisfy. A skill that needs
    /// nothing at runtime is fully compatible (`1.0`). Requirement tokens are
    /// derived generically from the profile's requested permissions (open
    /// capability-kind strings) — no per-category branch.
    fn runtime_fit(&self, profile: &CapabilityProfile) -> f32 {
        if profile.permissions.is_empty() {
            return 1.0;
        }
        let satisfied = profile
            .permissions
            .iter()
            .filter(|cap| self.runtime.is_available(&runtime_token(cap)))
            .count();
        satisfied as f32 / profile.permissions.len() as f32
    }
}

impl<R: RuntimeAvailability, S: StatisticsSource> CapabilityRanker
    for DefaultCapabilityRanker<R, S>
{
    fn rank(&self, intent: &GoalIntent, candidates: &mut [CapabilityCandidate], w: &RankWeights) {
        // 1. Pool of capabilities the candidate set can actually provide — used
        //    for dependency satisfiability. Open strings: provided tag ids plus
        //    declared output type tags.
        let mut provided_universe: HashSet<String> = HashSet::new();
        for c in candidates.iter() {
            if let Some(p) = &c.profile {
                for tag in &p.provides {
                    provided_universe.insert(tag.id.clone());
                }
                for out in &p.outputs {
                    provided_universe.insert(out.clone());
                }
            }
        }

        // 2. Fill the compatibility signal generically (index left it at 0.0).
        for c in candidates.iter_mut() {
            c.compatibility = self.compatibility(c, intent, &provided_universe);
        }

        // 3. Read the learned `popularity`/`success` signals from the frozen
        //    `SkillStatistics` for any candidate identified by `skill_id`. This
        //    closes the discover→execute→learn loop (R4.3): a prior run updated
        //    the stats via `record_feedback`, and this goal's ranking reflects
        //    them. Keyed on the open-string `skill_id` — no per-skill branch.
        //    When the source has no row for a skill, the candidate's existing
        //    signals are left untouched (honest: absence of data is not data).
        for c in candidates.iter_mut() {
            if let Some(skill_id) = c.skill_ref.as_deref() {
                if let Some(signals) = self.statistics.signals(skill_id) {
                    c.popularity = signals.popularity;
                    c.success = signals.success;
                }
            }
        }

        // 4. Deterministic order: final weighted score desc, stable tie-break by
        //    skill_ref (skill id) asc.
        candidates.sort_by(|a, b| {
            let sa = Self::weighted_score(a, w);
            let sb = Self::weighted_score(b, w);
            sb.total_cmp(&sa).then_with(|| {
                a.skill_ref
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.skill_ref.as_deref().unwrap_or(""))
            })
        });
    }
}

/// I/O type fit: how well the candidate's provided capabilities + declared I/O
/// type tags cover the goal's `required` capabilities, weighted by confidence.
///
/// Pure structural tag matching over open strings:
/// - a `required` tag is covered if any of the profile's `provides` tags matches
///   it structurally ([`tag_matches`]), or if its id appears in the profile's
///   declared `outputs`/`inputs` type tags.
/// When the goal declares no required capabilities, fit is neutral-positive
/// (`0.5`) for a candidate that provides anything, else `0.0` — so ranking still
/// leans on the other signals rather than dividing by zero.
fn io_type_fit(profile: &CapabilityProfile, intent: &GoalIntent) -> f32 {
    if intent.required.is_empty() {
        return if profile.provides.is_empty() {
            0.0
        } else {
            0.5
        };
    }
    let total_conf: f32 = intent.required.iter().map(|(_, c)| c.max(0.0)).sum();
    if total_conf <= 0.0 {
        return 0.0;
    }
    // Extra matchable tokens: declared I/O type tags (open strings).
    let io_tokens: HashSet<&str> = profile
        .outputs
        .iter()
        .chain(profile.inputs.iter())
        .map(String::as_str)
        .collect();

    let mut covered = 0.0f32;
    for (req, conf) in &intent.required {
        let matched = profile.provides.iter().any(|p| tag_matches(p, req))
            || io_tokens.contains(req.id.as_str());
        if matched {
            covered += conf.max(0.0);
        }
    }
    (covered / total_conf).clamp(0.0, 1.0)
}

/// Dependency satisfiability: fraction of the candidate's `consumes` tags that
/// some capability in `provided_universe` can satisfy. A candidate that consumes
/// nothing is fully satisfiable (`1.0`).
fn dependency_satisfiability(
    profile: &CapabilityProfile,
    provided_universe: &HashSet<String>,
) -> f32 {
    if profile.consumes.is_empty() {
        return 1.0;
    }
    let satisfied = profile
        .consumes
        .iter()
        .filter(|dep| provided_universe.contains(&dep.id))
        .count();
    satisfied as f32 / profile.consumes.len() as f32
}

/// Structural tag match: same id, and every qualifier the requirement specifies
/// is present with an equal value on the provided tag (the provided tag may be
/// more specific). Embeddings are ignored here (that is the `semantic` signal).
fn tag_matches(provided: &CapabilityTag, required: &CapabilityTag) -> bool {
    if provided.id != required.id {
        return false;
    }
    required
        .qualifiers
        .iter()
        .all(|(k, v)| provided.qualifiers.get(k) == Some(v))
}

/// Derive an open-string runtime requirement token from a requested permission.
/// Uses the capability *kind* (a generic, snake-case-ish token via `Debug`) so
/// there is no per-category branch — a new capability kind is just a new token.
fn runtime_token(cap: &crate::openclaw::capability::Capability) -> String {
    format!("{:?}", cap.kind).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::capability::{
        Capability, CapabilityKind, CapabilityMode, CapabilityScope,
    };
    use crate::openclaw::cil::index::CandidateSource;
    use crate::safety::RiskLevel;
    use proptest::prelude::*;

    fn tag(id: &str) -> CapabilityTag {
        CapabilityTag::new(id)
    }

    fn profile(
        skill_id: &str,
        provides: &[&str],
        consumes: &[&str],
        inputs: &[&str],
        outputs: &[&str],
        permissions: Vec<Capability>,
    ) -> CapabilityProfile {
        CapabilityProfile {
            skill_id: skill_id.to_string(),
            provides: provides.iter().map(|s| tag(s)).collect(),
            consumes: consumes.iter().map(|s| tag(s)).collect(),
            permissions,
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            outputs: outputs.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn candidate(
        skill_id: &str,
        profile: CapabilityProfile,
        semantic: f32,
        lexical: f32,
    ) -> CapabilityCandidate {
        CapabilityCandidate {
            capability: profile
                .provides
                .first()
                .cloned()
                .unwrap_or_else(|| tag(skill_id)),
            skill_ref: Some(skill_id.to_string()),
            source: CandidateSource::Installed,
            profile: Some(profile),
            semantic,
            lexical,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        }
    }

    fn intent(required: &[(&str, f32)]) -> GoalIntent {
        GoalIntent {
            raw: "goal".into(),
            goal_embedding: vec![],
            required: required.iter().map(|(id, c)| (tag(id), *c)).collect(),
            composite: false,
            max_risk: RiskLevel::Green,
        }
    }

    fn fs_perm() -> Capability {
        Capability {
            kind: CapabilityKind::Filesystem,
            mode: CapabilityMode::ReadOnly,
            scope: CapabilityScope::Workspace,
        }
    }

    /// An in-memory [`StatisticsSource`] mapping `skill_id -> SkillSignals`, for
    /// exercising the ranker's learned-signal read path without a real registry.
    #[derive(Default)]
    struct MapStatistics {
        map: std::collections::HashMap<String, SkillSignals>,
    }

    impl MapStatistics {
        fn with(mut self, skill_id: &str, popularity: f32, success: f32) -> Self {
            self.map.insert(
                skill_id.to_string(),
                SkillSignals {
                    popularity,
                    success,
                },
            );
            self
        }
    }

    impl StatisticsSource for MapStatistics {
        fn signals(&self, skill_id: &str) -> Option<SkillSignals> {
            self.map.get(skill_id).copied()
        }
    }

    // ---- Task 15.2: ranker reads learned SkillStatistics signals ----------

    /// The ranker fills `popularity`/`success` from the [`StatisticsSource`] for
    /// candidates identified by `skill_id`, and those learned signals then drive
    /// ordering under popularity/success weights. (R4.3 — learn loop read side.)
    #[test]
    fn ranker_reads_learned_signals_from_source() {
        // Two structurally identical candidates (same provides, same semantic/
        // lexical) so ONLY the learned signals can separate them.
        let mk = || {
            vec![
                candidate(
                    "used",
                    profile("used", &["x"], &[], &[], &[], vec![]),
                    0.5,
                    0.5,
                ),
                candidate(
                    "fresh",
                    profile("fresh", &["x"], &[], &[], &[], vec![]),
                    0.5,
                    0.5,
                ),
            ]
        };

        // "used" has strong history; "fresh" has none (source returns None → its
        // signals stay at the provided 0.0).
        let stats = MapStatistics::default().with("used", 0.9, 1.0);
        let ranker = DefaultCapabilityRanker::with_statistics(stats);

        // Weight popularity + success only, so ordering reflects the learned
        // signals the ranker just read.
        let w = RankWeights {
            semantic: 0.0,
            lexical: 0.0,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 1.0,
            success: 1.0,
        };
        let intent = intent(&[]);
        let mut cands = mk();
        ranker.rank(&intent, &mut cands, &w);

        // The historically-used skill was populated from the source and ranks
        // first; the fresh skill's signals were left untouched.
        assert_eq!(cands[0].skill_ref.as_deref(), Some("used"));
        let used = cands
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("used"))
            .unwrap();
        let fresh = cands
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("fresh"))
            .unwrap();
        assert!((used.popularity - 0.9).abs() < 1e-6);
        assert!((used.success - 1.0).abs() < 1e-6);
        assert_eq!(fresh.popularity, 0.0, "no stats => signal untouched");
        assert_eq!(fresh.success, 0.0, "no stats => signal untouched");
    }

    /// The default [`NoStatistics`] source reads nothing: learned signals stay at
    /// whatever the candidate carried (pre-learning-loop parity).
    #[test]
    fn no_statistics_leaves_learned_signals_untouched() {
        let ranker = DefaultCapabilityRanker::new(); // NoStatistics by default
        let intent = intent(&[]);
        let mut cands = vec![candidate(
            "any",
            profile("any", &["x"], &[], &[], &[], vec![]),
            0.5,
            0.5,
        )];
        // Pretend a prior stage populated a value; the ranker must not overwrite it.
        cands[0].popularity = 0.42;
        cands[0].success = 0.37;
        ranker.rank(&intent, &mut cands, &RankWeights::default());
        assert_eq!(cands[0].popularity, 0.42);
        assert_eq!(cands[0].success, 0.37);
    }

    /// `SkillSignals::from_statistics` maps `success_rate` straight through
    /// (clamped) and `usage_count` onto a saturating `0.0..=1.0` popularity curve.
    #[test]
    fn skill_signals_derivation_is_normalized() {
        use crate::openclaw::registry::SkillStatistics;
        let mk_stats = |usage: u64, rate: f64| SkillStatistics {
            skill_id: "s".to_string(),
            usage_count: usage,
            last_execution: None,
            success_rate: rate,
            failure_rate: 1.0 - rate,
            average_latency_ms: 0.0,
            average_resource_usage: 0.0,
            installation_date: chrono::Utc::now(),
            publisher_trust_score: 0.0,
        };

        // Zero usage → popularity 0.0; success passes through.
        let s0 =
            SkillSignals::from_statistics(&mk_stats(0, 1.0), DEFAULT_POPULARITY_HALF_SATURATION);
        assert_eq!(s0.popularity, 0.0);
        assert!((s0.success - 1.0).abs() < 1e-6);

        // usage == half_saturation → popularity exactly 0.5.
        let k = DEFAULT_POPULARITY_HALF_SATURATION;
        let s_half = SkillSignals::from_statistics(&mk_stats(k as u64, 0.5), k);
        assert!(
            (s_half.popularity - 0.5).abs() < 1e-6,
            "got {}",
            s_half.popularity
        );

        // Popularity is monotonic and bounded < 1.0; success clamps to 0..1.
        let s_big = SkillSignals::from_statistics(&mk_stats(10_000, 2.0), k);
        assert!(s_big.popularity > s_half.popularity && s_big.popularity < 1.0);
        assert_eq!(
            s_big.success, 1.0,
            "out-of-range success_rate clamps to 1.0"
        );
    }

    /// End-to-end closure of the learn loop against the FROZEN registry: recording
    /// executions shifts `SkillStatistics`, and [`RegistryStatistics`] surfaces
    /// those updated signals to the ranker on the next goal. (R4.3)
    #[test]
    fn registry_statistics_closes_the_learn_loop() {
        use crate::openclaw::registry::{
            DiscoverySource, ProductionSkillRegistry, SkillMetadata, SkillState,
        };
        use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
        use chrono::Utc;

        fn sample(skill_id: &str) -> SkillMetadata {
            SkillMetadata {
                skill_id: skill_id.to_string(),
                name: skill_id.to_string(),
                description: "test".to_string(),
                publisher: "test".to_string(),
                version: "1.0.0".to_string(),
                category: "test".to_string(),
                discovery_source: DiscoverySource::Bundled {
                    path: "t".to_string(),
                },
                discovered_at: Utc::now(),
                capabilities: SkillCapabilities::default(),
                runtime_requirements: "docker".to_string(),
                risk_level: RiskLevel::Green,
                resource_class: ResourceClass::Light,
                tags: vec![],
                categories: vec![],
                semantic_version: "1.0.0".to_string(),
                dependencies: vec![],
                compatibility_requirements: vec![],
                trust_tier: TrustTier::Community,
                content_hash: format!("hash_{skill_id}"),
                signature: None,
                granted_capabilities: Vec::new(),
                bundle_path: None,
                manifest_toml: None,
                input_schema: None,
                state: SkillState::Enabled,
                state_changed_at: Utc::now(),
            }
        }

        let dir = tempfile::TempDir::new().expect("temp dir");
        let registry = Arc::new(
            ProductionSkillRegistry::new(&dir.path().join("rank_stats.db")).expect("registry"),
        );
        registry
            .install_skill(&sample("oc_used"))
            .expect("install used");
        registry
            .install_skill(&sample("oc_fresh"))
            .expect("install fresh");

        // Simulate a learn-loop run: "oc_used" executed successfully several times.
        for _ in 0..DEFAULT_POPULARITY_HALF_SATURATION as u64 {
            registry
                .record_execution("oc_used", true, 100, 0.1)
                .expect("record");
        }

        let source = RegistryStatistics::new(Arc::clone(&registry));
        let ranker = DefaultCapabilityRanker::with_statistics(source);

        let w = RankWeights {
            semantic: 0.0,
            lexical: 0.0,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 1.0,
            success: 1.0,
        };
        let intent = intent(&[]);
        let mut cands = vec![
            candidate(
                "oc_used",
                profile("oc_used", &["x"], &[], &[], &[], vec![]),
                0.5,
                0.5,
            ),
            candidate(
                "oc_fresh",
                profile("oc_fresh", &["x"], &[], &[], &[], vec![]),
                0.5,
                0.5,
            ),
        ];
        ranker.rank(&intent, &mut cands, &w);

        // The executed skill's learned signals came straight from the frozen
        // registry stats and lifted it above the never-run skill.
        assert_eq!(cands[0].skill_ref.as_deref(), Some("oc_used"));
        let used = cands
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("oc_used"))
            .unwrap();
        assert!((used.success - 1.0).abs() < 1e-6, "success from stats");
        assert!(
            (used.popularity - 0.5).abs() < 1e-6,
            "usage==k => popularity 0.5, got {}",
            used.popularity
        );
    }

    /// Ordering is deterministic and stable for fixed inputs + weights.
    #[test]
    fn ordering_is_deterministic() {
        let w = RankWeights::default();
        let ranker = DefaultCapabilityRanker::new();
        let intent = intent(&[("doc.pdf.compress", 1.0)]);

        let mk = || {
            vec![
                candidate(
                    "a.pdf",
                    profile("a.pdf", &["doc.pdf.compress"], &[], &[], &[], vec![]),
                    0.9,
                    0.5,
                ),
                candidate(
                    "b.ocr",
                    profile("b.ocr", &["media.image.ocr"], &[], &[], &[], vec![]),
                    0.4,
                    0.3,
                ),
                candidate(
                    "c.email",
                    profile("c.email", &["net.email.send"], &[], &[], &[], vec![]),
                    0.2,
                    0.1,
                ),
            ]
        };
        let mut first = mk();
        let mut second = mk();
        ranker.rank(&intent, &mut first, &w);
        ranker.rank(&intent, &mut second, &w);
        let ids1: Vec<_> = first.iter().map(|c| c.skill_ref.clone()).collect();
        let ids2: Vec<_> = second.iter().map(|c| c.skill_ref.clone()).collect();
        assert_eq!(ids1, ids2);
        // The pdf skill (best semantic + full I/O fit) ranks first.
        assert_eq!(first[0].skill_ref.as_deref(), Some("a.pdf"));
    }

    /// Equal-score candidates break ties by skill id ascending (determinism).
    #[test]
    fn equal_scores_break_by_skill_id() {
        let w = RankWeights::default();
        let ranker = DefaultCapabilityRanker::new();
        // No required caps + identical signals → identical scores.
        let intent = intent(&[]);
        let p = |id: &str| profile(id, &["x"], &[], &[], &[], vec![]);
        let mut cands = vec![
            candidate("zzz", p("zzz"), 0.5, 0.5),
            candidate("aaa", p("aaa"), 0.5, 0.5),
            candidate("mmm", p("mmm"), 0.5, 0.5),
        ];
        ranker.rank(&intent, &mut cands, &w);
        let ids: Vec<_> = cands.iter().map(|c| c.skill_ref.clone().unwrap()).collect();
        assert_eq!(ids, vec!["aaa", "mmm", "zzz"]);
    }

    /// Changing a weight changes the ordering (weights are the tuning surface).
    #[test]
    fn changing_a_weight_changes_ordering() {
        let ranker = DefaultCapabilityRanker::new();
        let intent = intent(&[]);
        // Candidate L wins on lexical; candidate S wins on semantic.
        let mk = || {
            vec![
                candidate(
                    "s.skill",
                    profile("s.skill", &["x"], &[], &[], &[], vec![]),
                    0.9,
                    0.1,
                ),
                candidate(
                    "l.skill",
                    profile("l.skill", &["x"], &[], &[], &[], vec![]),
                    0.1,
                    0.9,
                ),
            ]
        };

        let mut semantic_heavy = mk();
        let w_sem = RankWeights {
            semantic: 1.0,
            lexical: 0.0,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        };
        ranker.rank(&intent, &mut semantic_heavy, &w_sem);
        assert_eq!(semantic_heavy[0].skill_ref.as_deref(), Some("s.skill"));

        let mut lexical_heavy = mk();
        let w_lex = RankWeights {
            semantic: 0.0,
            lexical: 1.0,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        };
        ranker.rank(&intent, &mut lexical_heavy, &w_lex);
        assert_eq!(lexical_heavy[0].skill_ref.as_deref(), Some("l.skill"));
    }

    /// A novel, never-before-seen capability id is ranked via its signals
    /// (compatibility from I/O tag match), NOT by its name — no per-category
    /// branch exists. (Property 2 smoke; full property test is task 5.4.)
    #[test]
    fn novel_capability_ranked_by_signals_not_name() {
        // Weight compatibility only, so ordering reflects I/O fit alone.
        let w = RankWeights {
            semantic: 0.0,
            lexical: 0.0,
            compatibility: 1.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        };
        let ranker = DefaultCapabilityRanker::new();
        let intent = intent(&[("quantum.entangle.route.v9", 1.0)]);

        // match provides the exact novel tag; miss provides an unrelated tag.
        let mut cands = vec![
            candidate(
                "miss",
                profile("miss", &["unrelated.cap"], &[], &[], &[], vec![]),
                0.0,
                0.0,
            ),
            candidate(
                "match",
                profile(
                    "match",
                    &["quantum.entangle.route.v9"],
                    &[],
                    &[],
                    &[],
                    vec![],
                ),
                0.0,
                0.0,
            ),
        ];
        ranker.rank(&intent, &mut cands, &w);
        assert_eq!(cands[0].skill_ref.as_deref(), Some("match"));
        assert!(cands[0].compatibility > cands[1].compatibility);
    }

    /// Compatibility folds in runtime availability + dependency satisfiability.
    #[test]
    fn compatibility_uses_runtime_and_dependencies() {
        let intent = intent(&[("doc.pdf.compress", 1.0)]);

        // Candidate needs a filesystem permission and consumes io.file.read.
        let p = profile(
            "needs",
            &["doc.pdf.compress"],
            &["io.file.read"],
            &[],
            &[],
            vec![fs_perm()],
        );

        // Runtime that can satisfy "filesystem"; io.file.read provided by a peer.
        let ranker = DefaultCapabilityRanker::with_runtime(RuntimeCapabilitySet::new([
            "filesystem".to_string(),
        ]));
        let peer = profile("peer", &["io.file.read"], &[], &[], &[], vec![]);

        let mut cands = vec![
            candidate("needs", p, 0.0, 0.0),
            candidate("peer", peer, 0.0, 0.0),
        ];
        let w = RankWeights {
            semantic: 0.0,
            lexical: 0.0,
            compatibility: 1.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        };
        ranker.rank(&intent, &mut cands, &w);

        let needs = cands
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("needs"))
            .unwrap();
        // io fit = 1.0 (provides required), runtime = 1.0 (fs available),
        // deps = 1.0 (io.file.read provided by peer) → compatibility = 1.0.
        assert!(
            (needs.compatibility - 1.0).abs() < 1e-6,
            "got {}",
            needs.compatibility
        );

        // With no runtime available, the same candidate's runtime fit drops,
        // lowering compatibility below 1.0 (honest, generic).
        let strict = DefaultCapabilityRanker::with_runtime(RuntimeCapabilitySet::default());
        let p2 = profile(
            "needs",
            &["doc.pdf.compress"],
            &["io.file.read"],
            &[],
            &[],
            vec![fs_perm()],
        );
        let peer2 = profile("peer", &["io.file.read"], &[], &[], &[], vec![]);
        let mut cands2 = vec![
            candidate("needs", p2, 0.0, 0.0),
            candidate("peer", peer2, 0.0, 0.0),
        ];
        strict.rank(&intent, &mut cands2, &w);
        let needs2 = cands2
            .iter()
            .find(|c| c.skill_ref.as_deref() == Some("needs"))
            .unwrap();
        assert!(needs2.compatibility < 1.0, "got {}", needs2.compatibility);
    }

    /// Determinism must depend only on scores + the stable skill_id tie-break —
    /// NOT on input order. For a fixed candidate set + fixed weights, every input
    /// permutation yields the identical ranked ordering. (R4.2, task 5.5)
    #[test]
    fn ordering_is_independent_of_input_order() {
        let w = RankWeights::default();
        let ranker = DefaultCapabilityRanker::new();
        let intent = intent(&[("doc.pdf.compress", 1.0)]);

        // Three candidates with distinct signals so scores are well-separated.
        let build = |order: &[usize]| {
            let base = [
                candidate(
                    "a.pdf",
                    profile("a.pdf", &["doc.pdf.compress"], &[], &[], &[], vec![]),
                    0.9,
                    0.5,
                ),
                candidate(
                    "b.ocr",
                    profile("b.ocr", &["media.image.ocr"], &[], &[], &[], vec![]),
                    0.4,
                    0.3,
                ),
                candidate(
                    "c.email",
                    profile("c.email", &["net.email.send"], &[], &[], &[], vec![]),
                    0.2,
                    0.1,
                ),
            ];
            order.iter().map(|&i| base[i].clone()).collect::<Vec<_>>()
        };

        // The canonical ranked ordering (from natural input order).
        let mut canonical = build(&[0, 1, 2]);
        ranker.rank(&intent, &mut canonical, &w);
        let expected: Vec<_> = canonical.iter().map(|c| c.skill_ref.clone()).collect();

        // Every permutation of the same set must rank to the same order.
        let permutations = [
            [0usize, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        for perm in permutations {
            let mut cands = build(&perm);
            ranker.rank(&intent, &mut cands, &w);
            let got: Vec<_> = cands.iter().map(|c| c.skill_ref.clone()).collect();
            assert_eq!(
                got, expected,
                "input permutation {perm:?} changed ranked order"
            );
        }
    }

    /// The FULL ordering (not just the winner) is stable and matches the
    /// descending weighted-score order. (R4.2, task 5.5)
    #[test]
    fn full_ordering_matches_weighted_score_order() {
        // Semantic-only weights → score == semantic, so the expected order is
        // simply descending semantic value.
        let w = RankWeights {
            semantic: 1.0,
            lexical: 0.0,
            compatibility: 0.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        };
        let ranker = DefaultCapabilityRanker::new();
        let intent = intent(&[]);
        let p = |id: &str| profile(id, &["x"], &[], &[], &[], vec![]);

        // Deliberately unsorted input; distinct semantic scores.
        let mut cands = vec![
            candidate("mid", p("mid"), 0.5, 0.0),
            candidate("top", p("top"), 0.9, 0.0),
            candidate("low", p("low"), 0.1, 0.0),
            candidate("high", p("high"), 0.7, 0.0),
        ];
        ranker.rank(&intent, &mut cands, &w);

        let ids: Vec<_> = cands.iter().map(|c| c.skill_ref.clone().unwrap()).collect();
        assert_eq!(ids, vec!["top", "high", "mid", "low"]);

        // Scores are monotonically non-increasing across the full ordering.
        let scores: Vec<f32> = cands
            .iter()
            .map(|c| {
                DefaultCapabilityRanker::<AllRuntimesAvailable, NoStatistics>::weighted_score(c, &w)
            })
            .collect();
        for pair in scores.windows(2) {
            assert!(
                pair[0] >= pair[1],
                "full ordering not score-descending: {scores:?}"
            );
        }
    }

    /// Two candidates differing ONLY by skill_id (identical signals → identical
    /// scores) always order by skill_id ascending, for every input order —
    /// the stable tie-break, not input position, decides. (R4.2, task 5.5)
    #[test]
    fn identical_signals_tie_break_by_skill_id_regardless_of_input_order() {
        let w = RankWeights::default();
        let ranker = DefaultCapabilityRanker::new();
        let intent = intent(&[("doc.pdf.compress", 1.0)]);

        // Same provides + same semantic/lexical → identical weighted score.
        let mk = |id: &str| {
            candidate(
                id,
                profile(id, &["doc.pdf.compress"], &[], &[], &[], vec![]),
                0.6,
                0.4,
            )
        };

        // Feed both possible input orderings; result must be skill_id ascending.
        let mut forward = vec![mk("alpha"), mk("beta")];
        let mut reversed = vec![mk("beta"), mk("alpha")];
        ranker.rank(&intent, &mut forward, &w);
        ranker.rank(&intent, &mut reversed, &w);

        let f: Vec<_> = forward
            .iter()
            .map(|c| c.skill_ref.clone().unwrap())
            .collect();
        let r: Vec<_> = reversed
            .iter()
            .map(|c| c.skill_ref.clone().unwrap())
            .collect();
        assert_eq!(f, vec!["alpha", "beta"]);
        assert_eq!(
            r,
            vec!["alpha", "beta"],
            "tie-break must not depend on input order"
        );
    }

    // ---- Property 2: compatibility-ranking generality (task 5.4) ----------
    //
    // The ranker MUST treat a synthetic, never-before-seen `CapabilityTag`
    // identically to any other capability: compatibility is computed from I/O
    // tag matching + runtime + deps, NEVER from the tag's *name*. If any novel
    // tag were ranked differently purely because of its string, that would be a
    // real no-hardcoding violation (R1.1) and this test would surface the
    // counterexample.

    /// A baseline "unrelated" capability id that the novel-tag generator can
    /// never produce (it contains digits; generated segments are `[a-z]` only),
    /// guaranteeing the non-provider candidate never accidentally matches the
    /// generated required tag.
    const UNRELATED_BASELINE: &str = "baseline9.unrelated9.cap";

    /// Generate a random, synthetic, reverse-DNS-ish `CapabilityTag` id built
    /// only from lowercase-letter segments — guaranteed to be a freely-generated
    /// string, never a hardcoded/common capability id.
    fn novel_tag_id() -> impl Strategy<Value = String> {
        proptest::collection::vec("[a-z]{3,8}", 2..=4).prop_map(|segs| segs.join("."))
    }

    /// Compatibility-only weights so ordering reflects the compatibility signal
    /// alone (isolates the structural I/O-fit computation under test).
    fn compat_only_weights() -> RankWeights {
        RankWeights {
            semantic: 0.0,
            lexical: 0.0,
            compatibility: 1.0,
            trust: 0.0,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// **Property 2 / R1.1, R12.1 — novel-capability generality.**
        ///
        /// For an ARBITRARY generated novel `CapabilityTag`:
        ///   (a) a candidate whose `provides` contains the required novel tag has
        ///       STRICTLY higher compatibility than one that does not, and ranks
        ///       at/above it — regardless of the specific string; and
        ///   (b) compatibility is INVARIANT under the tag string: renaming the
        ///       tag (novel A vs novel B) while keeping the same structural
        ///       relationship (provider provides the required tag) yields the
        ///       SAME compatibility score. Compatibility depends on structural
        ///       match, not on the name.
        #[test]
        fn novel_capability_generality(
            tag_a in novel_tag_id(),
            tag_b in novel_tag_id(),
        ) {
            // A and B must be genuinely different strings to make the rename
            // invariant meaningful (equal strings would pass trivially).
            prop_assume!(tag_a != tag_b);
            // Neither novel tag may collide with the unrelated baseline id.
            prop_assume!(tag_a != UNRELATED_BASELINE && tag_b != UNRELATED_BASELINE);

            let w = compat_only_weights();
            let ranker = DefaultCapabilityRanker::new();

            // ---- (a) provider outranks non-provider, for tag_a -------------
            let intent_a = intent(&[(tag_a.as_str(), 1.0)]);
            let mut cands_a = vec![
                candidate(
                    "non_provider",
                    profile("non_provider", &[UNRELATED_BASELINE], &[], &[], &[], vec![]),
                    0.0,
                    0.0,
                ),
                candidate(
                    "provider",
                    profile("provider", &[tag_a.as_str()], &[], &[], &[], vec![]),
                    0.0,
                    0.0,
                ),
            ];
            ranker.rank(&intent_a, &mut cands_a, &w);

            let provider_a = cands_a
                .iter()
                .find(|c| c.skill_ref.as_deref() == Some("provider"))
                .unwrap();
            let non_provider_a = cands_a
                .iter()
                .find(|c| c.skill_ref.as_deref() == Some("non_provider"))
                .unwrap();

            prop_assert!(
                provider_a.compatibility > non_provider_a.compatibility,
                "novel tag {tag_a:?}: provider compat {} not > non-provider {}",
                provider_a.compatibility,
                non_provider_a.compatibility
            );
            // Higher compatibility ⇒ ranks at/above the non-provider.
            prop_assert_eq!(
                cands_a[0].skill_ref.as_deref(),
                Some("provider"),
                "novel tag {:?}: provider did not rank first",
                tag_a
            );

            // ---- (b) rename invariance: tag_a scenario vs tag_b scenario ----
            let intent_b = intent(&[(tag_b.as_str(), 1.0)]);
            let mut cands_b = vec![
                candidate(
                    "provider",
                    profile("provider", &[tag_b.as_str()], &[], &[], &[], vec![]),
                    0.0,
                    0.0,
                ),
            ];
            ranker.rank(&intent_b, &mut cands_b, &w);
            let provider_b = &cands_b[0];

            // Same structural relationship under a different name ⇒ same score.
            // Bit-for-bit equality: the computation is a pure structural fn that
            // never reads the tag string beyond equality matching.
            prop_assert_eq!(
                provider_a.compatibility,
                provider_b.compatibility,
                "compatibility changed under rename {:?} -> {:?}: {} vs {}",
                tag_a,
                tag_b,
                provider_a.compatibility,
                provider_b.compatibility
            );
        }
    }
}
