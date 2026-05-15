//! Early intent whitelist — `ENHANCED_STT.md` §14.
//!
//! Before **`UtteranceCommitted`** the host **MUST NOT** run LLM completion,
//! tool calls, file writes, or network I/O. Only local, reversible actions
//! from the normative whitelist are permitted.
//!
//! The v2 desktop driver keeps LLM routing **after** the pipeline has a final
//! transcript (post-commit). See `voice_runtime_helpers` where `llm` is invoked
//! only after `run_turn` has finished STT.

use thiserror::Error;

/// Stable pointer for code review / doc cross-links.
pub const POLICY_DOC_REF: &str = "ENHANCED_STT.md §14";

/// Actions that **MAY** run before `UtteranceCommitted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreCommitAction {
    StopTts,
    CancelTurn,
    MicMute,
    VolumeDown,
}

impl PreCommitAction {
    pub const NORMATIVE_SET: &'static [PreCommitAction] = &[
        PreCommitAction::StopTts,
        PreCommitAction::CancelTurn,
        PreCommitAction::MicMute,
        PreCommitAction::VolumeDown,
    ];
    
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StopTts => "StopTts",
            Self::CancelTurn => "CancelTurn",
            Self::MicMute => "MicMute",
            Self::VolumeDown => "VolumeDown",
        }
    }
}

/// Policy violation error.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PolicyViolation {
    #[error("Action '{action}' forbidden before UtteranceCommitted (§14)")]
    ForbiddenBeforeCommit { action: String },
    
    #[error("LLM generation forbidden before UtteranceCommitted (§14)")]
    LlmGenerationBlocked,
    
    #[error("Tool execution '{tool}' forbidden before UtteranceCommitted (§14)")]
    ToolExecutionBlocked { tool: String },
    
    #[error("Filesystem write forbidden before UtteranceCommitted (§14)")]
    FilesystemWriteBlocked,
    
    #[error("Network action forbidden before UtteranceCommitted (§14)")]
    NetworkActionBlocked,
}

/// Guard: enforce that only whitelisted actions run before commit.
///
/// Returns `Ok(())` if action is allowed, `Err(PolicyViolation)` otherwise.
pub fn enforce_pre_commit_action(_action: PreCommitAction) -> Result<(), PolicyViolation> {
    // All PreCommitAction variants are whitelisted by definition
    Ok(())
}

/// Guard: block LLM generation before commit.
///
/// Call this at LLM invocation sites. Returns `Err` if utterance not committed.
pub fn guard_llm_generation(utterance_committed: bool) -> Result<(), PolicyViolation> {
    if utterance_committed {
        Ok(())
    } else {
        Err(PolicyViolation::LlmGenerationBlocked)
    }
}

/// Guard: block tool execution before commit.
///
/// Call this at tool execution sites. Returns `Err` if utterance not committed.
pub fn guard_tool_execution(
    tool_name: &str,
    utterance_committed: bool,
) -> Result<(), PolicyViolation> {
    if utterance_committed {
        Ok(())
    } else {
        Err(PolicyViolation::ToolExecutionBlocked {
            tool: tool_name.to_string(),
        })
    }
}

/// Guard: block filesystem writes before commit.
///
/// Call this at file write sites. Returns `Err` if utterance not committed.
pub fn guard_filesystem_write(utterance_committed: bool) -> Result<(), PolicyViolation> {
    if utterance_committed {
        Ok(())
    } else {
        Err(PolicyViolation::FilesystemWriteBlocked)
    }
}

/// Guard: block network actions before commit.
///
/// Call this at network I/O sites. Returns `Err` if utterance not committed.
pub fn guard_network_action(utterance_committed: bool) -> Result<(), PolicyViolation> {
    if utterance_committed {
        Ok(())
    } else {
        Err(PolicyViolation::NetworkActionBlocked)
    }
}

/// Helper: check if action string is in whitelist.
///
/// Used for audit/validation. Returns `true` if action is whitelisted.
pub fn is_whitelisted_action(action: &str) -> bool {
    matches!(
        action.to_lowercase().as_str(),
        "stoptts" | "cancelturn" | "micmute" | "volumedown"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforce_pre_commit_action_allows_whitelist() {
        for action in PreCommitAction::NORMATIVE_SET {
            assert!(enforce_pre_commit_action(*action).is_ok());
        }
    }

    #[test]
    fn guard_llm_generation_blocks_before_commit() {
        let result = guard_llm_generation(false);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), PolicyViolation::LlmGenerationBlocked);
    }

    #[test]
    fn guard_llm_generation_allows_after_commit() {
        let result = guard_llm_generation(true);
        assert!(result.is_ok());
    }

    #[test]
    fn guard_tool_execution_blocks_before_commit() {
        let result = guard_tool_execution("test_tool", false);
        assert!(result.is_err());
        match result.unwrap_err() {
            PolicyViolation::ToolExecutionBlocked { tool } => {
                assert_eq!(tool, "test_tool");
            }
            _ => panic!("wrong error variant"),
        }
    }

    #[test]
    fn guard_tool_execution_allows_after_commit() {
        let result = guard_tool_execution("test_tool", true);
        assert!(result.is_ok());
    }

    #[test]
    fn guard_filesystem_write_blocks_before_commit() {
        let result = guard_filesystem_write(false);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), PolicyViolation::FilesystemWriteBlocked);
    }

    #[test]
    fn guard_filesystem_write_allows_after_commit() {
        let result = guard_filesystem_write(true);
        assert!(result.is_ok());
    }

    #[test]
    fn guard_network_action_blocks_before_commit() {
        let result = guard_network_action(false);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), PolicyViolation::NetworkActionBlocked);
    }

    #[test]
    fn guard_network_action_allows_after_commit() {
        let result = guard_network_action(true);
        assert!(result.is_ok());
    }

    #[test]
    fn is_whitelisted_action_recognizes_all_variants() {
        assert!(is_whitelisted_action("StopTts"));
        assert!(is_whitelisted_action("stoptts"));
        assert!(is_whitelisted_action("CancelTurn"));
        assert!(is_whitelisted_action("MicMute"));
        assert!(is_whitelisted_action("VolumeDown"));
        assert!(!is_whitelisted_action("LlmGeneration"));
        assert!(!is_whitelisted_action("ToolExecution"));
        assert!(!is_whitelisted_action("FileWrite"));
    }

    #[test]
    fn pre_commit_action_as_str() {
        assert_eq!(PreCommitAction::StopTts.as_str(), "StopTts");
        assert_eq!(PreCommitAction::CancelTurn.as_str(), "CancelTurn");
        assert_eq!(PreCommitAction::MicMute.as_str(), "MicMute");
        assert_eq!(PreCommitAction::VolumeDown.as_str(), "VolumeDown");
    }

    #[test]
    fn normative_set_contains_all_variants() {
        assert_eq!(PreCommitAction::NORMATIVE_SET.len(), 4);
        assert!(PreCommitAction::NORMATIVE_SET.contains(&PreCommitAction::StopTts));
        assert!(PreCommitAction::NORMATIVE_SET.contains(&PreCommitAction::CancelTurn));
        assert!(PreCommitAction::NORMATIVE_SET.contains(&PreCommitAction::MicMute));
        assert!(PreCommitAction::NORMATIVE_SET.contains(&PreCommitAction::VolumeDown));
    }
}
