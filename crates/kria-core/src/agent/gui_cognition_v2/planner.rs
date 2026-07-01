//! GUI Cognition V2 — turn-level task PLANNER (spec Task 9, Requirement 4/17).
//!
//! The planner decomposes ONE natural-language desktop task into an ordered list
//! of verifiable [`SubGoal`]s using the configured LLM (model-neutral, behind the
//! same [`LlmBackend`] seam the Brain uses). This replaces the order-sensitive,
//! verb-gated keyword heuristics as the PRIMARY decomposition path; the
//! deterministic [`fallback_plan`] remains as an honest fallback when the model
//! is unavailable OR has not cleared the offline quality gate (Requirement 17.4).
//!
//! Hardening:
//! - The model input is the USER TASK ONLY; no screen-derived text is fed to the
//!   planner (injection hardening — Requirement 17.5).
//! - Output is grammar/schema-constrained and validated by a pure function
//!   ([`parse_plan_json`]) so it is fully unit-testable without a live model.
//! - Decomposition is bounded (sub-goal cap) and never loops.

use std::sync::Arc;
use std::time::Duration;

use crate::llm::{ChatMessage, LlmBackend};

use super::types::{SubGoal, SubGoalKind};

/// The canonical planner system prompt and JSON schema, shared verbatim with the
/// offline scorer (`testing/tools/gui_cog_planner_eval.py`) via `include_str!`
/// so the gate measures the SAME prompt the runtime uses (no drift).
pub const PLANNER_SYSTEM_PROMPT: &str = include_str!("planner_prompt.txt");
const PLANNER_SCHEMA_JSON: &str = include_str!("planner_schema.json");

const PLANNER_MAX_TOKENS: u32 = 768;
const PLANNER_TIMEOUT_MS: u64 = 45_000;
/// Hard cap on sub-goals per plan (matches the schema `maxItems`).
pub const MAX_SUB_GOALS: usize = 12;

/// An ordered, verifiable decomposition of a task.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub sub_goals: Vec<SubGoal>,
}

impl Plan {
    pub fn new(sub_goals: Vec<SubGoal>) -> Self {
        Self { sub_goals }
    }

    pub fn is_empty(&self) -> bool {
        self.sub_goals.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sub_goals.len()
    }

    /// The ordered list of sub-goal kinds (used by the offline accuracy gate).
    pub fn kinds(&self) -> Vec<SubGoalKind> {
        self.sub_goals.iter().map(|s| s.kind).collect()
    }
}

/// The grammar schema for one plan, parsed once from the shared JSON file.
pub fn plan_schema() -> serde_json::Value {
    serde_json::from_str(PLANNER_SCHEMA_JSON).expect("planner_schema.json is valid JSON")
}

/// Map a kind string (snake_case) to [`SubGoalKind`]. Unknown → `Other`.
fn kind_from_str(s: &str) -> SubGoalKind {
    match s.trim().to_ascii_lowercase().as_str() {
        "open_app" => SubGoalKind::OpenApp,
        "click" => SubGoalKind::Click,
        "type" => SubGoalKind::Type,
        "navigate" => SubGoalKind::Navigate,
        "run_command" => SubGoalKind::RunCommand,
        "write_file" => SubGoalKind::WriteFile,
        "read_output" => SubGoalKind::ReadOutput,
        "verify" => SubGoalKind::Verify,
        _ => SubGoalKind::Other,
    }
}

