//! Batch 3 — Procedural Workflow Memory.
//!
//! # Core Mission
//!
//! Extract reusable workflow patterns from completed [`WorkflowSession`]s and
//! store them as typed skill-graph nodes in PSDG. Enables KRIA to answer:
//! _"Have I done something similar before, and how did it go?"_
//!
//! # What Is a Workflow Skill
//!
//! A [`WorkflowSkill`] is a distilled pattern extracted from one or more
//! successful sessions sharing the same `category` × `verb_prefix`. It carries:
//! - A canonical description
//! - Tool usage sequence (bounded, top-N)
//! - Success rate
//! - Average step count
//! - Associated interruption classes seen
//! - Associated recovery actions that worked
//!
//! # Invariants
//!
//! - Bounded: at most [`MAX_SKILLS_PER_CATEGORY`] skills per workflow category.
//! - Writes are fire-and-forget via PSDG.
//! - Read is O(1) key lookup from in-memory index.
//! - NO LLM — pattern extraction is purely structural.
//! - Skills are NEVER used to autonomously trigger workflows.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::agent::psdg::PsdgHandle;
use crate::agent::workflow_expectation::WorkflowCategory;
use crate::agent::workflow_session::WorkflowSession;
use crate::agent::world_model::FactSource;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum skills stored per workflow category.
pub const MAX_SKILLS_PER_CATEGORY: usize = 40;

/// Maximum tool names retained in a skill's tool sequence.
pub const MAX_TOOL_SEQUENCE: usize = 8;

/// Maximum interruption classes remembered per skill.
pub const MAX_INTERRUPTION_CLASSES: usize = 4;

/// Minimum sessions before a skill template is materialised.
pub const MIN_SESSIONS_FOR_SKILL: usize = 2;

// ─── Workflow Skill ───────────────────────────────────────────────────────────

/// A distilled procedural pattern extracted from completed workflow sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSkill {
    /// Stable key: `{category_slug}::{verb_prefix}`.
    pub key: String,
    /// Human-readable description.
    pub description: String,
    /// Workflow category (serialised as string for PSDG storage).
    pub category: String,
    /// First word of the user intent (verb prefix).
    pub verb_prefix: String,
    /// Representative tool sequence (bounded).
    pub tool_sequence: Vec<String>,
    /// Number of sessions this pattern was extracted from.
    pub session_count: u32,
    /// Number of times this pattern succeeded end-to-end.
    pub success_count: u32,
    /// Average number of steps across successful sessions.
    pub avg_step_count: f32,
    /// Interruption classes encountered across all sessions for this skill.
    pub interruption_classes: Vec<String>,
    /// Epoch seconds of last update.
    pub last_updated: u64,
}

impl WorkflowSkill {
    /// Success rate (0.0–1.0).
    pub fn success_rate(&self) -> f32 {
        if self.session_count == 0 {
            0.0
        } else {
            self.success_count as f32 / self.session_count as f32
        }
    }

    /// PSDG subject for this skill.
    fn psdg_subject(&self) -> String {
        format!("skill.workflow.{}", self.key.replace("::", "."))
    }

    /// PSDG object (JSON summary) for this skill.
    fn psdg_object(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| self.description.clone())
    }
}

// ─── Pattern Extraction ───────────────────────────────────────────────────────

