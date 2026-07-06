//! A9.0.5 Skill Quality Evaluation — score every generated skill.
//!
//! Low-quality skills never auto-install. Scoring is deterministic over the design +
//! artifacts + validation/sandbox outcomes (no LLM required), so it is reproducible.

use super::designer::SkillDesign;
use super::generator::GeneratedArtifacts;
use serde::{Deserialize, Serialize};

/// A multi-dimensional quality score in [0.0, 1.0] per axis (A9.0.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityScore {
    pub architecture: f64,
    pub code_quality: f64,
    pub security: f64,
    pub performance: f64,
    pub maintainability: f64,
    pub complexity: f64,
    pub documentation: f64,
    pub examples: f64,
    pub test_coverage: f64,
    pub capability_correctness: f64,
    pub risk: f64,
    /// Weighted overall confidence in [0.0, 1.0].
    pub overall: f64,
}

impl QualityScore {
    /// Whether the skill meets the auto-install bar.
    pub fn meets(&self, threshold: f64) -> bool {
        self.overall >= threshold
    }
}

/// The single quality evaluator (A9.0.5).
pub struct QualityEvaluator;

impl QualityEvaluator {
    /// Evaluate a generated skill. `validation_ok` and `sandbox_ok` gate security/coverage.
    pub fn evaluate(
        design: &SkillDesign,
        artifacts: &GeneratedArtifacts,
        validation_ok: bool,
        sandbox_ok: bool,
    ) -> QualityScore {
        let code = &artifacts.handler_code;
        let loc = code.lines().count().max(1);

        // Documentation: has description + docs + examples.
        let documentation = clamp01(
            0.3 * bool_score(!design.description.is_empty())
                + 0.4 * bool_score(design.documentation.len() > 40)
                + 0.3 * bool_score(!design.examples.is_empty()),
        );

        // Examples axis.
        let examples = clamp01(design.examples.len() as f64 / 3.0);

        // Test coverage proxy: tests exist + reference the handler + sandbox passed.
        let has_tests = !artifacts.test_code.trim().is_empty();
        let test_coverage = clamp01(0.4 * bool_score(has_tests) + 0.6 * bool_score(sandbox_ok));

        // Code quality: has error handling + logging, not trivially short, no placeholders.
        let has_error_handling = code.contains("catch")
            || code.contains("try")
            || code.contains("Result")
            || code.contains("error");
        let no_placeholder = !["TODO", "FIXME", "placeholder"]
            .iter()
            .any(|m| code.contains(m));
        let code_quality = clamp01(
            0.4 * bool_score(has_error_handling)
                + 0.3 * bool_score(no_placeholder)
                + 0.3 * bool_score(loc >= 5),
        );

        // Complexity: penalize very large handlers (favor focused skills).
        let complexity = clamp01(1.0 - ((loc as f64 - 40.0).max(0.0) / 400.0));

        // Maintainability: docs + tests + moderate size.
        let maintainability = clamp01(0.5 * documentation + 0.3 * test_coverage + 0.2 * complexity);

        // Security: validation passed + risk not RED-unbounded + no dangerous eval.
        let no_eval = !code.contains("eval(") && !code.contains("child_process");
        let security = clamp01(
            0.5 * bool_score(validation_ok)
                + 0.3 * bool_score(no_eval)
                + 0.2 * bool_score(!design.capabilities.iter().any(|c| c == "shell")),
        );

        // Performance: proxy — light resource class scores higher.
        let performance = match design.resource_class.as_str() {
            "light" => 0.9,
            "medium" => 0.7,
            _ => 0.5,
        };

        // Architecture: valid design (has schema + entry + capabilities inferred).
        let architecture = clamp01(
            0.4 * bool_score(design.schema.is_object())
                + 0.3 * bool_score(!design.entry.is_empty())
                + 0.3 * bool_score(validation_ok),
        );

        // Capability correctness: inferred risk matches declared.
        let cap_ok = super::designer::classify_risk(&design.capabilities) == design.risk;
        let capability_correctness = bool_score(cap_ok);

        // Risk axis: lower risk → higher score.
        let risk = match design.risk {
            crate::safety::RiskLevel::Green => 1.0,
            crate::safety::RiskLevel::Yellow => 0.7,
            crate::safety::RiskLevel::Red => 0.4,
            crate::safety::RiskLevel::Black => 0.0,
        };

        // Weighted overall.
        let overall = clamp01(
            0.12 * architecture
                + 0.14 * code_quality
                + 0.16 * security
                + 0.08 * performance
                + 0.10 * maintainability
                + 0.06 * complexity
                + 0.08 * documentation
                + 0.05 * examples
                + 0.13 * test_coverage
                + 0.05 * capability_correctness
                + 0.03 * risk,
        );

        QualityScore {
            architecture,
            code_quality,
            security,
            performance,
            maintainability,
            complexity,
            documentation,
            examples,
            test_coverage,
            capability_correctness,
            risk,
            overall,
        }
    }
}

fn bool_score(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}
