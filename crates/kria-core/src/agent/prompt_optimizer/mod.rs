//! Prompt Optimizer — Epsilon-Greedy prompt variant tracking.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────┐
//! │  Task Domain      │  ← "system_admin", "file_ops", "communication"
//! └────────┬─────────┘
//!          │
//!          ▼
//! ┌──────────────────┐
//! │  Epsilon-Greedy   │  ← 90% exploit (best variant), 10% explore (new variant)
//! │  Selector         │
//! └────────┬─────────┘
//!          │
//!          ▼
//! ┌──────────────────┐
//! │  Prompt Variant   │  ← selected template for this domain
//! └────────┬─────────┘
//!          │
//!          ▼
//! ┌──────────────────┐
//! │  Task Execution   │
//! └────────┬─────────┘
//!          │
//!          ▼
//! ┌──────────────────┐
//! │  Outcome Recording│  ← success/failure → update variant stats in SQLite
//! └──────────────────┘
//! ```
//!
//! # Epsilon-Greedy Policy
//!
//! Most tasks (90%) use the highest-scoring prompt variant for that domain.
//! A small percentage (10%) tests new or lower-scoring variants to discover
//! improvements. This prevents the system from getting stuck on a "lucky"
//! variant while also preventing wild experimentation.
//!
//! The variant success rates are stored in SQLite and persist across restarts.
//! Over time, the system converges on the best prompt for each domain.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

// ─── Types ──────────────────────────────────────────────────────────────────

/// A task domain for prompt optimization.
///
/// Different domains may benefit from different prompt strategies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TaskDomain {
    SystemAdmin,
    FileOps,
    Communication,
    WebSearch,
    CodeGeneration,
    Planning,
    Diagnosis,
    General,
}

impl TaskDomain {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SystemAdmin => "system_admin",
            Self::FileOps => "file_ops",
            Self::Communication => "communication",
            Self::WebSearch => "web_search",
            Self::CodeGeneration => "code_generation",
            Self::Planning => "planning",
            Self::Diagnosis => "diagnosis",
            Self::General => "general",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "system_admin" => Self::SystemAdmin,
            "file_ops" => Self::FileOps,
            "communication" => Self::Communication,
            "web_search" => Self::WebSearch,
            "code_generation" => Self::CodeGeneration,
            "planning" => Self::Planning,
            "diagnosis" => Self::Diagnosis,
            _ => Self::General,
        }
    }
}

/// A prompt variant with its performance statistics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PromptVariant {
    /// Unique identifier for this variant.
    pub id: String,
    /// The domain this variant is optimized for.
    pub domain: TaskDomain,
    /// The prompt template (may contain {goal}, {context}, {tools} placeholders).
    pub template: String,
    /// Number of successful uses.
    pub successes: u64,
    /// Number of failed uses.
    pub failures: u64,
    /// When this variant was created.
    pub created_at: DateTime<Utc>,
    /// When this variant was last used.
    pub last_used: DateTime<Utc>,
}

impl PromptVariant {
    /// Total number of uses.
    pub fn total_uses(&self) -> u64 {
        self.successes + self.failures
    }

    /// Success rate (0.0 to 1.0).
    /// Returns 0.5 (neutral) if no data.
    pub fn success_rate(&self) -> f64 {
        let total = self.total_uses();
        if total == 0 {
            return 0.5; // Neutral prior (like Beta(1,1))
        }
        self.successes as f64 / total as f64
    }

    /// Upper Confidence Bound (UCB1) score for exploration.
    ///
    /// UCB1 = mean_reward + sqrt(2 * ln(N) / n_i)
    /// where N = total uses across all variants, n_i = uses for this variant.
    pub fn ucb1_score(&self, total_uses_all_variants: u64) -> f64 {
        let n = self.total_uses() as f64;
        if n == 0.0 {
            return f64::MAX; // Never tried → highest priority for exploration.
        }
        let n_all = total_uses_all_variants as f64;
        let exploration_bonus = (2.0 * n_all.ln() / n).sqrt();
        self.success_rate() + exploration_bonus
    }
}

/// The outcome of a task execution using a prompt variant.
#[derive(Debug, Clone)]
pub struct TaskOutcome {
    /// The domain of the task.
    pub domain: TaskDomain,
    /// The variant ID used.
    pub variant_id: String,
    /// Whether the task succeeded.
    pub success: bool,
    /// Optional notes about why it succeeded/failed.
    pub notes: Option<String>,
}

