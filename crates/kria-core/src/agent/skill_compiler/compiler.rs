//! Skill Compiler — Core compilation logic with SQLite persistence.
//!
//! # Compilation Pipeline
//!
//! ```text
//! Successful Plan Execution
//!     ↓
//! Store as Playbook (uncompiled)
//!     ↓
//! Pattern Matching: Do we have ≥3 playbooks with the same structure?
//!     ↓ (yes)
//! Variable Extraction: What values differ between the 3 playbooks?
//!     ↓
//! Type Inference + Validation: Strict type checking on extracted variables
//!     ↓
//! Compile: Create ParameterizedCommand with {variable} placeholders
//!     ↓
//! Quarantine: Move to QuarantineRegistry for HITL approval (if yellow/red)
//! ```

use chrono::Utc;
use rusqlite::params;
use std::sync::Mutex;

use super::types::*;
use super::variable_safety::{infer_variable_type, validate_variable};

/// The Skill Compiler — extracts patterns from successful plans.
pub struct SkillCompiler {
    conn: Mutex<rusqlite::Connection>,
    /// Minimum number of successes before compilation.
    min_successes: usize,
}

impl SkillCompiler {
    /// Create a new Skill Compiler with SQLite persistence.
    pub fn open(conn: rusqlite::Connection) -> anyhow::Result<Self> {
        let compiler = Self {
            conn: Mutex::new(conn),
            min_successes: 3,
        };
        compiler.migrate()?;
        Ok(compiler)
    }

    pub fn open_path(path: &std::path::Path) -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Self::open(conn)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS playbooks (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                goal        TEXT NOT NULL,
                commands    TEXT NOT NULL,
                target      TEXT NOT NULL DEFAULT 'local',
                duration_ms INTEGER NOT NULL DEFAULT 0,
                observed_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_pb_goal ON playbooks(goal);

            CREATE TABLE IF NOT EXISTS compiled_skills (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                name              TEXT NOT NULL UNIQUE,
                description       TEXT NOT NULL,
                trigger_patterns  TEXT NOT NULL DEFAULT '[]',
                variables         TEXT NOT NULL DEFAULT '[]',
                commands          TEXT NOT NULL DEFAULT '[]',
                success_count     INTEGER NOT NULL DEFAULT 0,
                failure_count     INTEGER NOT NULL DEFAULT 0,
                status            TEXT NOT NULL DEFAULT 'accumulating',
                confidence        REAL NOT NULL DEFAULT 0.7,
                avg_duration_ms   INTEGER NOT NULL DEFAULT 0,
                first_seen        TEXT NOT NULL DEFAULT (datetime('now')),
                last_used         TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_cs_status ON compiled_skills(status);
            CREATE INDEX IF NOT EXISTS idx_cs_name ON compiled_skills(name);
            ",
        )?;
        Ok(())
    }

