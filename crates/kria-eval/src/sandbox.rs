#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvalConfig {
    pub eval_mode: bool,
}

impl EvalConfig {
    pub fn from_env() -> Self {
        Self {
            eval_mode: is_eval_mode_enabled(),
        }
    }
}

pub fn is_eval_mode_enabled() -> bool {
    matches!(std::env::var("KRIA_EVAL_MODE").as_deref(), Ok("1"))
}