// ─── Epsilon-Greedy Prompt Optimizer ────────────────────────────────────────

/// Configuration for the prompt optimizer.
#[derive(Debug, Clone)]
pub struct PromptOptimizerConfig {
    /// Probability of exploring a new/random variant (0.0 to 1.0).
    /// Default: 0.1 (10% explore, 90% exploit).
    pub epsilon: f64,
    /// Minimum number of uses before a variant can be considered "best".
    /// Prevents premature convergence on a lucky variant.
    pub min_uses_for_convergence: u64,
    /// Maximum number of variants per domain.
    pub max_variants_per_domain: usize,
}

impl Default for PromptOptimizerConfig {
    fn default() -> Self {
        Self {
            epsilon: 0.1,
            min_uses_for_convergence: 10,
            max_variants_per_domain: 10,
        }
    }
}

/// Epsilon-Greedy Prompt Optimizer.
///
/// Tracks prompt variant performance per domain and selects the best
/// variant for each task. Uses epsilon-greedy exploration to discover
/// improvements while mostly exploiting the best-known variant.
pub struct PromptOptimizer {
    /// Configuration.
    config: PromptOptimizerConfig,
    /// Prompt variants indexed by (domain, variant_id).
    variants: Mutex<HashMap<(TaskDomain, String), PromptVariant>>,
    /// Total uses per domain (for UCB1 calculation).
    domain_totals: Mutex<HashMap<TaskDomain, u64>>,
    /// SQLite connection for persistence.
    conn: Mutex<rusqlite::Connection>,
}

impl PromptOptimizer {
    /// Create a new prompt optimizer with an in-memory SQLite database.
    pub fn new(config: PromptOptimizerConfig) -> Self {
        let conn = rusqlite::Connection::open_in_memory()
            .expect("Failed to create in-memory SQLite for PromptOptimizer");
        Self::init_db(&conn);
        Self {
            config,
            variants: Mutex::new(HashMap::new()),
            domain_totals: Mutex::new(HashMap::new()),
            conn: Mutex::new(conn),
        }
    }