/// Extract the first balanced top-level JSON object from model output.
fn extract_json_object(content: &str) -> Option<&str> {
    let bytes = content.as_bytes();
    let mut depth = 0i32;
    let mut start = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        return Some(&content[s..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse + validate a planner response into a [`Plan`] (pure, testable).
///
/// Enforces: at least one sub-goal, empty/blank intents dropped, kinds mapped,
/// the sub-goal cap, and that `target_hint`/`expect_contains` are non-empty when
/// present. Returns an error when no usable sub-goal can be parsed (the caller
/// then falls back deterministically).
pub fn parse_plan_json(content: &str) -> anyhow::Result<Plan> {
    let json = extract_json_object(content)
        .ok_or_else(|| anyhow::anyhow!("no JSON object in planner output"))?;
    let v: serde_json::Value = serde_json::from_str(json)?;
    let arr = v
        .get("sub_goals")
        .and_then(|s| s.as_array())
        .ok_or_else(|| anyhow::anyhow!("planner output missing 'sub_goals' array"))?;

    let mut sub_goals = Vec::new();
    for item in arr {
        let intent = item
            .get("intent")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let kind_str = item.get("kind").and_then(|x| x.as_str()).unwrap_or("");
        if intent.is_empty() || kind_str.is_empty() {
            continue;
        }
        let mut sg = SubGoal::new(intent, kind_from_str(kind_str));
        if let Some(t) = item.get("target_hint").and_then(|x| x.as_str()) {
            let t = t.trim();
            if !t.is_empty() {
                sg = sg.with_target(t.to_string());
            }
        }
        if let Some(e) = item.get("expect_contains").and_then(|x| x.as_str()) {
            let e = e.trim();
            if !e.is_empty() {
                sg = sg.expecting(e.to_string());
            }
        }
        sub_goals.push(sg);
        if sub_goals.len() >= MAX_SUB_GOALS {
            break;
        }
    }

    if sub_goals.is_empty() {
        anyhow::bail!("planner produced no usable sub-goals");
    }
    Ok(Plan::new(sub_goals))
}

/// Build the planner chat messages. The USER TASK is the only content
/// (injection hardening — Requirement 17.5).
pub fn build_planner_messages(task: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "system".into(),
            content: PLANNER_SYSTEM_PROMPT.into(),
            name: None,
            images: None,
        },
        ChatMessage {
            role: "user".into(),
            content: format!("Task: {}", task.trim()),
            name: None,
            images: None,
        },
    ]
}

/// LLM-backed planner. Model-neutral: any configured [`LlmBackend`].
pub struct LlmPlanner {
    backend: Arc<dyn LlmBackend>,
    timeout: Duration,
    max_tokens: u32,
}

impl LlmPlanner {
    pub fn new(backend: Arc<dyn LlmBackend>) -> Self {
        Self {
            backend,
            timeout: Duration::from_millis(PLANNER_TIMEOUT_MS),
            max_tokens: PLANNER_MAX_TOKENS,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Decompose a task into a [`Plan`]. On model-unavailable / transport / parse
    /// failure, returns the deterministic [`fallback_plan`] (honest, never empty)
    /// so a turn can always proceed (Requirement 17.4 / 24.2).
    pub async fn decompose(&self, task: &str) -> Plan {
        match self.try_decompose(task).await {
            Ok(plan) => normalize_plan(task, plan),
            Err(e) => {
                tracing::warn!(
                    target: "gui_cognition_v2",
                    error = %e,
                    "planner LLM decompose failed; using deterministic fallback plan"
                );
                fallback_plan(task)
            }
        }
    }

    /// The raw LLM decomposition (bounded retry on transport/timeout). Returns an
    /// error if no plan could be produced; callers map that to the fallback.
    pub async fn try_decompose(&self, task: &str) -> anyhow::Result<Plan> {
        if !self.backend.is_configured() {
            anyhow::bail!("planner backend unconfigured");
        }
        let messages = build_planner_messages(task);
        let schema = plan_schema();
        const MAX_ATTEMPTS: u8 = 3;
        let mut last_err: Option<String> = None;
        for attempt in 0..MAX_ATTEMPTS {
            let fut =
                self.backend
                    .chat_with_grammar(&messages, schema.clone(), 0.1, self.max_tokens);
            match tokio::time::timeout(self.timeout, fut).await {
                Ok(Ok(resp)) => match parse_plan_json(&resp.content) {
                    Ok(plan) => return Ok(plan),
                    Err(e) => last_err = Some(format!("planner parse error: {e}")),
                },
                Ok(Err(e)) => last_err = Some(format!("planner provider error: {e}")),
                Err(_) => last_err = Some("planner timed out".into()),
            }
            if attempt + 1 < MAX_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
        anyhow::bail!(last_err.unwrap_or_else(|| "planner failed".into()))
    }
}

#[async_trait::async_trait]
impl super::traits::GuiPlanner for LlmPlanner {
    async fn plan(&self, task: &str) -> Plan {
        self.decompose(task).await
    }
}

/// Known file extensions used to detect an explicit filename in a task.
const FILE_EXTS: &[&str] = &[
    "txt", "py", "sh", "md", "json", "csv", "js", "ts", "html", "css", "rs", "c", "cpp", "java",
    "go", "rb", "sql", "yaml", "yml", "toml", "xml", "ini", "cfg", "log",
];

/// Extract an explicit `name.ext` filename mentioned in the task (first match).
pub(crate) fn extract_filename(task: &str) -> Option<String> {
    for raw in task.split_whitespace() {
        let tok =
            raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_' && c != '-');
        if let Some(dot) = tok.rfind('.') {
            let ext = &tok[dot + 1..];
            if !ext.is_empty() && FILE_EXTS.contains(&ext.to_ascii_lowercase().as_str()) && dot > 0
            {
                return Some(tok.to_string());
            }
        }
    }
    None
}

/// Extract the literal content requested after "with the text/content ..." for a
/// file-creation task (so `create hello.txt with the text Hello KRIA` writes the
/// right body). Returns the trailing phrase, trimmed of a closing "and show…".
fn extract_inline_content(task: &str) -> Option<String> {
    let lower = task.to_ascii_lowercase();
    for marker in [
        "with the text ",
        "with the content ",
        "containing ",
        "that says ",
        "with text ",
    ] {
        if let Some(idx) = lower.find(marker) {
            let rest = &task[idx + marker.len()..];
            let mut out: Vec<&str> = Vec::new();
            for word in rest.split_whitespace() {
                let wl = word
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_ascii_lowercase();
                if matches!(wl.as_str(), "then" | "and" | "after" | "before") {
                    break;
                }
                out.push(word);
            }
            let cut = out.join(" ").trim().trim_end_matches('.').to_string();
            if !cut.trim().is_empty() {
                return Some(cut.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Whether the task is clearly asking to CREATE/WRITE a file or program.
fn task_writes_a_file(task: &str) -> bool {
    let l = task.to_ascii_lowercase();
    (l.contains("write ") || l.contains("create ") || l.contains("make "))
        && (l.contains("file")
            || l.contains("script")
            || l.contains("program")
            || extract_filename(task).is_some())
}

/// Normalize a plan so an explicit file-creation intent ALWAYS routes through a
/// `write_file` sub-goal (which the cross-substrate bridge fills with real,
/// generated content) instead of fragile editor-typing. Deterministic, applied
/// to both the LLM and fallback plans so code/file tasks reliably reach the
/// bridge regardless of how the model decomposed them.
pub(crate) fn normalize_plan(task: &str, mut plan: Plan) -> Plan {
    if !task_writes_a_file(task) {
        return plan;
    }
    // Determine the file path + optional inline content from the task, reusing an
    // existing write_file sub-goal's target if the model already chose one.
    let existing_wf = plan
        .sub_goals
        .iter()
        .find(|s| s.kind == SubGoalKind::WriteFile)
        .and_then(|s| s.target_hint.clone());
    let fname = extract_filename(task)
        .or(existing_wf)
        .unwrap_or_else(|| infer_filename(task));
    let mut wf = SubGoal::new(format!("write {fname}"), SubGoalKind::WriteFile).with_target(fname);
    if let Some(content) = extract_inline_content(task) {
        wf = wf.expecting(content);
    }
    // Drop the fragile editor-driven path: editor-typing (Type), in-app clicks,
    // AND opening a text EDITOR (nano/vim/gedit/VS Code/…). The bridge writes the
    // file directly, so an editor is unnecessary and a wrong/absent editor (e.g.
    // nano not installed) must not derail the task. Non-editor opens (e.g. a
    // terminal to run the script) and run/read sub-goals are kept.
    plan.sub_goals.retain(|s| {
        !matches!(
            s.kind,
            SubGoalKind::Type | SubGoalKind::Click | SubGoalKind::WriteFile
        ) && !(s.kind == SubGoalKind::OpenApp && target_is_text_editor(s.target_hint.as_deref()))
    });
    // Write the file FIRST (via the bridge) so it exists for any later run/show
    // step regardless of whether some optional app opens.
    plan.sub_goals.insert(0, wf);
    plan.sub_goals.truncate(MAX_SUB_GOALS);
    plan
}

/// Whether an OpenApp target names a text EDITOR (so a file-write task can skip
/// opening it — the bridge writes the file). Matches common editors generically.
fn target_is_text_editor(target: Option<&str>) -> bool {
    let t = match target {
        Some(t) => t.to_ascii_lowercase(),
        None => return false,
    };
    [
        "nano",
        "vim",
        "vi ",
        "emacs",
        "gedit",
        "kate",
        "sublime",
        "text editor",
        "texteditor",
        "gnome-text-editor",
        "code",
        "vscode",
        "visual studio code",
        "notepad",
        "mousepad",
        "leafpad",
    ]
    .iter()
    .any(|e| t == *e || t.contains(e))
}

/// Infer a sensible filename from the task's language hint when none is explicit.
fn infer_filename(task: &str) -> String {
    let l = task.to_ascii_lowercase();
    let ext = if l.contains("python") || l.contains(" py") {
        "py"
    } else if l.contains("bash") || l.contains("shell") || l.contains(".sh") {
        "sh"
    } else if l.contains("javascript") || l.contains("node") {
        "js"
    } else if l.contains("html") {
        "html"
    } else if l.contains("program") || l.contains("script") || l.contains("code") {
        // A generic "write a program/script" with no language is, in this
        // assistant's context, overwhelmingly Python.
        "py"
    } else {
        "txt"
    };
    format!("kria_script.{ext}")
}

/// Deterministic fallback decomposition (no model). Uses the same universal,
/// app-agnostic primitives the Brain assists use (open-app / navigate / command
/// / calc / follow-up) to build a sensible ordered plan. It is intentionally
/// conservative: it covers the common shapes and otherwise emits a single
/// best-effort sub-goal so the turn can still proceed. NEVER empty.
pub fn fallback_plan(task: &str) -> Plan {
    use super::llm_brain::{
        task_calc_expression, task_command_target, task_followup_action, task_navigation_target,
        task_open_app_target,
    };

    let mut sub_goals: Vec<SubGoal> = Vec::new();

    // 1) Open the named app first (if any).
    if let Some(app) = task_open_app_target(task) {
        sub_goals.push(SubGoal::new(format!("open {app}"), SubGoalKind::OpenApp).with_target(app));
    }

    // 2) A web navigation / search target.
    if let Some(target) = task_navigation_target(task) {
        sub_goals.push(
            SubGoal::new(format!("navigate to {target}"), SubGoalKind::Navigate)
                .with_target(target),
        );
    }

    // 3) A shell command to run.
    if let Some(cmd) = task_command_target(task) {
        sub_goals
            .push(SubGoal::new(format!("run {cmd}"), SubGoalKind::RunCommand).with_target(cmd));
    }

    // 4) A calculator expression (type-and-submit "A op B=").
    if let Some(expr) = task_calc_expression(task) {
        let result = eval_simple_expr(&expr);
        let mut sg = SubGoal::new(format!("compute {expr}"), SubGoalKind::Type).with_target(expr);
        if let Some(r) = result {
            sg = sg.expecting(r);
        }
        sub_goals.push(sg);
    }

    // 5) A standard keyboard follow-up (new tab / close tab / reload / ...).
    if let Some(combo) = task_followup_action(task) {
        sub_goals
            .push(SubGoal::new(format!("perform {combo}"), SubGoalKind::Click).with_target(combo));
    }

    // Nothing matched → a single best-effort "other" sub-goal carrying the task,
    // so the loop still runs (and the Brain/grounding can act).
    if sub_goals.is_empty() {
        sub_goals.push(SubGoal::new(task.trim().to_string(), SubGoalKind::Other));
    }

    sub_goals.truncate(MAX_SUB_GOALS);
    normalize_plan(task, Plan::new(sub_goals))
}

/// Evaluate a trivial "A op B=" expression to its integer/decimal string result
/// (used only to populate `expect_contains` for the calculator fallback). Returns
/// `None` for anything non-trivial.
fn eval_simple_expr(expr: &str) -> Option<String> {
    let e = expr.trim().trim_end_matches('=');
    for (sym, _) in [('*', 0), ('+', 0), ('-', 0), ('/', 0)] {
        if let Some(idx) = e.find(sym) {
            let a: f64 = e[..idx].trim().parse().ok()?;
            let b: f64 = e[idx + 1..].trim().parse().ok()?;
            let r = match sym {
                '*' => a * b,
                '+' => a + b,
                '-' => a - b,
                '/' => {
                    if b == 0.0 {
                        return None;
                    }
                    a / b
                }
                _ => return None,
            };
            // Integer-valued results print without a trailing ".0".
            if (r - r.round()).abs() < f64::EPSILON {
                return Some(format!("{}", r.round() as i64));
            }
            return Some(format!("{r}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_valid_and_constrains_kind() {
        let s = plan_schema();
        assert_eq!(s["type"], "object");
        let enum_vals = &s["properties"]["sub_goals"]["items"]["properties"]["kind"]["enum"];
        assert!(enum_vals
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "open_app"));
    }

    #[test]
    fn parses_a_well_formed_plan() {
        let json = r#"{"sub_goals":[
            {"intent":"open Chrome","kind":"open_app","target_hint":"chrome"},
            {"intent":"go to youtube","kind":"navigate","target_hint":"youtube.com"}
        ]}"#;
        let plan = parse_plan_json(json).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(
            plan.kinds(),
            vec![SubGoalKind::OpenApp, SubGoalKind::Navigate]
        );
        assert_eq!(plan.sub_goals[0].target_hint.as_deref(), Some("chrome"));
    }

    #[test]
    fn drops_blank_subgoals_and_maps_unknown_kind_to_other() {
        let json = r#"{"sub_goals":[
            {"intent":"","kind":"open_app"},
            {"intent":"do thing","kind":"frobnicate"}
        ]}"#;
        let plan = parse_plan_json(json).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.kinds(), vec![SubGoalKind::Other]);
    }

    #[test]
    fn tolerates_prose_around_json() {
        let content = "Sure! Here is the plan:\n{\"sub_goals\":[{\"intent\":\"open it\",\"kind\":\"open_app\",\"target_hint\":\"settings\"}]}\nDone.";
        let plan = parse_plan_json(content).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.sub_goals[0].kind, SubGoalKind::OpenApp);
    }

    #[test]
    fn empty_plan_is_an_error() {
        assert!(parse_plan_json(r#"{"sub_goals":[]}"#).is_err());
        assert!(parse_plan_json("not json").is_err());
        assert!(parse_plan_json(r#"{"x":1}"#).is_err());
    }

    #[test]
    fn caps_sub_goals_at_max() {
        let items: Vec<String> = (0..30)
            .map(|i| format!("{{\"intent\":\"step {i}\",\"kind\":\"click\"}}"))
            .collect();
        let json = format!("{{\"sub_goals\":[{}]}}", items.join(","));
        let plan = parse_plan_json(&json).unwrap();
        assert_eq!(plan.len(), MAX_SUB_GOALS);
    }

    #[test]
    fn fallback_open_and_navigate_in_order() {
        let plan = fallback_plan("open chrome and go to youtube.com");
        assert_eq!(plan.kinds()[0], SubGoalKind::OpenApp);
        assert!(plan.kinds().contains(&SubGoalKind::Navigate));
        // OpenApp precedes Navigate.
        let oi = plan
            .kinds()
            .iter()
            .position(|k| *k == SubGoalKind::OpenApp)
            .unwrap();
        let ni = plan
            .kinds()
            .iter()
            .position(|k| *k == SubGoalKind::Navigate)
            .unwrap();
        assert!(oi < ni);
    }

    #[test]
    fn fallback_command_and_calc() {
        let p1 = fallback_plan("open terminal and run ls");
        assert!(p1.kinds().contains(&SubGoalKind::RunCommand));
        let p2 = fallback_plan("open the calculator and compute 256 times 13");
        // The calc sub-goal carries the expected result for the verifier.
        let calc = p2
            .sub_goals
            .iter()
            .find(|s| s.kind == SubGoalKind::Type)
            .unwrap();
        assert_eq!(calc.expect_contains.as_deref(), Some("3328"));
        assert_eq!(calc.target_hint.as_deref(), Some("256*13="));
    }

    #[test]
    fn fallback_never_empty() {
        let p = fallback_plan("do something vague and unmatched");
        assert_eq!(p.len(), 1);
        assert_eq!(p.sub_goals[0].kind, SubGoalKind::Other);
    }

    #[test]
    fn eval_simple_expr_handles_basic_ops() {
        assert_eq!(eval_simple_expr("256*13="), Some("3328".into()));
        assert_eq!(eval_simple_expr("10+5="), Some("15".into()));
        assert_eq!(eval_simple_expr("9/2="), Some("4.5".into()));
        assert_eq!(eval_simple_expr("9/0="), None);
    }

    #[test]
    fn extract_filename_finds_explicit_names() {
        assert_eq!(
            extract_filename("create a file hello.txt with text").as_deref(),
            Some("hello.txt")
        );
        assert_eq!(
            extract_filename("run fib.py please").as_deref(),
            Some("fib.py")
        );
        assert_eq!(extract_filename("open the calculator").as_deref(), None);
    }

    #[test]
    fn normalize_routes_file_creation_to_write_file() {
        // A model plan that used editor-typing is normalized to a write_file the
        // bridge can fill with generated content.
        let typed = Plan::new(vec![
            SubGoal::new("open editor", SubGoalKind::OpenApp).with_target("text editor"),
            SubGoal::new("type the text", SubGoalKind::Type).with_target("Hello KRIA"),
        ]);
        let norm = normalize_plan("Create a file hello.txt with the text Hello KRIA", typed);
        let kinds = norm.kinds();
        assert!(kinds.contains(&SubGoalKind::WriteFile));
        assert!(!kinds.contains(&SubGoalKind::Type), "editor-typing dropped");
        let wf = norm
            .sub_goals
            .iter()
            .find(|s| s.kind == SubGoalKind::WriteFile)
            .unwrap();
        assert_eq!(wf.target_hint.as_deref(), Some("hello.txt"));
        assert_eq!(wf.expect_contains.as_deref(), Some("Hello KRIA"));
    }

    #[test]
    fn normalize_infers_filename_for_scripts() {
        let p = normalize_plan(
            "write a python script that prints fibonacci",
            Plan::new(vec![SubGoal::new("type code", SubGoalKind::Type)]),
        );
        let wf = p
            .sub_goals
            .iter()
            .find(|s| s.kind == SubGoalKind::WriteFile)
            .unwrap();
        assert!(wf.target_hint.as_deref().unwrap().ends_with(".py"));
    }

    #[test]
    fn normalize_leaves_non_file_tasks_untouched() {
        let p = Plan::new(vec![SubGoal::new("open chrome", SubGoalKind::OpenApp)]);
        let norm = normalize_plan("open chrome and go to youtube", p.clone());
        assert_eq!(norm, p);
    }
}