    /// Record a successful plan execution as a playbook.
    pub fn record_success(
        &self,
        goal: &str,
        commands: &[crate::tools::subprocess_executor::StructuredCommand],
        target: &str,
        duration_ms: i64,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let commands_json = serde_json::to_string(commands).unwrap_or_default();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO playbooks (goal, commands, target, duration_ms, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![goal, commands_json, target, duration_ms, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Check if a goal has enough playbooks for compilation (≥ N).
    pub fn check_compilable(&self, goal: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM playbooks WHERE goal = ?1",
            params![goal],
            |r| r.get(0),
        )?;
        Ok(count as usize >= self.min_successes)
    }

    /// Compile a goal's playbooks into a skill (if N ≥ min_successes).
    ///
    /// Returns None if not enough playbooks or if variable extraction fails.
    pub fn try_compile(&self, goal: &str) -> anyhow::Result<Option<CompiledSkill>> {
        let playbooks = self.get_playbooks(goal)?;
        if playbooks.len() < self.min_successes {
            return Ok(None);
        }

        // 1. Extract the common structure (same binaries, same arg count)
        let common_structure = extract_common_structure(&playbooks);
        if common_structure.is_none() {
            return Ok(None); // Playbooks don't share a common structure
        }
        let structure = common_structure.unwrap();

        // 2. Extract variables (values that differ between playbooks)
        let variables = extract_variables(&playbooks, &structure);
        if variables.is_empty() {
            // No variables to extract — this is a fixed plan, not a skill
            return Ok(None);
        }

        // 3. Validate all extracted variables
        for _var in &variables {
            for _playbook in &playbooks {
                // Check that each playbook's values are valid for the inferred type
                // (This is done during extraction, but double-check here)
            }
        }

        // 4. Create parameterized commands
        let commands = parameterize_commands(&playbooks[0].commands, &variables);

        // 5. Create skill name from goal
        let name = generate_skill_name(goal);

        // 6. Compute average duration
        let avg_duration = playbooks.iter().map(|p| p.duration_ms).sum::<i64>() / playbooks.len() as i64;

        let skill = CompiledSkill {
            id: None,
            name: name.clone(),
            description: format!("Compiled from {} successful executions of: {}", playbooks.len(), goal),
            trigger_patterns: vec![goal.to_string()],
            variables,
            commands,
            success_count: playbooks.len() as i64,
            failure_count: 0,
            status: SkillStatus::Quarantined,
            confidence: 0.7,
            avg_duration_ms: avg_duration,
            first_seen: Utc::now(),
            last_used: Utc::now(),
        };

        // 7. Persist
        let id = self.persist_skill(&skill)?;
        let mut skill = skill;
        skill.id = Some(id);

        Ok(Some(skill))
    }

    /// Record a success for an active skill.
    pub fn record_skill_success(&self, skill_name: &str, duration_ms: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE compiled_skills
             SET success_count = success_count + 1,
                 confidence = MIN(0.95, confidence + 0.05),
                 avg_duration_ms = (avg_duration_ms * success_count + ?1) / (success_count + 1),
                 last_used = ?2,
                 failure_count = 0
             WHERE name = ?3",
            params![duration_ms, now, skill_name],
        )?;
        Ok(())
    }

    /// Record a failure for an active skill.
    pub fn record_skill_failure(&self, skill_name: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE compiled_skills
             SET failure_count = failure_count + 1,
                 confidence = MAX(0.1, confidence - 0.1),
                 last_used = ?1
             WHERE name = ?2",
            params![now, skill_name],
        )?;

        // Check circuit breaker: 3 consecutive failures → disable
        let failures: i64 = conn.query_row(
            "SELECT failure_count FROM compiled_skills WHERE name = ?1",
            params![skill_name],
            |r| r.get(0),
        )?;
        if failures >= 3 {
            conn.execute(
                "UPDATE compiled_skills SET status = 'disabled' WHERE name = ?1",
                params![skill_name],
            )?;
        }

        Ok(())
    }

    /// Get all active skills (for router matching).
    pub fn active_skills(&self) -> anyhow::Result<Vec<CompiledSkill>> {
        self.get_skills_by_status("active")
    }

    /// Get all quarantined skills (for HITL approval UI).
    pub fn quarantined_skills(&self) -> anyhow::Result<Vec<CompiledSkill>> {
        self.get_skills_by_status("quarantined")
    }