    /// Create a new prompt optimizer with a file-backed SQLite database.
    pub fn with_db_path(config: PromptOptimizerConfig, db_path: &str) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(db_path)
            .map_err(|e| format!("Failed to open SQLite database: {}", e))?;
        Self::init_db(&conn);
        let optimizer = Self {
            config,
            variants: Mutex::new(HashMap::new()),
            domain_totals: Mutex::new(HashMap::new()),
            conn: Mutex::new(conn),
        };
        optimizer.load_from_db()?;
        Ok(optimizer)
    }

    /// Initialize the SQLite schema.
    fn init_db(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS prompt_variants (
                id TEXT NOT NULL,
                domain TEXT NOT NULL,
                template TEXT NOT NULL,
                successes INTEGER NOT NULL DEFAULT 0,
                failures INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                last_used TEXT NOT NULL,
                PRIMARY KEY (id, domain)
            );
            CREATE INDEX IF NOT EXISTS idx_prompt_variants_domain
                ON prompt_variants(domain);",
        )
        .expect("Failed to initialize PromptOptimizer schema");
    }

    /// Load variants from the database.
    fn load_from_db(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, domain, template, successes, failures, created_at, last_used FROM prompt_variants")
            .map_err(|e| format!("Failed to prepare query: {}", e))?;

        let variants: Vec<PromptVariant> = stmt
            .query_map([], |row| {
                Ok(PromptVariant {
                    id: row.get(0)?,
                    domain: TaskDomain::from_str(&row.get::<_, String>(1)?),
                    template: row.get(2)?,
                    successes: row.get::<_, i64>(3)? as u64,
                    failures: row.get::<_, i64>(4)? as u64,
                    created_at: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<_, String>(5)?,
                    )
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
                    last_used: chrono::DateTime::parse_from_rfc3339(
                        &row.get::<_, String>(6)?,
                    )
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
                })
            })
            .map_err(|e| format!("Failed to query variants: {}", e))?
            .filter_map(|r| r.ok())
            .collect();

        let mut variants_map = self.variants.lock().unwrap();
        let mut domain_totals = self.domain_totals.lock().unwrap();

        for variant in variants {
            let domain = variant.domain.clone();
            let total = variant.total_uses();
            variants_map.insert((domain.clone(), variant.id.clone()), variant);
            *domain_totals.entry(domain).or_insert(0) += total;
        }

        Ok(())
    }

    /// Register a new prompt variant for a domain.
    pub fn register_variant(
        &self,
        domain: TaskDomain,
        variant_id: &str,
        template: &str,
    ) -> Result<(), String> {
        let key = (domain.clone(), variant_id.to_string());

        {
            let variants = self.variants.lock().unwrap();
            if variants.contains_key(&key) {
                return Err(format!("Variant '{}' already exists for {:?}", variant_id, domain));
            }

            // Check max variants per domain.
            let domain_count = variants.keys().filter(|(d, _)| *d == domain).count();
            if domain_count >= self.config.max_variants_per_domain {
                return Err(format!(
                    "Maximum {} variants per domain reached for {:?}",
                    self.config.max_variants_per_domain, domain
                ));
            }
        }

        let variant = PromptVariant {
            id: variant_id.to_string(),
            domain: domain.clone(),
            template: template.to_string(),
            successes: 0,
            failures: 0,
            created_at: Utc::now(),
            last_used: Utc::now(),
        };

        // Persist to SQLite.
        self.persist_variant(&variant)?;

        // Insert into memory.
        let mut variants = self.variants.lock().unwrap();
        variants.insert(key, variant);

        Ok(())
    }

    /// Select the best prompt variant for a domain using epsilon-greedy.
    ///
    /// - With probability (1 - epsilon): select the variant with the highest success rate.
    /// - With probability epsilon: select a random variant (exploration).
    pub fn select_variant(&self, domain: &TaskDomain) -> Option<PromptVariant> {
        let variants = self.variants.lock().unwrap();
        let domain_variants: Vec<&PromptVariant> = variants
            .iter()
            .filter(|((d, _), _)| d == domain)
            .map(|(_, v)| v)
            .collect();

        if domain_variants.is_empty() {
            return None;
        }

        // Epsilon-greedy selection.
        let random_value: f64 = rand::random();

        if random_value < self.config.epsilon && domain_variants.len() > 1 {
            // Explore: pick a random variant (but prefer less-tried ones).
            let total_uses = self.domain_totals.lock().unwrap()
                .get(domain)
                .copied()
                .unwrap_or(0);

            // Use UCB1 for exploration — picks the variant with highest
            // upper confidence bound, which balances exploitation and exploration.
            let best_exploration = domain_variants
                .iter()
                .max_by(|a, b| {
                    a.ucb1_score(total_uses)
                        .partial_cmp(&b.ucb1_score(total_uses))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

            best_exploration.map(|v| (*v).clone())
        } else {
            // Exploit: pick the variant with the highest success rate.
            // But only if it has enough uses to be confident.
            let best_exploitation = domain_variants
                .iter()
                .filter(|v| v.total_uses() >= self.config.min_uses_for_convergence)
                .max_by(|a, b| {
                    a.success_rate()
                        .partial_cmp(&b.success_rate())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .or_else(|| {
                    // If no variant has enough uses, pick the one with the most uses
                    // (most data = most reliable estimate).
                    domain_variants
                        .iter()
                        .max_by_key(|v| v.total_uses())
                });

            best_exploitation.map(|v| (*v).clone())
        }
    }

    /// Record the outcome of a task execution.
    pub fn record_outcome(&self, outcome: &TaskOutcome) -> Result<(), String> {
        let key = (outcome.domain.clone(), outcome.variant_id.clone());

        let mut variants = self.variants.lock().unwrap();
        let variant = variants
            .get_mut(&key)
            .ok_or_else(|| format!("Variant '{}' not found for {:?}", outcome.variant_id, outcome.domain))?;

        if outcome.success {
            variant.successes += 1;
        } else {
            variant.failures += 1;
        }
        variant.last_used = Utc::now();

        // Update domain totals.
        let mut domain_totals = self.domain_totals.lock().unwrap();
        *domain_totals.entry(outcome.domain.clone()).or_insert(0) += 1;

        // Persist to SQLite.
        self.persist_variant_update(variant)?;

        Ok(())
    }

    /// Get all variants for a domain.
    pub fn variants_for_domain(&self, domain: &TaskDomain) -> Vec<PromptVariant> {
        let variants = self.variants.lock().unwrap();
        variants
            .iter()
            .filter(|((d, _), _)| d == domain)
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// Get the best variant for a domain (highest success rate with enough data).
    pub fn best_variant(&self, domain: &TaskDomain) -> Option<PromptVariant> {
        let variants = self.variants.lock().unwrap();
        variants
            .iter()
            .filter(|((d, _), v)| d == domain && v.total_uses() >= self.config.min_uses_for_convergence)
            .map(|(_, v)| v.clone())
            .max_by(|a, b| {
                a.success_rate()
                    .partial_cmp(&b.success_rate())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Get the total number of outcomes recorded across all domains.
    pub fn total_outcomes(&self) -> u64 {
        let domain_totals = self.domain_totals.lock().unwrap();
        domain_totals.values().sum()
    }

    /// Persist a new variant to SQLite.
    fn persist_variant(&self, variant: &PromptVariant) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO prompt_variants (id, domain, template, successes, failures, created_at, last_used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                variant.id,
                variant.domain.as_str(),
                variant.template,
                variant.successes as i64,
                variant.failures as i64,
                variant.created_at.to_rfc3339(),
                variant.last_used.to_rfc3339(),
            ],
        )
        .map_err(|e| format!("Failed to persist variant: {}", e))?;
        Ok(())
    }

    /// Update a variant's stats in SQLite.
    fn persist_variant_update(&self, variant: &PromptVariant) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE prompt_variants SET successes = ?1, failures = ?2, last_used = ?3
             WHERE id = ?4 AND domain = ?5",
            rusqlite::params![
                variant.successes as i64,
                variant.failures as i64,
                variant.last_used.to_rfc3339(),
                variant.id,
                variant.domain.as_str(),
            ],
        )
        .map_err(|e| format!("Failed to update variant: {}", e))?;
        Ok(())
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_optimizer() -> PromptOptimizer {
        PromptOptimizer::new(PromptOptimizerConfig::default())
    }

    // ── Registration Tests ──────────────────────────────────────────────────

    #[test]
    fn test_register_variant() {
        let opt = make_optimizer();
        opt.register_variant(TaskDomain::Planning, "v1", "Plan step by step: {goal}")
            .unwrap();

        let variants = opt.variants_for_domain(&TaskDomain::Planning);
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].id, "v1");
    }

    #[test]
    fn test_register_duplicate_variant_rejected() {
        let opt = make_optimizer();
        opt.register_variant(TaskDomain::Planning, "v1", "template 1").unwrap();
        let result = opt.register_variant(TaskDomain::Planning, "v1", "template 2");
        assert!(result.is_err());
    }

    #[test]
    fn test_max_variants_per_domain() {
        let config = PromptOptimizerConfig {
            max_variants_per_domain: 2,
            ..Default::default()
        };
        let opt = PromptOptimizer::new(config);

        opt.register_variant(TaskDomain::Planning, "v1", "t1").unwrap();
        opt.register_variant(TaskDomain::Planning, "v2", "t2").unwrap();
        let result = opt.register_variant(TaskDomain::Planning, "v3", "t3");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Maximum"));
    }

    // ── Epsilon-Greedy Selection Tests ──────────────────────────────────────

    #[test]
    fn test_select_returns_none_for_empty_domain() {
        let opt = make_optimizer();
        assert!(opt.select_variant(&TaskDomain::Planning).is_none());
    }

    #[test]
    fn test_select_returns_only_variant() {
        let opt = make_optimizer();
        opt.register_variant(TaskDomain::Planning, "v1", "template").unwrap();

        let selected = opt.select_variant(&TaskDomain::Planning);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().id, "v1");
    }

    #[test]
    fn test_exploit_selects_highest_success_rate() {
        let opt = make_optimizer();

        // Register two variants.
        opt.register_variant(TaskDomain::Planning, "good", "good template").unwrap();
        opt.register_variant(TaskDomain::Planning, "bad", "bad template").unwrap();

        // Give "good" variant 20 successes.
        for _ in 0..20 {
            opt.record_outcome(&TaskOutcome {
                domain: TaskDomain::Planning,
                variant_id: "good".to_string(),
                success: true,
                notes: None,
            })
            .unwrap();
        }

        // Give "bad" variant 20 failures.
        for _ in 0..20 {
            opt.record_outcome(&TaskOutcome {
                domain: TaskDomain::Planning,
                variant_id: "bad".to_string(),
                success: false,
                notes: None,
            })
            .unwrap();
        }

        // Run selection many times — with epsilon=0.1, ~90% should pick "good".
        let mut good_count = 0;
        let iterations = 1000;
        for _ in 0..iterations {
            if let Some(v) = opt.select_variant(&TaskDomain::Planning) {
                if v.id == "good" {
                    good_count += 1;
                }
            }
        }

        // With epsilon=0.1, "good" should be selected ~90% of the time.
        // Allow some variance: at least 70%.
        let ratio = good_count as f64 / iterations as f64;
        assert!(
            ratio > 0.70,
            "Expected 'good' to be selected >70% of the time, got {:.1}%",
            ratio * 100.0
        );
    }

    #[test]
    fn test_exploration_chooses_less_tried_variant() {
        let config = PromptOptimizerConfig {
            epsilon: 1.0, // Always explore.
            ..Default::default()
        };
        let opt = PromptOptimizer::new(config);

        opt.register_variant(TaskDomain::Planning, "tried", "tried template").unwrap();
        opt.register_variant(TaskDomain::Planning, "untried", "untried template").unwrap();

        // Give "tried" variant many uses.
        for _ in 0..50 {
            opt.record_outcome(&TaskOutcome {
                domain: TaskDomain::Planning,
                variant_id: "tried".to_string(),
                success: true,
                notes: None,
            })
            .unwrap();
        }

        // With epsilon=1.0 (always explore), UCB1 should prefer "untried"
        // because it has never been tried (infinite UCB1 score).
        let selected = opt.select_variant(&TaskDomain::Planning).unwrap();
        assert_eq!(selected.id, "untried");
    }

    // ── Outcome Recording Tests ─────────────────────────────────────────────

    #[test]
    fn test_record_outcome_updates_stats() {
        let opt = make_optimizer();
        opt.register_variant(TaskDomain::Planning, "v1", "template").unwrap();

        opt.record_outcome(&TaskOutcome {
            domain: TaskDomain::Planning,
            variant_id: "v1".to_string(),
            success: true,
            notes: None,
        })
        .unwrap();

        opt.record_outcome(&TaskOutcome {
            domain: TaskDomain::Planning,
            variant_id: "v1".to_string(),
            success: false,
            notes: None,
        })
        .unwrap();

        let variants = opt.variants_for_domain(&TaskDomain::Planning);
        assert_eq!(variants[0].successes, 1);
        assert_eq!(variants[0].failures, 1);
        assert_eq!(variants[0].total_uses(), 2);
    }

    #[test]
    fn test_record_outcome_for_unknown_variant_fails() {
        let opt = make_optimizer();
        let result = opt.record_outcome(&TaskOutcome {
            domain: TaskDomain::Planning,
            variant_id: "nonexistent".to_string(),
            success: true,
            notes: None,
        });
        assert!(result.is_err());
    }

    // ── Success Rate Tests ──────────────────────────────────────────────────

    #[test]
    fn test_variant_success_rate() {
        let variant = PromptVariant {
            id: "test".to_string(),
            domain: TaskDomain::Planning,
            template: "test".to_string(),
            successes: 7,
            failures: 3,
            created_at: Utc::now(),
            last_used: Utc::now(),
        };
        assert!((variant.success_rate() - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_variant_success_rate_neutral_prior() {
        let variant = PromptVariant {
            id: "test".to_string(),
            domain: TaskDomain::Planning,
            template: "test".to_string(),
            successes: 0,
            failures: 0,
            created_at: Utc::now(),
            last_used: Utc::now(),
        };
        assert!((variant.success_rate() - 0.5).abs() < 0.001);
    }

    // ── UCB1 Score Tests ────────────────────────────────────────────────────

    #[test]
    fn test_ucb1_untried_variant_has_max_score() {
        let variant = PromptVariant {
            id: "test".to_string(),
            domain: TaskDomain::Planning,
            template: "test".to_string(),
            successes: 0,
            failures: 0,
            created_at: Utc::now(),
            last_used: Utc::now(),
        };
        assert_eq!(variant.ucb1_score(100), f64::MAX);
    }

    #[test]
    fn test_ucb1_decreases_with_more_uses() {
        let v1 = PromptVariant {
            id: "v1".to_string(),
            domain: TaskDomain::Planning,
            template: "test".to_string(),
            successes: 5,
            failures: 5,
            created_at: Utc::now(),
            last_used: Utc::now(),
        };
        let v2 = PromptVariant {
            id: "v2".to_string(),
            domain: TaskDomain::Planning,
            template: "test".to_string(),
            successes: 50,
            failures: 50,
            created_at: Utc::now(),
            last_used: Utc::now(),
        };

        // Both have 50% success rate, but v1 has fewer uses → higher UCB1.
        assert!(v1.ucb1_score(200) > v2.ucb1_score(200));
    }

    // ── Best Variant Tests ──────────────────────────────────────────────────

    #[test]
    fn test_best_variant_requires_min_uses() {
        let config = PromptOptimizerConfig {
            min_uses_for_convergence: 5,
            ..Default::default()
        };
        let opt = PromptOptimizer::new(config);

        opt.register_variant(TaskDomain::Planning, "v1", "template").unwrap();

        // 3 uses (below threshold).
        for _ in 0..3 {
            opt.record_outcome(&TaskOutcome {
                domain: TaskDomain::Planning,
                variant_id: "v1".to_string(),
                success: true,
                notes: None,
            })
            .unwrap();
        }

        // Not enough uses for convergence.
        assert!(opt.best_variant(&TaskDomain::Planning).is_none());
    }

    #[test]
    fn test_best_variant_returns_highest_after_convergence() {
        let config = PromptOptimizerConfig {
            min_uses_for_convergence: 3,
            ..Default::default()
        };
        let opt = PromptOptimizer::new(config);

        opt.register_variant(TaskDomain::Planning, "good", "good template").unwrap();
        opt.register_variant(TaskDomain::Planning, "bad", "bad template").unwrap();

        for _ in 0..5 {
            opt.record_outcome(&TaskOutcome {
                domain: TaskDomain::Planning,
                variant_id: "good".to_string(),
                success: true,
                notes: None,
            })
            .unwrap();
        }
        for _ in 0..5 {
            opt.record_outcome(&TaskOutcome {
                domain: TaskDomain::Planning,
                variant_id: "bad".to_string(),
                success: false,
                notes: None,
            })
            .unwrap();
        }

        let best = opt.best_variant(&TaskDomain::Planning).unwrap();
        assert_eq!(best.id, "good");
    }

    // ── SQLite Persistence Tests ────────────────────────────────────────────

    #[test]
    fn test_outcomes_persisted_to_sqlite() {
        let opt = make_optimizer();
        opt.register_variant(TaskDomain::Planning, "v1", "template").unwrap();

        for _ in 0..10 {
            opt.record_outcome(&TaskOutcome {
                domain: TaskDomain::Planning,
                variant_id: "v1".to_string(),
                success: true,
                notes: None,
            })
            .unwrap();
        }

        // Verify SQLite has the data.
        let conn = opt.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT successes FROM prompt_variants WHERE id = 'v1' AND domain = 'planning'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 10);
    }

    // ── Total Outcomes Tests ────────────────────────────────────────────────

    #[test]
    fn test_total_outcomes() {
        let opt = make_optimizer();
        opt.register_variant(TaskDomain::Planning, "v1", "t1").unwrap();
        opt.register_variant(TaskDomain::FileOps, "v2", "t2").unwrap();

        for _ in 0..5 {
            opt.record_outcome(&TaskOutcome {
                domain: TaskDomain::Planning,
                variant_id: "v1".to_string(),
                success: true,
                notes: None,
            })
            .unwrap();
        }
        for _ in 0..3 {
            opt.record_outcome(&TaskOutcome {
                domain: TaskDomain::FileOps,
                variant_id: "v2".to_string(),
                success: false,
                notes: None,
            })
            .unwrap();
        }

        assert_eq!(opt.total_outcomes(), 8);
    }

    // ── Domain Isolation Tests ──────────────────────────────────────────────

    #[test]
    fn test_variants_isolated_by_domain() {
        let opt = make_optimizer();
        opt.register_variant(TaskDomain::Planning, "v1", "planning template").unwrap();
        opt.register_variant(TaskDomain::FileOps, "v1", "file ops template").unwrap();

        let planning_variants = opt.variants_for_domain(&TaskDomain::Planning);
        let file_ops_variants = opt.variants_for_domain(&TaskDomain::FileOps);

        assert_eq!(planning_variants.len(), 1);
        assert_eq!(file_ops_variants.len(), 1);
        assert_eq!(planning_variants[0].template, "planning template");
        assert_eq!(file_ops_variants[0].template, "file ops template");
    }
}