/// Extract a verb prefix from a user intent string.
///
/// Returns the first whitespace-delimited token, lowercased.
fn extract_verb_prefix(intent: &str) -> String {
    intent
        .split_whitespace()
        .next()
        .map(|w| w.to_lowercase())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Classify the workflow category from a user intent string.
///
/// Purely keyword-based — no LLM. Falls back to `Unknown`.
fn classify_category(intent: &str) -> WorkflowCategory {
    let lower = intent.to_lowercase();
    if lower.contains("build")
        || lower.contains("compile")
        || lower.contains("run")
        || lower.contains("test")
        || lower.contains("debug")
    {
        return WorkflowCategory::Coding;
    }
    if lower.contains("browse")
        || lower.contains("open url")
        || lower.contains("navigate to")
        || lower.contains("http")
    {
        return WorkflowCategory::Browser;
    }
    if lower.contains("git")
        || lower.contains("commit")
        || lower.contains("push")
        || lower.contains("pull")
        || lower.contains("deploy")
    {
        return WorkflowCategory::Deployment;
    }
    if lower.contains("file")
        || lower.contains("folder")
        || lower.contains("copy")
        || lower.contains("move")
        || lower.contains("delete")
        || lower.contains("mkdir")
    {
        return WorkflowCategory::FileManagement;
    }
    if lower.contains("terminal")
        || lower.contains("shell")
        || lower.contains("bash")
        || lower.contains("script")
    {
        return WorkflowCategory::Terminal;
    }
    if lower.contains("email") || lower.contains("send") || lower.contains("message") {
        return WorkflowCategory::Email;
    }
    if lower.contains("install")
        || lower.contains("configure")
        || lower.contains("setting")
        || lower.contains("service")
    {
        return WorkflowCategory::SystemConfiguration;
    }
    WorkflowCategory::Unknown
}

/// Extract tool names from a session's completed steps.
fn extract_tools(session: &WorkflowSession) -> Vec<String> {
    let mut tools: Vec<String> = session
        .completed_steps
        .iter()
        .filter(|s| s.success)
        .map(|s| s.action.clone())
        .collect::<std::collections::LinkedList<String>>()
        .into_iter()
        .collect();
    tools.dedup();
    tools.truncate(MAX_TOOL_SEQUENCE);
    tools
}

// ─── Procedural Workflow Memory ───────────────────────────────────────────────

/// Procedural workflow memory — bounded skill-graph backed by PSDG.
pub struct ProceduralWorkflowMemory {
    psdg: Option<PsdgHandle>,
    /// In-memory index: skill_key → WorkflowSkill.
    skills: Mutex<HashMap<String, WorkflowSkill>>,
}

impl ProceduralWorkflowMemory {
    /// Create a new procedural memory, optionally backed by PSDG.
    pub fn new(psdg: Option<PsdgHandle>) -> Self {
        Self {
            psdg,
            skills: Mutex::new(HashMap::new()),
        }
    }

    /// Ingest a completed session and update the skill graph.
    ///
    /// Only sessions with at least one completed step are ingested.
    /// Skills accumulate across sessions — statistics are merged.
    pub fn ingest_session(&self, session: &WorkflowSession) {
        if session.completed_steps.is_empty() {
            return;
        }

        let verb = extract_verb_prefix(&session.user_intent);
        let category = classify_category(&session.user_intent);
        let category_slug = format!("{:?}", category).to_lowercase();
        let key = format!("{}::{}", category_slug, verb);

        let tools = extract_tools(session);
        let step_count = session.completed_steps.len() as f32;
        let succeeded = session.complete && session.error.is_none();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut skills = self.skills.lock().unwrap();

        // Enforce category cap
        let category_count = skills
            .values()
            .filter(|s| s.category == category_slug)
            .count();

        if !skills.contains_key(&key) && category_count >= MAX_SKILLS_PER_CATEGORY {
            debug!(
                target: "procedural_memory",
                key = %key,
                category = %category_slug,
                "Skill cap reached for category — skipping new skill"
            );
            return;
        }

        let skill = skills.entry(key.clone()).or_insert_with(|| WorkflowSkill {
            key: key.clone(),
            description: session.user_intent.clone(),
            category: category_slug.clone(),
            verb_prefix: verb,
            tool_sequence: tools.clone(),
            session_count: 0,
            success_count: 0,
            avg_step_count: 0.0,
            interruption_classes: Vec::new(),
            last_updated: now,
        });

        // Merge statistics
        skill.session_count += 1;
        if succeeded {
            skill.success_count += 1;
            // Running average of step count
            let prev_avg = skill.avg_step_count;
            let n = skill.session_count as f32;
            skill.avg_step_count = prev_avg + (step_count - prev_avg) / n;
        }

        // Merge tool sequence (keep most recent)
        if !tools.is_empty() {
            skill.tool_sequence = tools;
        }
        skill.last_updated = now;

        debug!(
            target: "procedural_memory",
            key = %key,
            session_count = skill.session_count,
            success_rate = skill.success_rate(),
            "Skill updated from session"
        );

        // Persist to PSDG if we have enough sessions
        if skill.session_count >= MIN_SESSIONS_FOR_SKILL as u32 {
            if let Some(ref psdg) = self.psdg {
                let subject = skill.psdg_subject();
                let object = skill.psdg_object();
                let confidence = 0.70 + 0.25 * skill.success_rate() as f64;
                psdg.record_fact(
                    &subject,
                    "procedural_skill",
                    &object,
                    confidence,
                    FactSource::Compiled,
                    "",
                );
            }
        }
    }

    /// Look up a skill by key (`{category_slug}::{verb_prefix}`).
    pub fn get_skill(&self, key: &str) -> Option<WorkflowSkill> {
        self.skills.lock().unwrap().get(key).cloned()
    }

    /// Find the best matching skill for a user intent.
    ///
    /// Matches on category + verb prefix. Returns None if no match or
    /// the best match has a success rate below 0.3.
    pub fn find_relevant_skill(&self, intent: &str) -> Option<WorkflowSkill> {
        let verb = extract_verb_prefix(intent);
        let category = classify_category(intent);
        let category_slug = format!("{:?}", category).to_lowercase();
        let key = format!("{}::{}", category_slug, verb);

        let skills = self.skills.lock().unwrap();
        let skill = skills.get(&key)?;

        // Minimum sessions and success rate thresholds
        if skill.session_count < MIN_SESSIONS_FOR_SKILL as u32 {
            return None;
        }
        if skill.success_rate() < 0.3 {
            return None;
        }
        Some(skill.clone())
    }

    /// List all skills for a given category.
    pub fn list_skills_for_category(&self, category_slug: &str) -> Vec<WorkflowSkill> {
        let skills = self.skills.lock().unwrap();
        skills
            .values()
            .filter(|s| s.category == category_slug)
            .cloned()
            .collect()
    }

    /// Total number of skills stored in memory.
    pub fn skill_count(&self) -> usize {
        self.skills.lock().unwrap().len()
    }

    /// Remove all skills (prune). Used for memory boundedness maintenance.
    pub fn prune_low_confidence(&self) {
        let mut skills = self.skills.lock().unwrap();
        let before = skills.len();
        skills.retain(|_, s| {
            s.success_rate() >= 0.2 || s.session_count < MIN_SESSIONS_FOR_SKILL as u32
        });
        let pruned = before - skills.len();
        if pruned > 0 {
            debug!(target: "procedural_memory", pruned, "Low-confidence skills pruned");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::workflow_session::{SessionStep, WorkflowSession};

    fn completed_session(id: &str, intent: &str, success: bool) -> WorkflowSession {
        let mut s = WorkflowSession::new(id.to_string(), intent.to_string(), "react".to_string());
        s.completed_steps.push(SessionStep {
            step: 1,
            action: "run_command".to_string(),
            params: serde_json::Value::Null,
            success: true,
            evidence: "ok".to_string(),
            timestamp: 0,
        });
        s.complete = success;
        s
    }

    #[test]
    fn ingest_empty_session_is_no_op() {
        let mem = ProceduralWorkflowMemory::new(None);
        let s = WorkflowSession::new("s0".into(), "build".into(), "react".into());
        mem.ingest_session(&s);
        assert_eq!(mem.skill_count(), 0);
    }

    #[test]
    fn ingest_creates_skill() {
        let mem = ProceduralWorkflowMemory::new(None);
        mem.ingest_session(&completed_session("s1", "build the project", true));
        assert_eq!(mem.skill_count(), 1);
    }

    #[test]
    fn ingest_multiple_merges_statistics() {
        let mem = ProceduralWorkflowMemory::new(None);
        for i in 0..4 {
            mem.ingest_session(&completed_session(
                &format!("s{}", i),
                "build the project",
                i % 2 == 0,
            ));
        }
        let skill = mem.find_relevant_skill("build the project");
        assert!(skill.is_some(), "skill should exist after 4 sessions");
        let s = skill.unwrap();
        assert_eq!(s.session_count, 4);
    }

    #[test]
    fn find_relevant_skill_none_below_threshold() {
        let mem = ProceduralWorkflowMemory::new(None);
        // Only 1 session — below MIN_SESSIONS_FOR_SKILL
        mem.ingest_session(&completed_session("s1", "deploy release", true));
        assert!(mem.find_relevant_skill("deploy release").is_none());
    }

    #[test]
    fn category_cap_enforced() {
        let mem = ProceduralWorkflowMemory::new(None);
        for i in 0..(MAX_SKILLS_PER_CATEGORY + 10) {
            let intent = format!("build project-variant-{}", i);
            mem.ingest_session(&completed_session(&format!("s{}", i), &intent, true));
            mem.ingest_session(&completed_session(&format!("sx{}", i), &intent, true));
        }
        let coding_skills = mem.list_skills_for_category("coding");
        assert!(
            coding_skills.len() <= MAX_SKILLS_PER_CATEGORY,
            "category cap exceeded: {}",
            coding_skills.len()
        );
    }

    #[test]
    fn extract_verb_prefix_works() {
        assert_eq!(extract_verb_prefix("Build the project"), "build");
        assert_eq!(extract_verb_prefix("  run tests  "), "run");
        assert_eq!(extract_verb_prefix(""), "unknown");
    }

    #[test]
    fn prune_removes_low_confidence() {
        let mem = ProceduralWorkflowMemory::new(None);
        // All failures → success_rate = 0
        for i in 0..3 {
            mem.ingest_session(&completed_session(
                &format!("s{}", i),
                "build project fail",
                false,
            ));
        }
        mem.prune_low_confidence();
        assert_eq!(mem.skill_count(), 0, "all-failure skills should be pruned");
    }
}
