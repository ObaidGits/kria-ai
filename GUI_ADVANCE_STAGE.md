# KRIA GUI Cognition — Advanced Production Stage

**Date:** 2026-05-28
**Status:** Implementation Specification
**Goal:** Transform KRIA from a tool dispatcher into an **intelligent collaborative desktop assistant** with smart failure recovery and actionable HITL.

---

## The Core Vision

When a user says **"Text Faizan on WhatsApp 'hello'"** and KRIA cannot proceed, the user must NEVER see a dead-end error. They must see a **smart recovery panel** with concrete, clickable actions that resolve the problem:

```text
┌─────────────────────────────────────────────────────────────┐
│  🤖 I can't text Faizan on WhatsApp yet                     │
│                                                              │
│  WhatsApp isn't installed and I can't find a logged-in       │
│  browser session for WhatsApp Web.                           │
│                                                              │
│  Here's what I can do:                                       │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ 📦 Install WhatsApp Desktop                          │   │
│  │    flatpak install flathub com.rtosta.zapzap         │   │
│  │    [Install Now] [Show Command]                      │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ 🌐 Open WhatsApp Web in Chrome                       │   │
│  │    Will navigate to web.whatsapp.com — you'll need   │   │
│  │    to scan the QR code with your phone.              │   │
│  │    [Open in Chrome]                                  │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ 🌐 Open WhatsApp Web in Brave                        │   │
│  │    [Open in Brave]                                   │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ ✉️ Send via SMS instead                              │   │
│  │    [Use Default SMS App]                             │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                              │
│  [Skip This Task]    [Cancel Workflow]                      │
└─────────────────────────────────────────────────────────────┘
```

After the user clicks any action button, KRIA executes that action and **automatically resumes the original workflow** when the precondition is satisfied.

This document is the implementation blueprint for that system.

---

## Table of Contents

