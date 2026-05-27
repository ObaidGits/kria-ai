//! Browser/media workflow governance for Phase 7.
//!
//! This module is intentionally contract metadata only. It classifies browser
//! and media workflows that require visible verification or human/session
//! confirmation, but it does not navigate, play media, choose tools, verify, or
//! recover.

use crate::agent::execution_mode_reasoner::{ExecutionModeDecision, RequiredVerifier};
use crate::agent::semantic_workflow::{
    AmbiguitySeverity, AppClass, SemanticWorkflowAnalysis, TaskFamily,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMediaWorkflowKind {
    Browser,
    Media,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMediaSessionRisk {
    None,
    PublicSurface,
    AccountContextAmbiguous,
    PrivatePersonalContext,
    CredentialOrLogin,
    ExternalSideEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMediaGovernanceAction {
    None,
    RequireVisibleVerifier,
    RequireHitlPause,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserMediaGovernanceTrace {
    pub trace_labels: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserMediaGovernanceAssessment {
    pub workflow_kind: BrowserMediaWorkflowKind,
    pub session_risk: BrowserMediaSessionRisk,
    pub action: BrowserMediaGovernanceAction,
    pub requires_hitl_pause: bool,
    pub required_verifiers: Vec<RequiredVerifier>,
    pub navigation_only_completion_allowed: bool,
    pub trace: BrowserMediaGovernanceTrace,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct BrowserMediaGovernanceEvaluator;

impl BrowserMediaGovernanceEvaluator {
    pub fn assess(
        &self,
        analysis: &SemanticWorkflowAnalysis,
        decision: &ExecutionModeDecision,
        raw_user_text: &str,
    ) -> BrowserMediaGovernanceAssessment {
        let normalized = normalize_text(raw_user_text);
        let workflow_kind = classify_workflow_kind(analysis);
        let session_risk = classify_session_risk(analysis, &normalized, workflow_kind);
        let requires_hitl_pause = requires_hitl_pause(session_risk);
        let mut required_verifiers = required_verifiers_for_kind(workflow_kind);

        if matches!(
            session_risk,
            BrowserMediaSessionRisk::AccountContextAmbiguous
                | BrowserMediaSessionRisk::PrivatePersonalContext
                | BrowserMediaSessionRisk::CredentialOrLogin
                | BrowserMediaSessionRisk::ExternalSideEffect
        ) {
            push_unique(
                &mut required_verifiers,
                RequiredVerifier::BrowserAccountContext,
            );
        }
        if requires_hitl_pause {
            push_unique(
                &mut required_verifiers,
                RequiredVerifier::HumanReviewPending,
            );
            push_unique(&mut required_verifiers, RequiredVerifier::UserConfirmation);
        }

        let action = match workflow_kind {
            BrowserMediaWorkflowKind::Other => BrowserMediaGovernanceAction::None,
            BrowserMediaWorkflowKind::Browser | BrowserMediaWorkflowKind::Media
                if requires_hitl_pause =>
            {
                BrowserMediaGovernanceAction::RequireHitlPause
            }
            BrowserMediaWorkflowKind::Browser | BrowserMediaWorkflowKind::Media => {
                BrowserMediaGovernanceAction::RequireVisibleVerifier
            }
        };
        let navigation_only_completion_allowed =
            matches!(workflow_kind, BrowserMediaWorkflowKind::Other)
                || (matches!(workflow_kind, BrowserMediaWorkflowKind::Browser)
                    && matches!(session_risk, BrowserMediaSessionRisk::None)
                    && decision
                        .required_verifiers
                        .contains(&RequiredVerifier::BrowserPageVisible));

        let mut trace_labels = Vec::new();
        trace_labels.push(format!("workflow_kind::{:?}", workflow_kind));
        trace_labels.push(format!("session_risk::{:?}", session_risk));
        trace_labels.push(format!("action::{:?}", action));
        if requires_hitl_pause {
            trace_labels.push("hitl_pause_required".to_string());
        }
        if !navigation_only_completion_allowed {
            trace_labels.push("navigation_only_completion_forbidden".to_string());
        }
        for verifier in &required_verifiers {
            trace_labels.push(format!("required_verifier::{:?}", verifier));
        }

        BrowserMediaGovernanceAssessment {
            workflow_kind,
            session_risk,
            action,
            requires_hitl_pause,
            required_verifiers,
            navigation_only_completion_allowed,
            trace: BrowserMediaGovernanceTrace {
                trace_labels,
                explanation: "phase_7_browser_media_contract_metadata_only".to_string(),
            },
        }
    }
}

fn classify_workflow_kind(analysis: &SemanticWorkflowAnalysis) -> BrowserMediaWorkflowKind {
    if analysis.frame.task_family == TaskFamily::Media
        || analysis
            .frame
            .app_anchors
            .iter()
            .any(|anchor| anchor.app_class == AppClass::Media)
    {
        BrowserMediaWorkflowKind::Media
    } else if analysis.frame.task_family == TaskFamily::Browser
        || analysis
            .frame
            .app_anchors
            .iter()
            .any(|anchor| anchor.app_class == AppClass::Browser)
    {
        BrowserMediaWorkflowKind::Browser
    } else {
        BrowserMediaWorkflowKind::Other
    }
}

fn classify_session_risk(
    analysis: &SemanticWorkflowAnalysis,
    normalized: &str,
    workflow_kind: BrowserMediaWorkflowKind,
) -> BrowserMediaSessionRisk {
    if matches!(workflow_kind, BrowserMediaWorkflowKind::Other) {
        return BrowserMediaSessionRisk::None;
    }
    if contains_any(
        normalized,
        &[
            "send", "post", "share", "upload", "submit", "checkout", "purchase", "buy", "pay",
            "payment",
        ],
    ) {
        return BrowserMediaSessionRisk::ExternalSideEffect;
    }
    if contains_any(
        normalized,
        &[
            "login",
            "log in",
            "sign in",
            "signin",
            "authenticate",
            "authenticated",
            "password",
            "credential",
        ],
    ) {
        return BrowserMediaSessionRisk::CredentialOrLogin;
    }
    if contains_any(
        normalized,
        &[
            "my account",
            "my profile",
            "my playlist",
            "my playlists",
            "my liked",
            "my watch later",
            "my videos",
            "my music",
            "my photos",
            "my files",
            "personal",
            "private",
        ],
    ) {
        return BrowserMediaSessionRisk::PrivatePersonalContext;
    }
    if analysis.frame.ambiguity_level == AmbiguitySeverity::AccountSession
        || contains_any(normalized, &["account", "profile", "session"])
    {
        return BrowserMediaSessionRisk::AccountContextAmbiguous;
    }
    if matches!(
        workflow_kind,
        BrowserMediaWorkflowKind::Browser | BrowserMediaWorkflowKind::Media
    ) {
        BrowserMediaSessionRisk::PublicSurface
    } else {
        BrowserMediaSessionRisk::None
    }
}

fn required_verifiers_for_kind(workflow_kind: BrowserMediaWorkflowKind) -> Vec<RequiredVerifier> {
    match workflow_kind {
        BrowserMediaWorkflowKind::Browser => vec![RequiredVerifier::BrowserPageVisible],
        BrowserMediaWorkflowKind::Media => vec![RequiredVerifier::MediaPlaybackVisible],
        BrowserMediaWorkflowKind::Other => Vec::new(),
    }
}

fn requires_hitl_pause(session_risk: BrowserMediaSessionRisk) -> bool {
    matches!(
        session_risk,
        BrowserMediaSessionRisk::AccountContextAmbiguous
            | BrowserMediaSessionRisk::PrivatePersonalContext
            | BrowserMediaSessionRisk::CredentialOrLogin
            | BrowserMediaSessionRisk::ExternalSideEffect
    )
}

fn push_unique(verifiers: &mut Vec<RequiredVerifier>, verifier: RequiredVerifier) {
    if !verifiers.contains(&verifier) {
        verifiers.push(verifier);
    }
}

fn normalize_text(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::execution_mode_reasoner::{
        EnvironmentCapabilities, ExecutionModeReasoner, PolicyContext,
    };
    use crate::agent::intent_compiler::{ContentClass, GuiTaskSpec, TargetRef, Verb};
    use crate::agent::semantic_workflow::analyze_semantic_workflow;

    fn spec(
        primary_verb: Verb,
        targets: Vec<TargetRef>,
        content: Option<ContentClass>,
    ) -> GuiTaskSpec {
        GuiTaskSpec {
            primary_verb,
            targets,
            content,
            declared_preconditions: Vec::new(),
            declared_success_criteria: Vec::new(),
            ambiguities: Vec::new(),
        }
    }

    fn assess(prompt: &str, spec: &GuiTaskSpec) -> BrowserMediaGovernanceAssessment {
        let analysis = analyze_semantic_workflow(spec, prompt);
        let decision = ExecutionModeReasoner.decide(
            spec,
            &analysis,
            &EnvironmentCapabilities::unchecked_default(),
            &PolicyContext::default(),
        );
        BrowserMediaGovernanceEvaluator.assess(&analysis, &decision, prompt)
    }

    #[test]
    fn public_browser_workflow_requires_page_visible_but_no_hitl() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::Url("https://example.com".to_string())],
            None,
        );

        let assessment = assess("open example.com and show me the page", &spec);

        assert_eq!(assessment.workflow_kind, BrowserMediaWorkflowKind::Browser);
        assert_eq!(
            assessment.session_risk,
            BrowserMediaSessionRisk::PublicSurface
        );
        assert!(!assessment.requires_hitl_pause);
        assert!(assessment
            .required_verifiers
            .contains(&RequiredVerifier::BrowserPageVisible));
        assert!(!assessment.navigation_only_completion_allowed);
    }

    #[test]
    fn browser_upload_requires_account_context_and_hitl_pause() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("Firefox".to_string())],
            None,
        );

        let assessment = assess(
            "open browser, sign in to my account, and upload this file",
            &spec,
        );

        assert_eq!(
            assessment.session_risk,
            BrowserMediaSessionRisk::ExternalSideEffect
        );
        assert_eq!(
            assessment.action,
            BrowserMediaGovernanceAction::RequireHitlPause
        );
        assert!(assessment.requires_hitl_pause);
        assert!(assessment
            .required_verifiers
            .contains(&RequiredVerifier::BrowserAccountContext));
        assert!(assessment
            .required_verifiers
            .contains(&RequiredVerifier::HumanReviewPending));
    }

    #[test]
    fn personal_media_workflow_requires_media_visible_and_hitl_pause() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
        );

        let assessment = assess("open youtube and play my playlist", &spec);

        assert_eq!(assessment.workflow_kind, BrowserMediaWorkflowKind::Media);
        assert_eq!(
            assessment.session_risk,
            BrowserMediaSessionRisk::PrivatePersonalContext
        );
        assert!(assessment.requires_hitl_pause);
        assert!(assessment
            .required_verifiers
            .contains(&RequiredVerifier::MediaPlaybackVisible));
        assert!(assessment
            .required_verifiers
            .contains(&RequiredVerifier::BrowserAccountContext));
    }

    #[test]
    fn governance_assessment_serializes_to_json() {
        let spec = spec(
            Verb::Open,
            vec![TargetRef::App("YouTube".to_string())],
            None,
        );

        let assessment = assess("open youtube and play lo fi music", &spec);
        let json = serde_json::to_string(&assessment).expect("assessment serializes");
        let roundtrip: BrowserMediaGovernanceAssessment =
            serde_json::from_str(&json).expect("assessment deserializes");

        assert_eq!(roundtrip, assessment);
    }
}