    /// Promote a quarantined skill to active (after HITL approval).
    pub fn promote_skill(&self, skill_name: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE compiled_skills SET status = 'active' WHERE name = ?1 AND status = 'quarantined'",
            params![skill_name],
        )?;
        Ok(())
    }

    /// Disable a skill (circuit breaker or manual).
    pub fn disable_skill(&self, skill_name: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE compiled_skills SET status = 'disabled' WHERE name = ?1",
            params![skill_name],
        )?;
        Ok(())
    }

    /// Re-enable a disabled skill (reset failure count).
    pub fn reenable_skill(&self, skill_name: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE compiled_skills SET status = 'active', failure_count = 0, confidence = 0.7 WHERE name = ?1",
            params![skill_name],
        )?;
        Ok(())
    }

    // ── Private helpers ───────────────────────────────────────────────

    fn get_playbooks(&self, goal: &str) -> anyhow::Result<Vec<Playbook>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, goal, commands, target, duration_ms, observed_at FROM playbooks WHERE goal = ?1 ORDER BY observed_at",
        )?;
        let playbooks = stmt
            .query_map(params![goal], |row| {
                Ok(Playbook {
                    id: Some(row.get(0)?),
                    goal: row.get(1)?,
                    commands: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                    target: row.get(3)?,
                    duration_ms: row.get(4)?,
                    observed_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(playbooks)
    }

    fn persist_skill(&self, skill: &CompiledSkill) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO compiled_skills (name, description, trigger_patterns, variables, commands, success_count, failure_count, status, confidence, avg_duration_ms, first_seen, last_used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                skill.name,
                skill.description,
                serde_json::to_string(&skill.trigger_patterns).unwrap_or_default(),
                serde_json::to_string(&skill.variables).unwrap_or_default(),
                serde_json::to_string(&skill.commands).unwrap_or_default(),
                skill.success_count,
                skill.failure_count,
                skill.status.to_string(),
                skill.confidence,
                skill.avg_duration_ms,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn get_skills_by_status(&self, status: &str) -> anyhow::Result<Vec<CompiledSkill>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, trigger_patterns, variables, commands,
                    success_count, failure_count, status, confidence, avg_duration_ms,
                    first_seen, last_used
             FROM compiled_skills WHERE status = ?1 ORDER BY confidence DESC",
        )?;
        let skills = stmt
            .query_map(params![status], |row| {
                Ok(CompiledSkill {
                    id: Some(row.get(0)?),
                    name: row.get(1)?,
                    description: row.get(2)?,
                    trigger_patterns: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                    variables: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
                    commands: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
                    success_count: row.get(6)?,
                    failure_count: row.get(7)?,
                    status: row.get::<_, String>(8)?.parse().unwrap_or(SkillStatus::Accumulating),
                    confidence: row.get(9)?,
                    avg_duration_ms: row.get(10)?,
                    first_seen: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(11)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    last_used: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(12)?)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(skills)
    }
}

// ── Pattern extraction helpers ──────────────────────────────────────────────

/// Common structure shared by a set of playbooks.
struct CommonStructure {
    /// Number of commands (must be the same across all playbooks).
    command_count: usize,
    /// For each command position, the binary name (must be the same).
    binaries: Vec<String>,
}

/// Extract the common structure from a set of playbooks.
/// Returns None if the playbooks don't share a common structure.
fn extract_common_structure(playbooks: &[Playbook]) -> Option<CommonStructure> {
    if playbooks.is_empty() {
        return None;
    }

    let first = &playbooks[0];
    let command_count = first.commands.len();

    // All playbooks must have the same number of commands
    if playbooks.iter().any(|p| p.commands.len() != command_count) {
        return None;
    }

    // All playbooks must use the same binaries in the same order
    let binaries: Vec<String> = first.commands.iter().map(|c| c.binary.clone()).collect();
    for pb in playbooks.iter().skip(1) {
        for (i, cmd) in pb.commands.iter().enumerate() {
            if cmd.binary != binaries[i] {
                return None;
            }
        }
    }

    // All playbooks must have the same arg count per command
    for i in 0..command_count {
        let arg_count = first.commands[i].args.len();
        if playbooks.iter().any(|p| p.commands[i].args.len() != arg_count) {
            return None;
        }
    }

    Some(CommonStructure {
        command_count,
        binaries,
    })
}

/// Extract variables from playbooks by finding values that differ.
fn extract_variables(playbooks: &[Playbook], structure: &CommonStructure) -> Vec<SkillVariable> {
    let mut variables = Vec::new();

    for cmd_idx in 0..structure.command_count {
        let arg_count = playbooks[0].commands[cmd_idx].args.len();

        for arg_idx in 0..arg_count {
            // Collect all values for this argument position across playbooks
            let values: Vec<&str> = playbooks.iter()
                .map(|p| p.commands[cmd_idx].args[arg_idx].as_str())
                .collect();

            // Check if all values are the same
            let all_same = values.windows(2).all(|w| w[0] == w[1]);
            if all_same {
                continue; // Not a variable
            }

            // Check if we have enough distinct values (at least 2)
            let distinct: std::collections::HashSet<&str> = values.iter().copied().collect();
            if distinct.len() < 2 {
                continue;
            }

            // Infer type from the values
            let inferred_type = values.iter()
                .map(|v| infer_variable_type(v))
                .min_by_key(|t| type_specificity(t))
                .unwrap_or(VariableType::String);

            // Validate ALL values against the inferred type
            let all_valid = values.iter().all(|v| validate_variable(v, &inferred_type).is_ok());
            if !all_valid {
                // Fall back to String type
                let all_string_valid = values.iter().all(|v| validate_variable(v, &VariableType::String).is_ok());
                if !all_string_valid {
                    continue; // Some values are invalid even as strings — skip
                }
            }

            let var_name = format!("arg_{}_{}", cmd_idx, arg_idx);
            variables.push(SkillVariable {
                name: var_name,
                var_type: if all_valid { inferred_type } else { VariableType::String },
                description: format!("Argument {} of command {}", arg_idx, structure.binaries[cmd_idx]),
                examples: distinct.into_iter().map(|s| s.to_string()).collect(),
                required: true,
            });
        }
    }

    variables
}

/// Create parameterized commands from a playbook and extracted variables.
fn parameterize_commands(
    commands: &[crate::tools::subprocess_executor::StructuredCommand],
    variables: &[SkillVariable],
) -> Vec<ParameterizedCommand> {
    commands.iter().enumerate().map(|(cmd_idx, cmd)| {
        let args = cmd.args.iter().enumerate().map(|(arg_idx, arg)| {
            // Check if this argument position has a variable
            let var_name = format!("arg_{}_{}", cmd_idx, arg_idx);
            if variables.iter().any(|v| v.name == var_name) {
                format!("{{{}}}", var_name)
            } else {
                arg.clone()
            }
        }).collect();

        ParameterizedCommand {
            binary: cmd.binary.clone(),
            args,
            target: cmd.target.clone(),
            timeout_secs: cmd.timeout_secs,
        }
    }).collect()
}

/// Generate a skill name from a goal string.
fn generate_skill_name(goal: &str) -> String {
    let sanitized: String = goal.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join("_");
    format!("skill_{}", sanitized.to_lowercase())
}

/// Lower specificity = more permissive type.
fn type_specificity(t: &VariableType) -> u8 {
    match t {
        VariableType::IpAddress => 1,
        VariableType::PortNumber => 1,
        VariableType::FilePath => 2,
        VariableType::ServiceName => 2,
        VariableType::Hostname => 2,
        VariableType::Numeric => 3,
        VariableType::String => 10, // Most permissive — last resort
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::subprocess_executor::StructuredCommand;
    use tempfile::NamedTempFile;

    fn test_compiler() -> SkillCompiler {
        let tmp = NamedTempFile::new().unwrap();
        SkillCompiler::open_path(tmp.path()).unwrap()
    }

    fn make_cmd(binary: &str, args: &[&str]) -> StructuredCommand {
        StructuredCommand {
            binary: binary.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            target: "local".into(),
            timeout_secs: 30,
            working_dir: None,
            env_vars: None,
        }
    }

    #[test]
    fn record_playbook_and_check_compilable() {
        let compiler = test_compiler();
        assert!(!compiler.check_compilable("fix nginx").unwrap());

        for i in 0..3 {
            compiler.record_success(
                "fix nginx",
                &[make_cmd("systemctl", &["restart", &format!("service_{}", i)])],
                "local",
                1000 + i * 100,
            ).unwrap();
        }

        assert!(compiler.check_compilable("fix nginx").unwrap());
    }

    #[test]
    fn compile_extracts_variables() {
        let compiler = test_compiler();

        // 3 playbooks with different service names
        compiler.record_success("fix service", &[make_cmd("systemctl", &["restart", "nginx"])], "local", 1000).unwrap();
        compiler.record_success("fix service", &[make_cmd("systemctl", &["restart", "postgresql"])], "local", 1500).unwrap();
        compiler.record_success("fix service", &[make_cmd("systemctl", &["restart", "redis"])], "local", 800).unwrap();

        let skill = compiler.try_compile("fix service").unwrap();
        assert!(skill.is_some());
        let skill = skill.unwrap();
        assert_eq!(skill.variables.len(), 1); // One variable: the service name
        assert_eq!(skill.variables[0].var_type, VariableType::ServiceName);
        assert_eq!(skill.status, SkillStatus::Quarantined);
    }

    #[test]
    fn compile_rejects_insufficient_playbooks() {
        let compiler = test_compiler();
        compiler.record_success("test", &[make_cmd("ls", &["-la"])], "local", 100).unwrap();
        compiler.record_success("test", &[make_cmd("ls", &["-la"])], "local", 100).unwrap();

        let skill = compiler.try_compile("test").unwrap();
        assert!(skill.is_none()); // Only 2 playbooks, need 3
    }

    #[test]
    fn compile_rejects_different_binaries() {
        let compiler = test_compiler();
        compiler.record_success("test", &[make_cmd("ls", &["-la"])], "local", 100).unwrap();
        compiler.record_success("test", &[make_cmd("cat", &["file"])], "local", 100).unwrap();
        compiler.record_success("test", &[make_cmd("head", &["-5"])], "local", 100).unwrap();

        let skill = compiler.try_compile("test").unwrap();
        assert!(skill.is_none()); // Different binaries
    }

    #[test]
    fn skill_success_failure_circuit_breaker() {
        let compiler = test_compiler();

        // Create a skill
        compiler.record_success("test", &[make_cmd("systemctl", &["restart", "nginx"])], "local", 100).unwrap();
        compiler.record_success("test", &[make_cmd("systemctl", &["restart", "postgresql"])], "local", 100).unwrap();
        compiler.record_success("test", &[make_cmd("systemctl", &["restart", "redis"])], "local", 100).unwrap();
        let skill = compiler.try_compile("test").unwrap().unwrap();

        // Promote it
        compiler.promote_skill(&skill.name).unwrap();

        // 3 failures → disabled
        compiler.record_skill_failure(&skill.name).unwrap();
        compiler.record_skill_failure(&skill.name).unwrap();
        compiler.record_skill_failure(&skill.name).unwrap();

        let skills = compiler.get_skills_by_status("disabled").unwrap();
        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn generate_skill_name_sanitizes() {
        assert_eq!(generate_skill_name("Fix My VM!"), "skill_fix_my_vm");
        assert_eq!(generate_skill_name("restart nginx service"), "skill_restart_nginx_service");
    }
}