1. [Architectural Vision](#1-architectural-vision)
2. [The Recovery Substrate](#2-the-recovery-substrate)
3. [Failure Cognition Engine](#3-failure-cognition-engine)
4. [Action Suggestion System](#4-action-suggestion-system)
5. [Workflow Resumption Protocol](#5-workflow-resumption-protocol)
6. [UI Components Specification](#6-ui-components-specification)
7. [Backend Architecture](#7-backend-architecture)
8. [Knowledge Base for Smart Suggestions](#8-knowledge-base-for-smart-suggestions)
9. [Multi-Step Recovery Workflows](#9-multi-step-recovery-workflows)
10. [Implementation Phases](#10-implementation-phases)
11. [Testing Strategy](#11-testing-strategy)
12. [Production Examples](#12-production-examples)

---

## 1. Architectural Vision

### What Makes This System "Intelligent"

A normal automation tool fails. An intelligent assistant **understands why it failed and offers paths forward.**

The intelligence has 4 layers:

| Layer | Purpose | Example |
|-------|---------|---------|
| **Failure Diagnosis** | Why did this fail? | "WhatsApp app missing, no browser session" |
| **Goal Decomposition** | What does the user actually want? | "Send 'hello' to contact 'Faizan'" |
| **Capability Reasoning** | What can I do here? | "Brave is installed, Chrome is installed, can open URLs" |
| **Recovery Synthesis** | What concrete action sequence resolves this? | "Open Chrome → navigate to web.whatsapp.com → wait for QR scan → resume original message workflow" |

### Core Design Principles

1. **Every failure ends with a button, not an error string.** Users never see "Step 2 failed" with no recourse.
2. **Buttons execute concrete actions.** Clicking "Install WhatsApp" actually runs the install command.
3. **Original workflow is preserved.** When recovery succeeds, the original goal resumes automatically.
4. **Multiple alternatives are always offered.** Never a single forced path.
5. **User preferences are remembered.** "Always use Brave for WhatsApp" → never asks again.
6. **The system asks only what it cannot determine.** Don't ask "which browser?" if the user always picks Brave.

### What This is NOT

- Not a chatbot that explains what went wrong
- Not a generic "retry/skip/cancel" prompt
- Not a wizard with hardcoded flows
- Not a configuration UI

It IS: **a context-aware action proposer that bridges specific failures to specific recoveries.**

---

## 2. The Recovery Substrate

### Recovery as a First-Class Workflow Type

Today, KRIA has substrates: `FileWriteThenOpen`, `BrowserNavigate`, `Keystroke`, etc. We add a new substrate:

```rust
pub enum ExecutionSubstrate {
    // ... existing variants ...

    /// Recovery substrate — generates and executes recovery workflows
    /// when a primary workflow encounters a blocker.
    Recovery {
        /// The original workflow that failed
        parent_workflow_id: String,
        /// What blocked the parent
        blocker: WorkflowBlocker,
        /// Recovery action being executed
        action: RecoveryAction,
    },
}
```

### Recovery Action Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryAction {
    /// Install a missing application
    InstallApp {
        app_id: String,
        package_manager: PackageManager,
        package_name: String,
        post_install_step: Option<Box<RecoveryAction>>,
    },

    /// Open an alternative URL/app
    OpenAlternative {
        target: AlternativeTarget,
        post_open_step: Option<Box<RecoveryAction>>,
    },

    /// Initiate a login flow
    LoginFlow {
        service: String,
        login_url: String,
        login_method: LoginMethod,
        verification: SessionVerification,
    },

    /// Switch to a different substrate
    SwitchSubstrate {
        from: ExecutionSubstrate,
        to: ExecutionSubstrate,
        reason: String,
    },

    /// Run a shell command (with safety gating)
    RunCommand {
        command: String,
        safety_classification: RiskLevel,
        expected_outcome: ExpectedOutcome,
    },

    /// Composite — run multiple recovery actions in sequence
    Sequence {
        steps: Vec<RecoveryAction>,
        continue_on_failure: bool,
    },

    /// Manual step — user does it themselves, KRIA waits
    ManualStep {
        instruction: String,
        completion_signal: CompletionSignal,
    },

    /// Switch to an alternative communication channel
    SwitchCommunicationChannel {
        original_service: String,
        alternative_service: String,
        message_to_send: String,
    },

    /// Skip the failed step and continue with a fallback
    SkipWithFallback {
        skip_step: u32,
        fallback_value: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PackageManager {
    Apt,
    Flatpak,
    Snap,
    Pacman,
    Dnf,
    Brew,
    Pkg,
    Custom { install_command_template: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlternativeTarget {
    /// Open a URL in a specific browser
    UrlInBrowser { url: String, browser: String },
    /// Open in any available browser (default)
    UrlInDefaultBrowser { url: String },
    /// Launch an alternative app
    AlternativeApp { app_id: String, file_arg: Option<String> },
    /// Switch to native app for the same service
    NativeApp { service: String },
    /// Switch to web version of the same service
    WebApp { service: String, url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoginMethod {
    /// QR code scan (WhatsApp Web, Telegram Web)
    QrCodeScan { wait_timeout_secs: u64 },
    /// Username/password form
    CredentialForm { fields: Vec<String> },
    /// OAuth/SSO flow
    OAuth { provider: String },
    /// Email magic link
    MagicLink { email_hint: Option<String> },
    /// Manual (user logs in however they prefer)
    Manual { instruction: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompletionSignal {
    /// Wait for a process to appear
    ProcessAppears { binary: String, max_wait_secs: u64 },
    /// Wait for a file to exist
    FileAppears { path: PathBuf, max_wait_secs: u64 },
    /// Wait for a URL to be reachable
    UrlReachable { url: String, max_wait_secs: u64 },
    /// Wait for an AT-SPI element to appear
    AccessibilityElement { role: String, name: String, max_wait_secs: u64 },
    /// Wait for a port to be listening
    PortListening { port: u16, max_wait_secs: u64 },
    /// Wait for the user to click "Done"
    UserConfirmation { question: String },
}
```

---

## 3. Failure Cognition Engine

### How KRIA Diagnoses Why a Workflow Failed

When a step fails, the executor invokes the **FailureCognitionEngine**:

```rust
pub struct FailureCognitionEngine {
    knowledge_base: Arc<RecoveryKnowledgeBase>,
    capability_set: CapabilitySet,
    user_preferences: Arc<UserPreferenceStore>,
    history: Arc<RecoveryHistoryStore>,
}

impl FailureCognitionEngine {
    /// Diagnose why a workflow step failed and synthesize recovery options.
    pub async fn diagnose(
        &self,
        failed_step: &HybridStep,
        error: &StepError,
        workflow_context: &WorkflowMemory,
        original_intent: &WorkflowIntent,
    ) -> RecoveryDiagnosis {
        // Step 1: Classify the error
        let blocker = self.classify_blocker(failed_step, error, workflow_context);

        // Step 2: Identify what the user was trying to achieve at the SEMANTIC level
        // (not the tool level — "send message" not "type_text into web.whatsapp.com")
        let semantic_goal = self.extract_semantic_goal(original_intent, workflow_context);

        // Step 3: Query the knowledge base for known recovery strategies
        let candidates = self.knowledge_base.suggest_recoveries(
            &blocker,
            &semantic_goal,
            &self.capability_set,
        );

        // Step 4: Filter by user preferences (already-rejected options excluded)
        let filtered = self.apply_user_preferences(candidates);

        // Step 5: Rank by likelihood of success
        let ranked = self.rank_recoveries(filtered, workflow_context).await;

        // Step 6: Build human-readable explanations
        RecoveryDiagnosis {
            blocker,
            semantic_goal,
            explanation: self.build_explanation(&blocker, &semantic_goal),
            recovery_options: ranked,
            allow_skip: self.is_skippable(failed_step),
            allow_cancel: true,
        }
    }
}
```

### Blocker Classification Taxonomy

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowBlocker {
    /// The required app isn't installed
    AppNotInstalled {
        app_id: String,
        install_alternatives: Vec<InstallOption>,
        web_alternatives: Vec<WebAlternative>,
    },

    /// User isn't logged into the required service
    NotLoggedIn {
        service: String,
        login_methods: Vec<LoginMethod>,
        login_urls_per_browser: Vec<(String, String)>,  // (browser, url)
    },

    /// Session expired
    SessionExpired {
        service: String,
        re_login_required: bool,
    },

    /// App is installed but not running
    AppNotRunning {
        app_id: String,
        launch_method: LaunchMethod,
    },

    /// Network issue prevented action
    NetworkUnavailable {
        affected_service: String,
        suggestion: String,
    },

    /// File or resource not found
    ResourceNotFound {
        resource_type: ResourceType,
        identifier: String,
        suggestions: Vec<String>,
    },

    /// Permission denied
    PermissionDenied {
        action: String,
        elevation_method: Option<ElevationMethod>,
    },

    /// uinput daemon not running
    UinputUnavailable {
        manual_alternative: Option<RecoveryAction>,
    },

    /// AT-SPI not available
    AtspiUnavailable {
        impact: String,
        manual_alternative: Option<RecoveryAction>,
    },

    /// Window not focused / focus stolen
    FocusLost {
        target_window: String,
        manual_intervention: bool,
    },

    /// Ambiguous target (multiple matches)
    AmbiguousTarget {
        candidates: Vec<TargetCandidate>,
        question: String,
    },

    /// Disk space, memory, or resource constraint
    ResourceConstraint {
        resource: String,
        required: u64,
        available: u64,
        cleanup_suggestion: Option<String>,
    },

    /// Cloud service rate limit / quota
    QuotaExceeded {
        service: String,
        retry_after_secs: Option<u64>,
        alternative_service: Option<String>,
    },

    /// Generic timeout
    Timeout {
        action: String,
        elapsed_ms: u64,
        suggested_action: String,
    },

    /// Generic — fallback when no classification matches
    Unknown {
        raw_error: String,
        retry_safe: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOption {
    pub package_manager: PackageManager,
    pub package_name: String,
    pub estimated_size_mb: u32,
    pub install_command: String,
    pub requires_sudo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebAlternative {
    pub service: String,
    pub url: String,
    pub requires_login: bool,
    pub supports_browsers: Vec<String>,  // ["chrome", "brave", "firefox"]
}
```

### Error → Blocker Classification Logic

```rust
impl FailureCognitionEngine {
    fn classify_blocker(
        &self,
        step: &HybridStep,
        error: &StepError,
        ctx: &WorkflowMemory,
    ) -> WorkflowBlocker {
        let error_msg = error.message.to_lowercase();

        // App not found
        if error_msg.contains("not found in the installed app registry")
            || error_msg.contains("application not found")
            || error_msg.contains("command not found") {
            let app_name = self.extract_app_name(step, error);
            return WorkflowBlocker::AppNotInstalled {
                app_id: app_name.clone(),
                install_alternatives: self.knowledge_base.find_install_options(&app_name),
                web_alternatives: self.knowledge_base.find_web_alternatives(&app_name),
            };
        }

        // Login required signals
        let login_signals = [
            "login required", "sign in", "not logged in", "session expired",
            "unauthorized", "401", "403", "authentication required",
            "please log in", "session has expired",
        ];
        if login_signals.iter().any(|s| error_msg.contains(s)) {
            let service = self.infer_service_from_step(step);
            return WorkflowBlocker::NotLoggedIn {
                service: service.clone(),
                login_methods: self.knowledge_base.login_methods_for(&service),
                login_urls_per_browser: self.build_login_urls(&service),
            };
        }

        // Network issues
        if error_msg.contains("connection refused") || error_msg.contains("network unreachable")
            || error_msg.contains("dns") || error_msg.contains("could not resolve") {
            return WorkflowBlocker::NetworkUnavailable {
                affected_service: self.infer_service_from_step(step),
                suggestion: "Check your network connection and try again.".into(),
            };
        }

        // Permission
        if error_msg.contains("permission denied") || error_msg.contains("access denied") {
            return WorkflowBlocker::PermissionDenied {
                action: step.action.clone(),
                elevation_method: self.suggest_elevation(step),
            };
        }

        // uinput
        if error_msg.contains("uinput") || error_msg.contains("input daemon") {
            return WorkflowBlocker::UinputUnavailable {
                manual_alternative: self.suggest_manual_alternative(step),
            };
        }

        // Timeout
        if error_msg.contains("timed out") || error_msg.contains("timeout") {
            return WorkflowBlocker::Timeout {
                action: step.action.clone(),
                elapsed_ms: error.elapsed_ms.unwrap_or(0),
                suggested_action: "Retry with longer timeout or check if the app started.".into(),
            };
        }

        // Default: unknown
        WorkflowBlocker::Unknown {
            raw_error: error.message.clone(),
            retry_safe: step.idempotent,
        }
    }
}
```

---

## 4. Action Suggestion System

### The Suggestion Pipeline

```text
WorkflowBlocker
       ↓
[Knowledge Base Query]  ← finds known recovery patterns for this blocker type
       ↓
List<RecoveryCandidate> ← raw suggestions
       ↓
[Capability Filter]     ← removes options the env can't satisfy
       ↓
[Preference Filter]     ← removes options user has rejected before
       ↓
[Ranking]               ← orders by likelihood of success + user preference
       ↓
List<RecoveryOption>    ← final UI-ready options with buttons
```

### RecoveryOption (the typed structure rendered as UI buttons)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryOption {
    /// Stable ID for this option
    pub id: String,

    /// Title shown on the button (1 line)
    pub title: String,

    /// Subtitle/description (1-2 lines, explains what will happen)
    pub description: String,

    /// Icon name (lucide-react icon name)
    pub icon: String,

    /// Visual category for color coding
    pub category: RecoveryCategory,

    /// The action to execute when clicked
    pub action: RecoveryAction,

    /// Estimated time to complete this recovery
    pub estimated_duration_secs: Option<u32>,

    /// Whether this requires user attention during execution (e.g., scan QR)
    pub requires_user_attention: bool,

    /// Confidence score 0.0-1.0 (how likely this recovery will succeed)
    pub success_confidence: f32,

    /// Whether to show in the primary action area or "more options"
    pub primary: bool,

    /// Risk level — affects button styling and confirmation dialog
    pub risk_level: RiskLevel,

    /// Preview of what will happen (for HITL confirmation)
    pub preview: RecoveryPreview,

    /// Whether to remember this choice for similar future failures
    pub allow_remember: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryCategory {
    /// Install something
    Install,
    /// Open an alternative
    Alternative,
    /// Login flow
    Login,
    /// Run a command
    Command,
    /// Switch substrate
    SwitchStrategy,
    /// Manual step
    Manual,
    /// Skip and continue
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPreview {
    pub will_install: Vec<String>,
    pub will_open: Vec<String>,
    pub will_run_commands: Vec<String>,
    pub will_navigate_to: Vec<String>,
    pub will_modify_files: Vec<String>,
    pub estimated_time: String,
    pub privacy_notes: Vec<String>,  // e.g., "Will share login state with Chrome"
}
```

### Example: WhatsApp Recovery Generation

```rust
fn generate_whatsapp_recovery(
    blocker: &WorkflowBlocker,
    capabilities: &CapabilitySet,
    message_context: &MessageContext,
) -> Vec<RecoveryOption> {
    let mut options = Vec::new();

    // Option 1: Install native WhatsApp (if Flatpak/Snap available)
    if capabilities.has_flatpak() {
        options.push(RecoveryOption {
            id: "install_whatsapp_flatpak".into(),
            title: "📦 Install WhatsApp Desktop".into(),
            description: "Install via Flatpak. ~120MB download. After install, requires QR code scan with your phone.".into(),
            icon: "package".into(),
            category: RecoveryCategory::Install,
            action: RecoveryAction::InstallApp {
                app_id: "whatsapp".into(),
                package_manager: PackageManager::Flatpak,
                package_name: "com.rtosta.zapzap".into(),
                post_install_step: Some(Box::new(RecoveryAction::Sequence {
                    steps: vec![
                        RecoveryAction::OpenAlternative {
                            target: AlternativeTarget::AlternativeApp {
                                app_id: "whatsapp".into(),
                                file_arg: None,
                            },
                            post_open_step: None,
                        },
                        RecoveryAction::ManualStep {
                            instruction: "Scan the QR code with your phone's WhatsApp camera.".into(),
                            completion_signal: CompletionSignal::AccessibilityElement {
                                role: "list".into(),
                                name: "Chats".into(),
                                max_wait_secs: 120,
                            },
                        },
                    ],
                    continue_on_failure: false,
                })),
            },
            estimated_duration_secs: Some(180),
            requires_user_attention: true,
            success_confidence: 0.85,
            primary: true,
            risk_level: RiskLevel::Green,
            preview: RecoveryPreview {
                will_install: vec!["WhatsApp Desktop (com.rtosta.zapzap, ~120MB)".into()],
                will_open: vec!["WhatsApp Desktop".into()],
                will_run_commands: vec!["flatpak install -y flathub com.rtosta.zapzap".into()],
                will_navigate_to: vec![],
                will_modify_files: vec![],
                estimated_time: "~3 minutes".into(),
                privacy_notes: vec![
                    "WhatsApp Desktop links to your phone account.".into(),
                    "QR code scan is required to authenticate.".into(),
                ],
            },
            allow_remember: true,
        });
    }

    // Option 2-N: Open WhatsApp Web in each available browser
    let web_url = "https://web.whatsapp.com";
    for browser in capabilities.installed_browsers() {
        let already_logged_in = capabilities.has_browser_session(&browser, "web.whatsapp.com");

        options.push(RecoveryOption {
            id: format!("open_whatsapp_web_{}", browser.id),
            title: format!("🌐 Open WhatsApp Web in {}", browser.display_name),
            description: if already_logged_in {
                format!("You're already logged in. Will resume the message workflow.")
            } else {
                format!("Will navigate to web.whatsapp.com. You'll need to scan the QR code with your phone.")
            },
            icon: browser.icon.clone(),
            category: RecoveryCategory::Alternative,
            action: RecoveryAction::OpenAlternative {
                target: AlternativeTarget::UrlInBrowser {
                    url: web_url.into(),
                    browser: browser.id.clone(),
                },
                post_open_step: Some(Box::new(if already_logged_in {
                    RecoveryAction::Sequence {
                        steps: vec![],  // resume workflow directly
                        continue_on_failure: false,
                    }
                } else {
                    RecoveryAction::ManualStep {
                        instruction: format!(
                            "Scan the QR code in {}. The workflow will continue automatically once you're logged in.",
                            browser.display_name
                        ),
                        completion_signal: CompletionSignal::AccessibilityElement {
                            role: "main".into(),
                            name: "Chats".into(),
                            max_wait_secs: 120,
                        },
                    }
                })),
            },
            estimated_duration_secs: if already_logged_in { Some(5) } else { Some(60) },
            requires_user_attention: !already_logged_in,
            success_confidence: if already_logged_in { 0.95 } else { 0.75 },
            primary: already_logged_in,  // primary if already logged in
            risk_level: RiskLevel::Green,
            preview: RecoveryPreview {
                will_install: vec![],
                will_open: vec![format!("{} → web.whatsapp.com", browser.display_name)],
                will_run_commands: vec![],
                will_navigate_to: vec![web_url.into()],
                will_modify_files: vec![],
                estimated_time: if already_logged_in { "~5 seconds".into() } else { "~1 minute".into() },
                privacy_notes: if already_logged_in {
                    vec!["Will use your existing browser session.".into()]
                } else {
                    vec!["Browser session will be tied to your phone.".into()]
                },
            },
            allow_remember: true,
        });
    }

    // Option N+1: Send via SMS instead
    if message_context.has_phone_number() {
        options.push(RecoveryOption {
            id: "send_via_sms".into(),
            title: "✉️ Send via SMS instead".into(),
            description: format!(
                "Open the default SMS app with the message pre-filled to {}.",
                message_context.contact_phone.as_deref().unwrap_or("the contact")
            ),
            icon: "message-circle".into(),
            category: RecoveryCategory::Alternative,
            action: RecoveryAction::SwitchCommunicationChannel {
                original_service: "whatsapp".into(),
                alternative_service: "sms".into(),
                message_to_send: message_context.message_body.clone(),
            },
            estimated_duration_secs: Some(10),
            requires_user_attention: true,
            success_confidence: 0.80,
            primary: false,
            risk_level: RiskLevel::Green,
            preview: RecoveryPreview {
                will_install: vec![],
                will_open: vec!["Default SMS application".into()],
                will_run_commands: vec![],
                will_navigate_to: vec![],
                will_modify_files: vec![],
                estimated_time: "~10 seconds".into(),
                privacy_notes: vec!["Message will be sent over SMS instead of WhatsApp.".into()],
            },
            allow_remember: false,
        });
    }

    options
}
```

---

## 5. Workflow Resumption Protocol

### Linking Recovery to Original Workflow

When a recovery succeeds, the original workflow must resume. Implementation:

```rust
pub struct WorkflowSession {
    pub primary_workflow: WorkflowInstance,
    pub recovery_chain: Vec<RecoveryWorkflow>,
    pub state: SessionState,
}

#[derive(Debug)]
pub struct RecoveryWorkflow {
    pub id: String,
    pub parent_workflow_id: String,
    pub blocker: WorkflowBlocker,
    pub action: RecoveryAction,
    pub state: WorkflowState,
    pub started_at: Instant,
    pub completion_signal: Option<CompletionSignal>,
}

#[derive(Debug)]
pub enum SessionState {
    /// Primary workflow running normally
    PrimaryActive,
    /// Primary suspended, recovery in progress
    RecoveryActive { recovery_id: String, suspended_at_step: u32 },
    /// Recovery succeeded, primary resuming
    PrimaryResuming { from_step: u32 },
    /// Recovery failed, ask user for next step
    RecoveryFailed { recovery_id: String, fallback_options: Vec<RecoveryOption> },
    /// All workflows complete
    Complete { verdict: WorkflowVerdict },
    /// All workflows cancelled
    Cancelled,
}
```

### Resumption Flow

```text
1. Primary workflow runs
2. Step N fails → FailureCognitionEngine produces RecoveryDiagnosis
3. UI shows recovery options
4. User clicks "Open WhatsApp Web in Chrome"
5. Backend creates RecoveryWorkflow:
   - Action: OpenAlternative { url: web.whatsapp.com, browser: chrome }
   - CompletionSignal: AccessibilityElement { role: "main", name: "Chats" }
6. Primary workflow → SUSPENDED
7. Recovery workflow → executes (opens Chrome to URL)
8. Backend monitors CompletionSignal in background
9. User scans QR code → "Chats" element appears in AT-SPI tree
10. CompletionSignal fires
11. Recovery workflow → COMPLETE
12. Primary workflow → RESUMING from step N (re-execute the failed step)
13. Step N now succeeds (because the precondition is now met)
14. Workflow continues to step N+1, ..., completion
```

### Code: Workflow Resumption Manager

```rust
pub struct WorkflowResumptionManager {
    sessions: Arc<Mutex<HashMap<String, WorkflowSession>>>,
    completion_monitor: CompletionMonitor,
}

impl WorkflowResumptionManager {
    pub async fn handle_recovery_completion(
        &self,
        session_id: &str,
        recovery_id: &str,
    ) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions.get_mut(session_id).ok_or(ResumeError::SessionNotFound)?;

        // Find the recovery workflow
        let recovery = session.recovery_chain.iter_mut()
            .find(|r| r.id == recovery_id)
            .ok_or(ResumeError::RecoveryNotFound)?;

        recovery.state = WorkflowState::Finalized {
            verdict: WorkflowVerdict::Complete,
        };

        // Re-evaluate the precondition that originally failed
        let SessionState::RecoveryActive { suspended_at_step, .. } = &session.state else {
            return Err(ResumeError::InvalidSessionState);
        };

        let suspended_at_step = *suspended_at_step;

        // Resume primary workflow from the failed step
        session.state = SessionState::PrimaryResuming {
            from_step: suspended_at_step,
        };

        // Emit telemetry
        emit_telemetry(WorkflowTelemetry::WorkflowResumed {
            workflow_id: session.primary_workflow.id.clone(),
            after_recovery_id: recovery_id.to_string(),
            resuming_from_step: suspended_at_step,
        });

        // Re-execute the failed step
        let executor = WorkflowExecutor::new();
        executor.resume_from_step(
            &mut session.primary_workflow,
            suspended_at_step,
        ).await
    }
}
```

### Completion Signal Monitoring

The system polls/observes completion signals in the background:

```rust
pub struct CompletionMonitor {
    active_signals: Arc<Mutex<HashMap<String, ActiveSignal>>>,
    atspi_engine: Arc<AtSpiEngine>,
    process_table: Arc<ProcessTableObserver>,
}

pub struct ActiveSignal {
    pub recovery_id: String,
    pub session_id: String,
    pub signal: CompletionSignal,
    pub started_at: Instant,
    pub poll_interval_ms: u64,
    pub timeout_at: Instant,
}

impl CompletionMonitor {
    pub async fn watch_signal(
        &self,
        session_id: String,
        recovery_id: String,
        signal: CompletionSignal,
    ) {
        let active = ActiveSignal {
            recovery_id: recovery_id.clone(),
            session_id: session_id.clone(),
            signal: signal.clone(),
            started_at: Instant::now(),
            poll_interval_ms: 1000,  // Poll every 1s
            timeout_at: Instant::now() + Duration::from_secs(self.timeout_for(&signal)),
        };

        self.active_signals.lock().await.insert(recovery_id.clone(), active);

        // Spawn monitor task
        let monitor = self.clone();
        tokio::spawn(async move {
            monitor.poll_until_satisfied(session_id, recovery_id, signal).await;
        });
    }

    async fn poll_until_satisfied(
        &self,
        session_id: String,
        recovery_id: String,
        signal: CompletionSignal,
    ) {
        let timeout_secs = self.timeout_for(&signal);
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);

        while Instant::now() < deadline {
            let satisfied = match &signal {
                CompletionSignal::ProcessAppears { binary, .. } => {
                    self.process_table.is_running(binary)
                }
                CompletionSignal::FileAppears { path, .. } => {
                    path.exists()
                }
                CompletionSignal::UrlReachable { url, .. } => {
                    self.check_url_reachable(url).await
                }
                CompletionSignal::AccessibilityElement { role, name, .. } => {
                    self.atspi_engine.find_element(role, name).await.is_ok()
                }
                CompletionSignal::PortListening { port, .. } => {
                    self.is_port_listening(*port).await
                }
                CompletionSignal::UserConfirmation { .. } => {
                    false  // Wait for explicit user click
                }
            };

            if satisfied {
                let _ = self.notify_satisfied(&session_id, &recovery_id).await;
                return;
            }

            tokio::time::sleep(Duration::from_millis(1000)).await;
        }

        // Timeout — recovery did not complete
        let _ = self.notify_timeout(&session_id, &recovery_id).await;
    }
}
```

---

## 6. UI Components Specification

### Component Hierarchy

```text
ChatView
├── MessageList
├── ActiveWorkflowPanel
│   ├── WorkflowProgress (existing)
│   ├── RecoveryPanel (NEW) ← shown when workflow blocked
│   │   ├── BlockerExplanation
│   │   ├── RecoveryOptionList
│   │   │   ├── PrimaryOption[] (large buttons)
│   │   │   └── SecondaryOptions (collapsed list)
│   │   ├── RecoveryActionPreview (modal)
│   │   └── SkipCancelControls
│   ├── HitlModal (existing)
│   └── ContinuationPanel (post-completion)
├── ChatInput
└── ...
```

### RecoveryPanel Component (TypeScript / SolidJS)

```tsx
// ui/src/components/RecoveryPanel.tsx

import { Component, createSignal, For, Show } from "solid-js";
import { RecoveryOption, RecoveryDiagnosis } from "../types/recovery";

interface RecoveryPanelProps {
  diagnosis: RecoveryDiagnosis;
  onActionSelected: (option: RecoveryOption) => void;
  onSkip: () => void;
  onCancel: () => void;
  onPreview: (option: RecoveryOption) => void;
}

export const RecoveryPanel: Component<RecoveryPanelProps> = (props) => {
  const [showAllOptions, setShowAllOptions] = createSignal(false);
  const [previewOption, setPreviewOption] = createSignal<RecoveryOption | null>(null);

  const primaryOptions = () => props.diagnosis.recovery_options.filter(o => o.primary);
  const secondaryOptions = () => props.diagnosis.recovery_options.filter(o => !o.primary);

  return (
    <div class="recovery-panel border rounded-lg p-4 bg-amber-50 dark:bg-amber-900/20">
      {/* Blocker explanation */}
      <div class="flex items-start gap-3 mb-4">
        <BlockerIcon blocker={props.diagnosis.blocker} />
        <div>
          <h3 class="font-semibold text-base">{props.diagnosis.explanation.title}</h3>
          <p class="text-sm text-gray-600 dark:text-gray-300 mt-1">
            {props.diagnosis.explanation.description}
          </p>
        </div>
      </div>

      {/* Suggestion intro */}
      <p class="text-sm font-medium mb-3">Here's what I can do:</p>

      {/* Primary options — large prominent buttons */}
      <div class="space-y-2 mb-3">
        <For each={primaryOptions()}>
          {(option) => (
            <RecoveryOptionCard
              option={option}
              variant="primary"
              onSelect={() => props.onActionSelected(option)}
              onPreview={() => setPreviewOption(option)}
            />
          )}
        </For>
      </div>

      {/* Secondary options — collapsed */}
      <Show when={secondaryOptions().length > 0}>
        <button
          class="text-sm text-blue-600 hover:underline mb-2"
          onClick={() => setShowAllOptions(!showAllOptions())}
        >
          {showAllOptions() ? "Hide" : `Show ${secondaryOptions().length} more options`}
        </button>
        <Show when={showAllOptions()}>
          <div class="space-y-2 mb-3">
            <For each={secondaryOptions()}>
              {(option) => (
                <RecoveryOptionCard
                  option={option}
                  variant="secondary"
                  onSelect={() => props.onActionSelected(option)}
                  onPreview={() => setPreviewOption(option)}
                />
              )}
            </For>
          </div>
        </Show>
      </Show>

      {/* Skip / Cancel controls */}
      <div class="flex gap-2 mt-4 pt-3 border-t">
        <Show when={props.diagnosis.allow_skip}>
          <button
            class="text-sm px-3 py-1 rounded border border-gray-300 hover:bg-gray-100"
            onClick={props.onSkip}
          >
            Skip This Step
          </button>
        </Show>
        <button
          class="text-sm px-3 py-1 rounded border border-red-300 text-red-600 hover:bg-red-50"
          onClick={props.onCancel}
        >
          Cancel Workflow
        </button>
      </div>

      {/* Preview modal */}
      <Show when={previewOption()}>
        <RecoveryPreviewModal
          option={previewOption()!}
          onConfirm={() => {
            props.onActionSelected(previewOption()!);
            setPreviewOption(null);
          }}
          onClose={() => setPreviewOption(null)}
        />
      </Show>
    </div>
  );
};
```

### RecoveryOptionCard Component

```tsx
// ui/src/components/RecoveryOptionCard.tsx

interface RecoveryOptionCardProps {
  option: RecoveryOption;
  variant: "primary" | "secondary";
  onSelect: () => void;
  onPreview: () => void;
}

export const RecoveryOptionCard: Component<RecoveryOptionCardProps> = (props) => {
  const cardClass = props.variant === "primary"
    ? "p-3 border-2 border-blue-200 hover:border-blue-400 bg-white dark:bg-gray-800 cursor-pointer"
    : "p-2 border border-gray-200 hover:border-gray-300 bg-white dark:bg-gray-800 cursor-pointer";

  return (
    <div class={`rounded-lg ${cardClass} transition-colors`}>
      <div class="flex items-start gap-3">
        <div class="text-2xl">{getIconForOption(props.option.icon)}</div>
        <div class="flex-1">
          <div class="flex items-center justify-between">
            <h4 class="font-medium text-sm">{props.option.title}</h4>
            <Show when={props.option.estimated_duration_secs}>
              <span class="text-xs text-gray-500">
                ~{formatDuration(props.option.estimated_duration_secs!)}
              </span>
            </Show>
          </div>
          <p class="text-xs text-gray-600 dark:text-gray-400 mt-1">
            {props.option.description}
          </p>

          {/* Action buttons */}
          <div class="flex gap-2 mt-2">
            <button
              class="text-xs px-2 py-1 bg-blue-600 text-white rounded hover:bg-blue-700"
              onClick={(e) => {
                e.stopPropagation();
                props.onSelect();
              }}
            >
              {getActionLabel(props.option)}
            </button>
            <button
              class="text-xs px-2 py-1 bg-gray-100 hover:bg-gray-200 rounded"
              onClick={(e) => {
                e.stopPropagation();
                props.onPreview();
              }}
            >
              Preview
            </button>
            <Show when={props.option.requires_user_attention}>
              <span class="text-xs px-2 py-1 bg-amber-100 text-amber-700 rounded">
                Needs your attention
              </span>
            </Show>
          </div>
        </div>
      </div>
    </div>
  );
};

function getActionLabel(option: RecoveryOption): string {
  switch (option.category) {
    case "Install": return "Install Now";
    case "Alternative": return "Open This";
    case "Login": return "Start Login";
    case "Command": return "Run Command";
    case "Manual": return "Show Instructions";
    case "Skip": return "Skip";
    default: return "Continue";
  }
}
```

### RecoveryPreviewModal — "What will happen?"

```tsx
interface RecoveryPreviewModalProps {
  option: RecoveryOption;
  onConfirm: () => void;
  onClose: () => void;
}

export const RecoveryPreviewModal: Component<RecoveryPreviewModalProps> = (props) => {
  const preview = props.option.preview;

  return (
    <div class="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div class="bg-white dark:bg-gray-900 rounded-lg p-6 max-w-md w-full m-4">
        <h3 class="font-semibold mb-3">Confirm: {props.option.title}</h3>

        <div class="space-y-3 text-sm">
          <Show when={preview.will_install.length > 0}>
            <Section icon="📦" title="Will install">
              <For each={preview.will_install}>{(item) => <li>{item}</li>}</For>
            </Section>
          </Show>

          <Show when={preview.will_run_commands.length > 0}>
            <Section icon="⌨️" title="Will run">
              <For each={preview.will_run_commands}>{(cmd) =>
                <code class="block bg-gray-100 dark:bg-gray-800 p-1 rounded">{cmd}</code>
              }</For>
            </Section>
          </Show>

          <Show when={preview.will_open.length > 0}>
            <Section icon="🪟" title="Will open">
              <For each={preview.will_open}>{(item) => <li>{item}</li>}</For>
            </Section>
          </Show>

          <Show when={preview.will_navigate_to.length > 0}>
            <Section icon="🌐" title="Will navigate to">
              <For each={preview.will_navigate_to}>{(url) =>
                <code class="text-blue-600 break-all">{url}</code>
              }</For>
            </Section>
          </Show>

          <Show when={preview.privacy_notes.length > 0}>
            <Section icon="🔒" title="Privacy notes" highlight>
              <For each={preview.privacy_notes}>{(note) => <li>{note}</li>}</For>
            </Section>
          </Show>

          <div class="flex items-center gap-2 text-gray-500">
            <span>⏱️</span>
            <span>Estimated time: {preview.estimated_time}</span>
          </div>
        </div>

        {/* Risk indicator */}
        <Show when={props.option.risk_level !== "Green"}>
          <RiskWarning level={props.option.risk_level} />
        </Show>

        <div class="flex gap-2 mt-4">
          <button class="flex-1 px-3 py-2 bg-blue-600 text-white rounded" onClick={props.onConfirm}>
            Yes, do this
          </button>
          <button class="flex-1 px-3 py-2 bg-gray-200 rounded" onClick={props.onClose}>
            Cancel
          </button>
        </div>

        <Show when={props.option.allow_remember}>
          <label class="flex items-center gap-2 mt-3 text-xs text-gray-600">
            <input type="checkbox" />
            Remember this choice for similar situations
          </label>
        </Show>
      </div>
    </div>
  );
};
```

### Live Recovery Progress Component

When a recovery is executing, show progress with live updates:

```tsx
interface RecoveryProgressProps {
  recovery: ActiveRecovery;
  onCancel: () => void;
}

export const RecoveryProgress: Component<RecoveryProgressProps> = (props) => {
  return (
    <div class="recovery-progress p-3 border rounded-lg bg-blue-50">
      <div class="flex items-center gap-2 mb-2">
        <Spinner />
        <h4 class="font-medium text-sm">Recovering: {props.recovery.title}</h4>
      </div>

      {/* Step progress */}
      <For each={props.recovery.steps}>
        {(step, idx) => (
          <div class="flex items-center gap-2 text-sm py-1">
            <StepIcon status={step.status} />
            <span class={step.status === "complete" ? "text-gray-500 line-through" : ""}>
              {step.description}
            </span>
            <Show when={step.status === "active"}>
              <span class="text-xs text-blue-600">{step.elapsed_secs}s elapsed</span>
            </Show>
          </div>
        )}
      </For>

      {/* User attention required */}
      <Show when={props.recovery.requires_user_attention}>
        <div class="mt-3 p-2 bg-amber-100 rounded text-sm">
          <strong>👋 Please complete:</strong> {props.recovery.attention_instruction}
          <Show when={props.recovery.allows_done_button}>
            <button class="ml-2 px-2 py-1 bg-green-600 text-white text-xs rounded">
              I'm done
            </button>
          </Show>
        </div>
      </Show>

      <button
        class="mt-2 text-xs text-red-600 hover:underline"
        onClick={props.onCancel}
      >
        Cancel recovery and go back to options
      </button>
    </div>
  );
};
```

---

## 7. Backend Architecture

### Module Layout

```text
crates/kria-core/src/agent/recovery/
├── mod.rs                          # Public API
├── types.rs                        # WorkflowBlocker, RecoveryAction, RecoveryOption
├── cognition_engine.rs             # FailureCognitionEngine
├── knowledge_base.rs               # RecoveryKnowledgeBase
├── action_executor.rs              # Executes RecoveryAction
├── completion_monitor.rs           # CompletionMonitor for signal watching
├── resumption_manager.rs           # WorkflowResumptionManager
├── preferences.rs                  # UserPreferenceStore (remember choices)
├── history.rs                      # RecoveryHistoryStore (learning)
└── tests/
    ├── whatsapp_recovery.rs
    ├── code_install_recovery.rs
    ├── browser_login_recovery.rs
    └── ...

crates/kria-core/src/agent/recovery_strategies/
├── mod.rs
├── app_install.rs                  # Strategy: install missing app
├── browser_alternative.rs          # Strategy: open in alternative browser
├── login_flow.rs                   # Strategy: initiate login
├── command_substitution.rs         # Strategy: alternative command
├── manual_step.rs                  # Strategy: ask user to do it
├── communication_switch.rs         # Strategy: SMS instead of WhatsApp
└── ...
```

### Wiring into the Workflow Lifecycle

```rust
// In WorkflowExecutor::execute_step:

let result = self.execute_step_inner(step, ctx).await;

if !result.success {
    // Step failed — invoke recovery cognition
    let diagnosis = self.failure_cognition_engine
        .diagnose(step, &result.error.unwrap(), &self.memory, &self.original_intent)
        .await;

    // Emit telemetry — frontend will show recovery panel
    self.emit_telemetry(WorkflowTelemetry::RecoveryRequired {
        workflow_id: self.workflow_id.clone(),
        step_index: step.index,
        diagnosis: diagnosis.clone(),
    }).await;

    // Suspend workflow
    self.transition_state(WorkflowState::RecoveryPending {
        diagnosis,
        suspended_at_step: step.index,
    });

    return StepResult::SuspendedForRecovery;
}
```

### Recovery Action Executor

```rust
pub struct RecoveryActionExecutor {
    tool_registry: Arc<ToolRegistry>,
    completion_monitor: Arc<CompletionMonitor>,
    capability_set: CapabilitySet,
}

impl RecoveryActionExecutor {
    pub async fn execute(
        &self,
        action: &RecoveryAction,
        session_id: &str,
        recovery_id: &str,
    ) -> RecoveryExecutionResult {
        match action {
            RecoveryAction::InstallApp { app_id, package_manager, package_name, post_install_step } => {
                // Build install command
                let cmd = build_install_command(package_manager, package_name);

                // Execute install (with progress streaming)
                self.run_command_with_progress(&cmd, session_id).await?;

                // Verify installation
                let verify = self.verify_app_installed(app_id).await;
                if !verify.success {
                    return RecoveryExecutionResult::Failed {
                        reason: format!("Install command ran but {} is still not detected", app_id),
                        fallback_options: self.suggest_fallback_after_install_failure(app_id),
                    };
                }

                // Run post-install step if defined (e.g., open the app + manual step)
                if let Some(post) = post_install_step {
                    return Box::pin(self.execute(post, session_id, recovery_id)).await;
                }

                RecoveryExecutionResult::Success
            }

            RecoveryAction::OpenAlternative { target, post_open_step } => {
                self.execute_open_alternative(target, post_open_step, session_id, recovery_id).await
            }

            RecoveryAction::LoginFlow { service, login_url, login_method, verification } => {
                self.execute_login_flow(service, login_url, login_method, verification, session_id, recovery_id).await
            }

            RecoveryAction::ManualStep { instruction, completion_signal } => {
                // Emit instruction to UI
                self.emit_manual_step_instruction(instruction, session_id).await;

                // Watch for completion signal
                self.completion_monitor.watch_signal(
                    session_id.to_string(),
                    recovery_id.to_string(),
                    completion_signal.clone(),
                ).await;

                RecoveryExecutionResult::WaitingForSignal { signal: completion_signal.clone() }
            }

            RecoveryAction::Sequence { steps, continue_on_failure } => {
                for step in steps {
                    let result = Box::pin(self.execute(step, session_id, recovery_id)).await;
                    if matches!(result, RecoveryExecutionResult::Failed { .. }) && !continue_on_failure {
                        return result;
                    }
                }
                RecoveryExecutionResult::Success
            }

            RecoveryAction::SwitchCommunicationChannel { original_service, alternative_service, message_to_send } => {
                self.execute_communication_switch(
                    original_service, alternative_service, message_to_send, session_id
                ).await
            }

            // ... other variants ...
        }
    }
}
```

---

## 8. Knowledge Base for Smart Suggestions

The intelligence of the system depends on a curated knowledge base of recovery patterns.

### Knowledge Base Structure

```rust
pub struct RecoveryKnowledgeBase {
    /// Service-specific recovery patterns
    services: HashMap<String, ServiceRecoveryConfig>,
    /// App installation registry
    install_registry: HashMap<String, AppInstallConfig>,
    /// Browser-service compatibility matrix
    browser_compatibility: HashMap<String, Vec<String>>,
    /// Communication alternatives (WhatsApp → SMS, Email; Discord → Element; etc.)
    communication_alternatives: HashMap<String, Vec<CommunicationAlternative>>,
}

pub struct ServiceRecoveryConfig {
    pub service_name: String,
    pub native_apps: Vec<AppInstallConfig>,
    pub web_url: Option<String>,
    pub login_methods: Vec<LoginMethod>,
    pub session_check: SessionCheckMethod,
    pub alternatives: Vec<CommunicationAlternative>,
}

pub struct AppInstallConfig {
    pub app_id: String,
    pub display_name: String,
    pub install_options: Vec<InstallOption>,
    pub binary_name: String,
    pub window_class: String,
    pub typical_install_size_mb: u32,
    pub requires_account: bool,
}
```

### Example Knowledge Base Entries

```rust
// In recovery/knowledge_base/services.rs

pub fn whatsapp_recovery_config() -> ServiceRecoveryConfig {
    ServiceRecoveryConfig {
        service_name: "whatsapp".into(),
        native_apps: vec![
            AppInstallConfig {
                app_id: "whatsapp".into(),
                display_name: "WhatsApp Desktop".into(),
                install_options: vec![
                    InstallOption {
                        package_manager: PackageManager::Flatpak,
                        package_name: "com.rtosta.zapzap".into(),
                        estimated_size_mb: 120,
                        install_command: "flatpak install -y flathub com.rtosta.zapzap".into(),
                        requires_sudo: false,
                    },
                    InstallOption {
                        package_manager: PackageManager::Snap,
                        package_name: "whatsdesk".into(),
                        estimated_size_mb: 95,
                        install_command: "sudo snap install whatsdesk".into(),
                        requires_sudo: true,
                    },
                ],
                binary_name: "zapzap".into(),
                window_class: "ZapZap".into(),
                typical_install_size_mb: 120,
                requires_account: true,
            },
        ],
        web_url: Some("https://web.whatsapp.com".into()),
        login_methods: vec![
            LoginMethod::QrCodeScan { wait_timeout_secs: 300 },
        ],
        session_check: SessionCheckMethod::BrowserSession {
            domain: "web.whatsapp.com".into(),
        },
        alternatives: vec![
            CommunicationAlternative {
                service: "sms".into(),
                description: "Send via default SMS app".into(),
                requires_phone_number: true,
            },
            CommunicationAlternative {
                service: "email".into(),
                description: "Send via email instead".into(),
                requires_email: true,
            },
            CommunicationAlternative {
                service: "telegram".into(),
                description: "Send via Telegram if you have the contact there".into(),
                requires_phone_number: true,
            },
        ],
    }
}

pub fn vscode_recovery_config() -> ServiceRecoveryConfig {
    ServiceRecoveryConfig {
        service_name: "vscode".into(),
        native_apps: vec![
            AppInstallConfig {
                app_id: "vscode".into(),
                display_name: "Visual Studio Code".into(),
                install_options: vec![
                    InstallOption {
                        package_manager: PackageManager::Snap,
                        package_name: "code".into(),
                        estimated_size_mb: 200,
                        install_command: "sudo snap install code --classic".into(),
                        requires_sudo: true,
                    },
                    InstallOption {
                        package_manager: PackageManager::Flatpak,
                        package_name: "com.visualstudio.code".into(),
                        estimated_size_mb: 250,
                        install_command: "flatpak install -y flathub com.visualstudio.code".into(),
                        requires_sudo: false,
                    },
                ],
                binary_name: "code".into(),
                window_class: "Code".into(),
                typical_install_size_mb: 200,
                requires_account: false,
            },
        ],
        web_url: Some("https://vscode.dev".into()),
        login_methods: vec![],
        session_check: SessionCheckMethod::Unverifiable,
        alternatives: vec![
            CommunicationAlternative {
                service: "gedit".into(),
                description: "Use gedit (basic text editor) instead".into(),
                requires_phone_number: false,
            },
            CommunicationAlternative {
                service: "nano".into(),
                description: "Use nano (terminal editor) instead".into(),
                requires_phone_number: false,
            },
            CommunicationAlternative {
                service: "vim".into(),
                description: "Use vim instead".into(),
                requires_phone_number: false,
            },
        ],
    }
}
```

### Knowledge Base Loading

```rust
impl RecoveryKnowledgeBase {
    pub fn load_default() -> Self {
        let mut services = HashMap::new();

        // Communication services
        services.insert("whatsapp".into(), whatsapp_recovery_config());
        services.insert("telegram".into(), telegram_recovery_config());
        services.insert("discord".into(), discord_recovery_config());
        services.insert("slack".into(), slack_recovery_config());
        services.insert("signal".into(), signal_recovery_config());

        // IDEs and editors
        services.insert("vscode".into(), vscode_recovery_config());
        services.insert("intellij".into(), intellij_recovery_config());
        services.insert("sublime".into(), sublime_recovery_config());

        // Browsers
        services.insert("chrome".into(), chrome_recovery_config());
        services.insert("firefox".into(), firefox_recovery_config());
        services.insert("brave".into(), brave_recovery_config());

        // Productivity
        services.insert("zoom".into(), zoom_recovery_config());
        services.insert("teams".into(), teams_recovery_config());
        services.insert("meet".into(), meet_recovery_config());

        // ... 30+ more services

        Self { services, /* ... */ }
    }
}
```

### Extensibility — User-Defined Knowledge

Users (or the community) can extend the knowledge base via TOML:

```toml
# ~/.kria/recovery/services/custom_app.toml
[service]
name = "myapp"
display_name = "My Custom App"

[[native_apps]]
app_id = "myapp"
display_name = "My App Desktop"
binary_name = "myapp"
window_class = "MyApp"

[[native_apps.install_options]]
package_manager = "Flatpak"
package_name = "com.example.MyApp"
estimated_size_mb = 50
install_command = "flatpak install -y flathub com.example.MyApp"
```

---

## 9. Multi-Step Recovery Workflows

Some recoveries need multiple steps coordinated. Example: install WhatsApp + login + send message.

### Composite Recovery Example

```rust
RecoveryAction::Sequence {
    steps: vec![
        // Step 1: Install
        RecoveryAction::InstallApp {
            app_id: "whatsapp".into(),
            package_manager: PackageManager::Flatpak,
            package_name: "com.rtosta.zapzap".into(),
            post_install_step: None,
        },
        // Step 2: Open
        RecoveryAction::OpenAlternative {
            target: AlternativeTarget::AlternativeApp {
                app_id: "whatsapp".into(),
                file_arg: None,
            },
            post_open_step: None,
        },
        // Step 3: Manual login (QR scan)
        RecoveryAction::ManualStep {
            instruction: "Scan the QR code with your phone's WhatsApp camera (Settings → Linked Devices → Link a Device)".into(),
            completion_signal: CompletionSignal::AccessibilityElement {
                role: "list".into(),
                name: "Chats".into(),
                max_wait_secs: 300,
            },
        },
    ],
    continue_on_failure: false,
}
```

### Progressive Disclosure During Multi-Step Recovery

The UI shows a step-by-step progress view:

```text
┌─────────────────────────────────────────────────────────┐
│  Recovering: Install WhatsApp + Login                   │
│                                                          │
│  ✓ 1/3 Installing WhatsApp Desktop (45s)                │
│  ● 2/3 Opening WhatsApp...                              │
│  ○ 3/3 Waiting for QR code scan                         │
│                                                          │
│  [Cancel Recovery]                                       │
└─────────────────────────────────────────────────────────┘
```

When step 3 becomes active:

```text
┌─────────────────────────────────────────────────────────┐
│  Recovering: Install WhatsApp + Login                   │
│                                                          │
│  ✓ 1/3 Installing WhatsApp Desktop                      │
│  ✓ 2/3 Opened WhatsApp                                  │
│  ● 3/3 Waiting for you to scan QR code                  │
│                                                          │
│  👋 Please complete:                                    │
│  Open WhatsApp on your phone → Settings →               │
│  Linked Devices → Link a Device → Scan the QR code      │
│  shown in the WhatsApp Desktop window.                  │
│                                                          │
│  Time elapsed: 0:34 (timeout in 4:26)                   │
│                                                          │
│  [I'm done] (only enabled after auto-detection)         │
│  [Cancel Recovery]                                       │
└─────────────────────────────────────────────────────────┘
```

After all steps complete:

```text
┌─────────────────────────────────────────────────────────┐
│  ✓ Recovery complete — resuming original task           │
│                                                          │
│  Now sending "hello" to Faizan via WhatsApp...          │
└─────────────────────────────────────────────────────────┘
```

---

## 10. Implementation Phases

### Phase A: Foundation (Week 1)

**Deliverables:**
1. Define all recovery types (`WorkflowBlocker`, `RecoveryAction`, `RecoveryOption`, etc.) in `recovery/types.rs`
2. Build `RecoveryKnowledgeBase` with seed data for top 10 services (WhatsApp, Telegram, Discord, VS Code, Chrome, Firefox, Brave, Zoom, Slack, Signal)
3. Implement `FailureCognitionEngine::classify_blocker` with explicit error pattern matching
4. Add `RecoveryRequired` and `WorkflowResumed` variants to `WorkflowTelemetry`
5. Wire failure detection in `WorkflowExecutor::execute_step` (emits telemetry on failure, doesn't yet execute recovery)

**Testing:**
- Unit tests: each blocker classification has 5+ error string fixtures
- Integration test: simulate WhatsApp failure → verify correct `RecoveryDiagnosis` produced

**Success criteria:** Frontend receives structured failure info instead of raw error strings.

---

### Phase B: UI Components (Week 1-2)

**Deliverables:**
1. Build `RecoveryPanel` component with primary + secondary options
2. Build `RecoveryOptionCard` with action button + preview button
3. Build `RecoveryPreviewModal` showing what will happen
4. Build `RecoveryProgress` component for active recoveries
5. Wire into `ChatView`: when telemetry is `RecoveryRequired`, render `RecoveryPanel`
6. Add Tauri command `submit_recovery_action(workflow_id, option_id)`

**Testing:**
- Storybook fixtures for each recovery type
- Manual test: trigger a known failure, verify panel renders correctly

**Success criteria:** When KRIA fails, user sees buttons instead of error text.

---

### Phase C: Action Execution (Week 2)

**Deliverables:**
1. Build `RecoveryActionExecutor` with handlers for each `RecoveryAction` variant
2. Build `CompletionMonitor` for signal watching (process, file, AT-SPI, port)
3. Implement `WorkflowResumptionManager` that handles primary/recovery transitions
4. Wire `submit_recovery_action` Tauri command → `RecoveryActionExecutor::execute`
5. On recovery success, automatically re-execute the failed primary step

**Testing:**
- Integration test: WhatsApp scenario end-to-end
  - Start: prompt asks to message someone
  - Step fails: WhatsApp not installed
  - Recovery shown: install + web alternatives
  - Click "Open WhatsApp Web in Chrome"
  - Chrome opens to web.whatsapp.com
  - Wait for AT-SPI signal (mocked: send the signal manually)
  - Original workflow resumes
  - Message gets sent

**Success criteria:** End-to-end recovery flow works for at least 1 service (WhatsApp).

---

### Phase D: Knowledge Base Expansion (Week 3)

**Deliverables:**
1. Add 25+ services to knowledge base (covering all common workflows)
2. Implement `UserPreferenceStore` to remember chosen options
3. Implement `RecoveryHistoryStore` to track success rates
4. Add ranking algorithm based on history + preferences
5. Add TOML loader for user-defined service configs

**Services to cover:**
- Communication: WhatsApp, Telegram, Discord, Slack, Signal, Teams, SMS, Email
- Editors/IDEs: VS Code, IntelliJ, Sublime, gedit, vim, nano, Cursor, Windsurf
- Browsers: Chrome, Firefox, Brave, Edge, Vivaldi
- Productivity: Zoom, Meet, Notion, Obsidian, Trello
- Media: Spotify, YouTube Music, VLC, mpv
- Files: Files (Nautilus), Dolphin, Thunar
- Terminal: GNOME Terminal, Konsole, Alacritty, Kitty
- Office: LibreOffice (Writer, Calc, Impress), OnlyOffice
- Dev tools: Docker, Postman, DBeaver, GitHub Desktop

**Success criteria:** 80% of failed workflows produce useful recovery options.

---

### Phase E: Advanced Recovery Patterns (Week 4)

**Deliverables:**
1. **Browser session detection** — check existing cookies for already-logged-in services
2. **Cross-app context** — "send via SMS" preserves message + recipient
3. **Composite recoveries** — install + open + login as one flow
4. **Branching recoveries** — if install fails, automatically offer web alternative
5. **User attention awareness** — distinguish "do this for me" from "I need to do this manually"
6. **Smart re-evaluation** — after recovery, check if the precondition is REALLY satisfied before resuming

**Success criteria:** WhatsApp message scenario works end-to-end:
- User: "Text Faizan on WhatsApp 'hello'"
- KRIA detects: no WhatsApp app, no logged-in browser session
- Shows 3 options: Install WhatsApp / Open in Chrome / Open in Brave / Send SMS
- User clicks "Open in Brave"
- Brave opens to web.whatsapp.com
- User scans QR code
- KRIA detects login complete
- Original message workflow resumes
- Message gets sent via WhatsApp Web
- Final UI: "✓ Sent 'hello' to Faizan on WhatsApp"

---

### Phase F: Production Hardening (Week 5)

**Deliverables:**
1. **Cancellation propagation** — clicking "Cancel" during recovery cleans up orphan processes
2. **Timeout handling** — recovery with timeout shows countdown
3. **Failure within recovery** — if the install fails, offer different approach
4. **Persistence** — workflows + recoveries survive app restart
5. **Telemetry persistence** — last 100 recoveries logged to SQLite
6. **Eval suite** — automated tests for each recovery type
7. **Telemetry to frontend** — structured events, no string parsing
8. **Security hardening** — validate every recovery option ID against pre-emitted set

**Success criteria:** System passes 50+ recovery scenarios across full automation eval suite.

---

## 11. Testing Strategy

### Unit Test Coverage

| Module | Test Coverage Target |
|--------|---------------------|
| `cognition_engine` | 95% — every blocker classification path |
| `knowledge_base` | 85% — every service config validated |
| `action_executor` | 90% — every action type with mock executor |
| `completion_monitor` | 90% — every signal type |
| `resumption_manager` | 85% — state transitions |

### Integration Test Scenarios

```rust
#[tokio::test]
async fn whatsapp_install_then_send_message() {
    // 1. Setup: WhatsApp not installed, no browser session
    // 2. Submit: "Text Faizan on WhatsApp 'hello'"
    // 3. Verify: RecoveryDiagnosis emitted with correct blocker
    // 4. Click: "Open WhatsApp Web in Chrome"
    // 5. Verify: Chrome opens, ManualStep emitted with QR scan instruction
    // 6. Mock: AT-SPI signal "Chats" element appears (simulating user scanning QR)
    // 7. Verify: Recovery completes, primary workflow resumes
    // 8. Verify: Message sent (or HITL requesting confirmation)
}

#[tokio::test]
async fn vscode_not_installed_use_gedit_alternative() {
    // 1. Setup: VS Code not installed, gedit installed
    // 2. Submit: "Open VS Code and write hello.py with print('hello')"
    // 3. Verify: Recovery offers "Install VS Code" + "Use gedit"
    // 4. Click: "Use gedit"
    // 5. Verify: Workflow re-plans with gedit, file written, gedit opens with file
}

#[tokio::test]
async fn cloud_llm_fails_local_llm_takes_over() {
    // 1. Setup: cloud LLM mocked to return 400
    // 2. Submit: complex prompt requiring LLM
    // 3. Verify: Telemetry shows cloud failure
    // 4. Verify: Local LLM auto-failover, prompt completes
}
```

### Manual Test Scenarios

A test playbook with 20+ scenarios:

```markdown
## Scenario: WhatsApp Send Message
1. Uninstall WhatsApp: `flatpak uninstall com.rtosta.zapzap`
2. Open Chrome, log out of WhatsApp Web
3. In KRIA: "Send 'test' to Faizan on WhatsApp"
4. Expected: RecoveryPanel appears with 3 options
5. Click "Open WhatsApp Web in Chrome"
6. Expected: Chrome opens to web.whatsapp.com
7. Scan QR code with phone
8. Expected: KRIA detects login, auto-resumes workflow
9. Expected: Final message: "Sent 'test' to Faizan on WhatsApp"
```

---

## 12. Production Examples

### Example 1: WhatsApp Send (the scenario you described)

```text
User: "Text Faizan on WhatsApp 'hello'"

Step 1: KRIA tries to open WhatsApp
   Failure: AppNotInstalled { app_id: "whatsapp" }

Step 2: Recovery diagnosis
   Detected: WhatsApp not installed
   Detected: No active browser session for web.whatsapp.com
   Detected: Chrome and Brave are installed
   Detected: User has phone number for Faizan in contacts

Step 3: UI shows RecoveryPanel
   Primary options:
   - 📦 Install WhatsApp Desktop (~120MB, ~3min)
   - 🌐 Open WhatsApp Web in Chrome (you'll need to scan QR)
   - 🌐 Open WhatsApp Web in Brave (you'll need to scan QR)

   Secondary options:
   - ✉️ Send via SMS instead
   - 📧 Send via Email (if you have Faizan's email)
   - 💬 Send via Telegram (if you have Telegram)

Step 4: User clicks "Open WhatsApp Web in Chrome"
   Recovery executes:
   - Brave opens to https://web.whatsapp.com
   - UI shows progress: "Waiting for you to scan QR code..."
   - CompletionMonitor watches for AT-SPI element 'Chats'

Step 5: User scans QR code
   AT-SPI element 'Chats' detected → recovery complete

Step 6: Primary workflow resumes
   - Find chat with 'Faizan' (search in WhatsApp Web)
   - Type message 'hello' (via uinput injection)
   - Click send button
   - Verify: message visible in chat with timestamp

Final response: "✓ Sent 'hello' to Faizan on WhatsApp"
```

### Example 2: Code Editor Substitution

```text
User: "Open VS Code, write a Python script that prints fibonacci, run it"

Step 1: KRIA tries to open VS Code
   Failure: AppNotInstalled { app_id: "vscode" }

Step 2: Recovery options
   - 📦 Install VS Code via Snap (~200MB, ~2min, requires sudo)
   - 📦 Install VS Code via Flatpak (~250MB, ~3min, no sudo)
   - 🔄 Use gedit instead (basic editor, but workflow can continue)
   - 🔄 Use nano in terminal (terminal-based, will work fine for this script)
   - 🌐 Use vscode.dev (browser-based VS Code)

Step 3: User clicks "Use gedit instead"
   Recovery executes:
   - Replans workflow with gedit as the editor
   - Writes Python script via write_file
   - Opens script in gedit
   - Runs script via execute_bash
   - Shows output

Final response: "✓ Created /tmp/fib.py, opened in gedit, output: [0, 1, 1, 2, 3, 5, 8, 13, 21, 34]"
```

### Example 3: LLM Failure with Smart Fallback

```text
User: "Summarize the last 10 emails about project Alpha"

Step 1: KRIA tries cloud LLM (default for complex queries)
   Failure: LlmError { status: 400, message: "Invalid tool schema" }

Step 2: Auto-failover to local LLM (no UI shown)

Step 3: Local LLM struggles with the long context
   Failure: ContextOverflow { tokens: 12000, limit: 8000 }

Step 4: Recovery options
   - 📉 Process emails in batches (will take ~2 minutes)
   - 🔧 Use simpler summary (one line per email, no theme analysis)
   - 🌐 Switch to GPT-4o cloud (~$0.05 estimated cost)
   - ⏭️ Skip and just list the email subjects

Step 5: User clicks "Process emails in batches"
   Recovery executes:
   - Splits 10 emails into 3 batches
   - Processes each batch with local LLM
   - Combines results into final summary

Final response: Summary delivered in chunks
```

### Example 4: Network Failure During Workflow

```text
User: "Open Spotify and play workout playlist"

Step 1: Open Spotify ✓

Step 2: Search for "workout" playlist
   Failure: NetworkUnavailable { affected_service: "spotify" }

Step 3: Recovery options
   - 🔄 Retry in 30 seconds (network might recover)
   - 📂 Open offline playlists instead
   - 🎵 Use local music player (Rhythmbox?) instead
   - ⏭️ Cancel and try later

Step 4: User clicks "Retry in 30 seconds"
   - 30-second countdown
   - Re-checks network: connectivity restored
   - Retries the search step
   - Workflow continues
```

---

## Production Readiness Checklist

After implementing this entire spec, KRIA should pass these tests:

- [ ] User says "open Slack" without Slack installed → sees recovery options
- [ ] User says "send WhatsApp message" without app/login → sees recovery options
- [ ] User says "open VS Code" without VS Code → can use gedit alternative
- [ ] User says "play music on Spotify" without Spotify → can use Rhythmbox
- [ ] User says "join Zoom call" without Zoom → can join via browser
- [ ] Each recovery option is clearly labeled and previewable
- [ ] Each recovery executes the action and waits for completion signal
- [ ] After recovery, original workflow resumes automatically
- [ ] User preferences are remembered ("always use Brave for WhatsApp")
- [ ] Failed recovery shows next-level alternatives
- [ ] Cancellation cleanly stops recovery and returns to options
- [ ] Multi-step recoveries (install + login) work end-to-end
- [ ] All telemetry events are typed (no string parsing in frontend)
- [ ] User feedback (👍/👎 on outcomes) is collected
- [ ] Knowledge base is extensible via TOML

---

## Summary

This document specifies a complete production-grade GUI cognition recovery system that transforms KRIA from a tool dispatcher into an **intelligent collaborative assistant**.

**The key innovation:** every failure produces actionable buttons that resolve the failure and resume the original workflow.

**Implementation timeline:** 5 weeks across 6 phases.

**Outcome:** Users never see dead-end errors. Every failure becomes a clear path forward. The system feels like a competent assistant that understands what they want and figures out how to make it happen, even when things go wrong.

**Success metric:** When a user submits any reasonable prompt, KRIA either completes it or offers concrete recovery options that the user can click. Zero "I can't do this" with no recourse.

This is what production-grade GUI cognition looks like.
