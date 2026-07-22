import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import GuiCognitionPanel from "./GuiCognitionPanel";
import type { GuiCognitionSessionState } from "../types/guiCognition";

const baseSession: GuiCognitionSessionState = {
  lifecycle: "verifying",
  sessionId: "session-1",
  turnId: "turn-1",
  workflowId: "workflow-1",
  lastSequence: 9,
  observation: {
    activeWindow: "Kria",
    activeWindowSource: "get_active_window",
    activeWindowConfidence: 0.95,
    activeWindowReliability: "reliable",
    activeWindowAuthoritySource: "kria_gnome_shell_bridge",
    activeWindowAuthorityConfidence: 0.98,
    activeWindowAuthorityStatus: "available",
    gnomeBridgeStatus: "available",
    activeWindowFallbackChain: [{ source: "get_active_window", status: "matched", reliability: "reliable" }],
    activeWindowFailureChain: [],
    visibleControlCount: 5,
    visibleAccessibleControlCount: 4,
    disabledControlCount: 1,
    hiddenControlCount: 0,
    trustedControlCount: 4,
    partialControlCount: 0,
    notExecutableControlCount: 1,
    textFieldCount: 1,
    buttonCount: 2,
    otherControlCount: 2,
    ocrAvailable: true,
    ocrBlockCount: 2,
    ocrTrust: "untrusted",
    ocrWaitForScreenshotMs: 96,
    ocrEngineSelected: "tesseract_cli",
    ocrEngineStatus: "completed",
    ocrImageStatus: "downscaled_2560x1440_to_1600x900",
    ocrTotalMs: 310,
    ocrFastPath: "full_screen_tesseract",
    ocrCacheHit: false,
    ocrRoiCount: 1,
    ocrChangedRegionCount: 0,
    ocrInjectionCount: 0,
    accessibilityAvailable: true,
    accessibilitySourceStatus: "degraded",
    accessibilityOverallStatus: "degraded",
    accessibilityOverallConfidence: 0.73,
    accessibilityStaleNodeCount: 2,
    accessibilityTimeoutCount: 1,
    accessibilityNodeCount: 42,
    accessibilityControlCount: 5,
    atspiSnapshotTotalMs: 760,
    atspiSkippedAppCount: 1,
    atspiOmittedNodeCount: 24,
    accessibilityRemediation: ["Enable desktop accessibility"],
    screenshotAvailable: true,
    screenshotStatus: "available",
    screenshotCaptureMs: 96,
    screenshotDurationMs: 96,
    screenHashPrefix: "abcdef0123456789",
    monitorCount: 2,
    dpiAvailable: true,
    cursorFocusKnown: true,
    focusedWindow: "Kria",
    focusedApp: "Code",
    focusedControlLabel: "Search KRIA",
    focusedControlRole: "text",
    focusedControlBounds: { x: 12, y: 24, width: 180, height: 32 },
    editableTargetKnown: true,
    terminalLike: false,
    focusSource: "atspi_focused_object",
    focusConfidence: 0.88,
    focusReliability: "reliable",
    focusAdapterStatus: "available",
    focusLatencyMs: 33,
    visualControlCount: 3,
    visualButtonLikeCount: 2,
    visualMatchedCount: 2,
    visualUnmatchedCount: 1,
    observationTotalMs: 420,
    slowestProbe: "run_ocr",
    slowestProbeMs: 310,
    probeTimeoutCount: 0,
    probeTimings: [
      {
        probe_name: "run_ocr",
        duration_ms: 310,
        status: "ok",
        source: "ocr_image",
        cache_hit: false,
      },
    ],
    cacheHit: false,
    cachePolicy: "observe_plan_ttl_750ms",
    freshness: "fresh",
    sourceBlockers: ["ocr: low confidence block omitted"],
  },
  context: {
    contextId: "ctx-1",
    observationId: "obs-1",
    status: "ready",
    freshness: "fresh",
    screenHashPrefix: "abcdef0123456789",
    trustedControlCount: 5,
    executableControlCount: 4,
    disabledOrHiddenCount: 1,
    ocrUntrusted: true,
    ocrInjectionCount: 0,
    redactionCount: 1,
    sourceConfidence: {
      accessibility: 0.9,
      ocr: 0.45,
    },
    sourceTrust: {
      accessibility: "trusted_executable",
      ocr: "untrusted_text",
    },
    deltaSummary: ["screen_hash_changed"],
    sourceBlockers: ["ocr: low confidence block omitted"],
    warnings: ["Sensitive text was redacted before context use."],
  },
  goalContract: {
    contractId: "goal-1",
    observationId: "obs-1",
    contextId: "ctx-1",
    goalSummary: "Fill visible input",
    promptHash: "prompt-hash-123",
    intentKind: "type_text",
    actionType: "type_text",
    targetAppKind: "browser",
    targetAppHint: "Browser",
    targetWindowHint: "Kria",
    targetControlHint: "Search",
    querySummary: "KRIA docs",
    queryHash: "query-hash-123",
    textPayloadSummary: "hello world",
    textPayloadHash: "text-hash-123",
    desiredFinalState: "requested text is present",
    riskLevel: "medium",
    requiresUserApproval: false,
    ambiguityCount: 1,
    ambiguities: [
      {
        kind: "missing_text_payload",
        field: "typed_text",
        message: "Exact text must be provided before typing.",
      },
    ],
    sourceEvidence: [
      {
        source: "user_prompt",
        field: "action_type",
        summary: "type text",
        confidence: 0.9,
      },
      {
        source: "heuristic",
        field: "target_app_kind",
        summary: "browser",
        confidence: 0.86,
      },
    ],
    extractionConfidence: 0.77,
    extractorMode: "deterministic",
  },
  goalSummary: "Fill visible input",
  plannerMode: "llm_assisted",
  llmAttempted: true,
  llmStatus: "completed",
  plannerConfidence: 0.86,
  planValidationStatus: "valid",
  planReadinessStatus: "valid_for_resolution",
  planCanProceedToTargetResolution: true,
  planCanExecute: false,
  planValidationBlockerCount: 0,
  planValidationWarningCount: 1,
  planBlockedReasons: [],
  planWarnings: ["Untrusted OCR evidence was excluded from planner instructions."],
  planStepValidationResults: [
    {
      stepId: "det-1",
      stepType: "FocusField",
      status: "needs_target_resolution",
      riskLevel: "low",
      targetResolutionRequired: true,
      targetAvailable: true,
      verificationPresent: true,
    },
  ],
  planRequiresUserApproval: false,
  planSteps: ["Find input", "Type text", "Verify result"],
  typedPlanSteps: [
    {
      stepId: "det-1",
      stepType: "FocusField",
      summary: "Focus input",
      targetControlHint: "Search",
      expectedPostcondition: "field is focused",
      verificationStrategy: "focused_control",
      riskLevel: "low",
      requiresApproval: false,
      allowedToExecute: false,
      confidence: 0.86,
      reason: "type_text",
    },
  ],
  planSourceEvidence: [],
  riskLevel: "low",
  target: {
    label: "Search",
    role: "push button",
    confidence: 0.91,
  },
  safetyDecision: {
    status: "Allowed",
    riskLevel: "low",
    reasons: [],
  },
  actionBackend: {
    selectedBackend: "uinput_accessibility",
    haltKind: "none",
    sessionType: "wayland",
    automationEnabled: true,
    globalHaltEngaged: false,
    uinputAvailable: true,
    ydotoolAvailable: false,
    xdotoolAvailable: true,
    xdotoolUsableForActions: false,
    ydotoolUsableForActions: false,
    uinputSocketPath: "/run/user/1000/kria-uinput.sock",
    uinputSocketAccessible: true,
    backendProbeStatus: "wayland_uinput_ready",
    backendSelectionReason: "Wayland session selected uinput because the daemon and socket are healthy.",
    backendProbeErrors: ["xdotool detected but not usable for Wayland GUI actions"],
    inputBackendKind: "uinput",
    focusSupported: true,
    typingSupported: true,
    clickSupported: true,
    verificationSupported: true,
    canExecuteActions: true,
    canObserve: true,
    canPlan: true,
    blockers: [],
    releaseConditions: [],
    capabilities: {
      focus_field: true,
      fill_field: true,
      click_control: true,
    },
  },
  currentAction: {
    actionKind: "ClickControl",
    target: "Search",
    status: "running",
  },
  verification: {
    status: "completed",
    confidence: 0.77,
  },
  recoveryOptions: [],
};

describe("GuiCognitionPanel", () => {
  afterEach(() => cleanup());

  it("renders observation, plan, target, safety, execution, and verification", () => {
    render(() => <GuiCognitionPanel session={baseSession} />);

    expect(screen.getByText("GUI Cognition")).toBeInTheDocument();
    expect(screen.getByText("Running")).toBeInTheDocument();
    expect(screen.getByText(/Active window: Kria/)).toBeInTheDocument();
    expect(screen.getByText(/get_active_window/)).toBeInTheDocument();
    expect(screen.getAllByText(/reliable/).length).toBeGreaterThan(0);
    expect(screen.getByText("Screen observed with degraded sources")).toBeInTheDocument();
    expect(screen.getByText(/Authority KRIA Gnome Shell Bridge/)).toBeInTheDocument();
    expect(screen.getByText(/GNOME bridge available/)).toBeInTheDocument();
    expect(screen.getByText("Controls 5")).toBeInTheDocument();
    expect(screen.getByText("Inputs 1")).toBeInTheDocument();
    expect(screen.getByText("Buttons 2")).toBeInTheDocument();
    expect(screen.getByText("Other 2")).toBeInTheDocument();
    expect(screen.getByText(/Screenshot available/)).toBeInTheDocument();
    expect(screen.getByText(/OCR available/)).toBeInTheDocument();
    expect(screen.getAllByText(/untrusted/).length).toBeGreaterThan(0);
    expect(screen.getByText(/OCR engine tesseract_cli/)).toBeInTheDocument();
    expect(screen.getByText(/downscaled_2560x1440_to_1600x900/)).toBeInTheDocument();
    expect(screen.getByText(/waited 96ms/)).toBeInTheDocument();
    expect(screen.getByText(/total 310ms/)).toBeInTheDocument();
    expect(screen.getByText(/OCR fast path full_screen_tesseract/)).toBeInTheDocument();
    expect(screen.getAllByText(/cache miss/i).length).toBeGreaterThan(0);
    expect(screen.getByText(/ROI 1/)).toBeInTheDocument();
    expect(screen.getAllByText(/injections 0/).length).toBeGreaterThan(0);
    expect(screen.getByText(/42 nodes/)).toBeInTheDocument();
    expect(screen.getByText(/Quality trusted 4/)).toBeInTheDocument();
    expect(screen.getByText(/not executable 1/)).toBeInTheDocument();
    expect(screen.getByText(/Monitors 2/)).toBeInTheDocument();
    expect(screen.getByText(/Observation 420ms/)).toBeInTheDocument();
    expect(screen.getByText(/Screenshot 96ms/)).toBeInTheDocument();
    expect(screen.getByText(/Slowest probe: run_ocr 310ms/)).toBeInTheDocument();
    expect(screen.getByText(/Cache miss/)).toBeInTheDocument();
    expect(screen.getByText(/AT-SPI degraded/)).toBeInTheDocument();
    expect(screen.getByText(/Accessibility health degraded/)).toBeInTheDocument();
    expect(screen.getByText(/stale nodes 2/)).toBeInTheDocument();
    expect(screen.getByText(/timeouts 1/)).toBeInTheDocument();
    expect(screen.getByText(/Visual controls 3/)).toBeInTheDocument();
    expect(screen.getByText(/button-like 2/)).toBeInTheDocument();
    expect(screen.getByText(/matched 2/)).toBeInTheDocument();
    expect(screen.getByText(/unmatched 1/)).toBeInTheDocument();
    expect(screen.getByText(/snapshot 760ms/)).toBeInTheDocument();
    expect(screen.getByText(/skipped apps 1/)).toBeInTheDocument();
    expect(screen.getByText(/omitted nodes 24/)).toBeInTheDocument();
    expect(screen.getByText(/Enable desktop accessibility/)).toBeInTheDocument();
    expect(screen.getByText(/Disabled\/hidden controls 1/)).toBeInTheDocument();
    expect(screen.getByText(/Focus source Atspi Focused Object/)).toBeInTheDocument();
    expect(screen.getAllByText(/available/).length).toBeGreaterThan(0);
    expect(screen.getByText(/33ms/)).toBeInTheDocument();
    expect(screen.getByText(/editable known/)).toBeInTheDocument();
    expect(screen.getByText(/Focused app Code/)).toBeInTheDocument();
    expect(screen.getByText(/Control Search KRIA/)).toBeInTheDocument();
    expect(screen.getByText(/Screen hash abcdef0123456789/)).toBeInTheDocument();
    expect(screen.getByText("Context")).toBeInTheDocument();
    expect(screen.getByText(/ready · fresh/)).toBeInTheDocument();
    expect(screen.getByText("Trusted 5")).toBeInTheDocument();
    expect(screen.getByText("Executable 4")).toBeInTheDocument();
    expect(screen.getByText(/OCR untrusted/)).toBeInTheDocument();
    expect(screen.getByText(/redactions 1/)).toBeInTheDocument();
    expect(screen.getByText(/Changed since previous: screen_hash_changed/)).toBeInTheDocument();
    expect(screen.getByText("Fill visible input")).toBeInTheDocument();
    expect(screen.getByText(/Action type_text/)).toBeInTheDocument();
    expect(screen.getByText(/Final state requested text is present/)).toBeInTheDocument();
    expect(screen.getByText(/kind browser/)).toBeInTheDocument();
    expect(screen.getByText(/app Browser/)).toBeInTheDocument();
    expect(screen.getAllByText(/window Kria/).length).toBeGreaterThan(0);
    expect(screen.getByText(/control Search/)).toBeInTheDocument();
    expect(screen.getByText(/Query KRIA docs/)).toBeInTheDocument();
    expect(screen.getByText(/Text hello world/)).toBeInTheDocument();
    expect(screen.getByText(/Prompt hash prompt-hash-123/)).toBeInTheDocument();
    expect(screen.getByText(/query hash query-hash-123/)).toBeInTheDocument();
    expect(screen.getByText(/text hash text-hash-123/)).toBeInTheDocument();
    expect(screen.getByText(/Goal confidence 77%/)).toBeInTheDocument();
    expect(screen.getByText(/Evidence user_prompt: action_type/)).toBeInTheDocument();
    expect(screen.getByText(/type text/)).toBeInTheDocument();
    expect(screen.getByText(/Evidence heuristic: target_app_kind/)).toBeInTheDocument();
    expect(screen.getByText(/LLM completed/)).toBeInTheDocument();
    expect(screen.getByText(/Plan confidence 86%/)).toBeInTheDocument();
    expect(screen.getByText(/Validation valid/)).toBeInTheDocument();
    expect(screen.getByText(/Readiness valid_for_resolution/)).toBeInTheDocument();
    expect(screen.getByText(/ready for target resolution/)).toBeInTheDocument();
    expect(screen.getAllByText(/execution disabled/).length).toBeGreaterThan(0);
    expect(screen.getByText(/Validation blockers 0/)).toBeInTheDocument();
    expect(screen.getByText("Untrusted OCR evidence was excluded from planner instructions.")).toBeInTheDocument();
    expect(screen.getByText("Exact text must be provided before typing.")).toBeInTheDocument();
    expect(screen.getByText("Find input")).toBeInTheDocument();
    expect(screen.getByText(/FocusField: Focus input/)).toBeInTheDocument();
    expect(screen.getByText(/verify focused_control/)).toBeInTheDocument();
    expect(screen.getByText(/plan only/)).toBeInTheDocument();
    expect(screen.getByText(/FocusField validation: needs_target_resolution/)).toBeInTheDocument();
    expect(screen.getByText(/target resolution required/)).toBeInTheDocument();
    expect(screen.getAllByText("Search").length).toBeGreaterThan(0);
    expect(screen.getByText(/Confidence 91%/)).toBeInTheDocument();
    expect(screen.getByText("Allowed")).toBeInTheDocument();
    expect(screen.getByText("Action backend")).toBeInTheDocument();
    expect(screen.getByText(/ready · uinput_accessibility/)).toBeInTheDocument();
    expect(screen.getByText(/Session wayland/)).toBeInTheDocument();
    expect(screen.getByText(/Probe wayland_uinput_ready/)).toBeInTheDocument();
    expect(screen.getByText(/xdotool detected but not usable for Wayland actions/)).toBeInTheDocument();
    expect(screen.getByText(/uinput socket available/)).toBeInTheDocument();
    expect(screen.getByText(/Capabilities focus available/)).toBeInTheDocument();
    expect(screen.getByText(/ClickControl/)).toBeInTheDocument();
    expect(screen.getByText(/Verification completed/)).toBeInTheDocument();
  });

  it("renders Step 8 verified post-action verification with strategy and state summary", () => {
    const session = {
      ...baseSession,
      verification: {
        status: "verified",
        confidence: 0.9,
        verificationStrategy: "result_visible",
        matchedExpectedState: true,
        postStateSummary: "app=KRIA; controls=3; dialogs=0; focus_role=none; screen=abcd1234",
      },
    };
    render(() => <GuiCognitionPanel session={session} />);
    expect(screen.getByText(/Verification verified/)).toBeInTheDocument();
    expect(screen.getByText(/result_visible/)).toBeInTheDocument();
    expect(screen.getByText(/state: app=KRIA/)).toBeInTheDocument();
  });

  it("renders Step 8 verification failure with a safe recovery hint and no blind success", () => {
    const session = {
      ...baseSession,
      verification: {
        status: "verification_failed",
        confidence: 0.2,
        verificationStrategy: "result_visible",
        matchedExpectedState: false,
        safeErrorSummary: "Expected post-action state was not verified for ClickControl.",
        recoveryHint: "Re-observe and confirm the expected state before retrying; do not blind-retry.",
      },
    };
    render(() => <GuiCognitionPanel session={session} />);
    expect(screen.getByText(/Verification verification_failed/)).toBeInTheDocument();
    expect(
      screen.getByText(/Expected post-action state was not verified for ClickControl./)
    ).toBeInTheDocument();
    expect(screen.getByText(/do not blind-retry/)).toBeInTheDocument();
  });

  it("renders Step 11 checkpoint status, hash prefix, and resume state", () => {
    const session = {
      ...baseSession,
      checkpoint: {
        checkpointId: "checkpoint-1",
        checkpointHashPrefix: "abc123def456",
        currentStepIndex: 1,
        stepCount: 2,
        completedStepCount: 1,
        pendingStepId: "s1",
        resumeStatus: "resumed",
        nextStepId: "s1",
      },
    };
    render(() => <GuiCognitionPanel session={session} />);
    expect(screen.getByText(/abc123def456/)).toBeInTheDocument();
    expect(screen.getByText(/1 completed/)).toBeInTheDocument();
  });

  it("renders a Step 11 duplicate-action-blocked resume guard", () => {
    const session = {
      ...baseSession,
      checkpoint: {
        checkpointId: "checkpoint-2",
        resumeStatus: "duplicate_action_blocked",
        duplicateActionGuards: ["external_submit already completed"],
        resumeExplanation: "This risky step already completed once; KRIA will not repeat it.",
      },
    };
    render(() => <GuiCognitionPanel session={session} />);
    expect(screen.getByText(/duplicate blocked: external_submit already completed/)).toBeInTheDocument();
  });

  it("renders Step 10 workflow run status and per-step list", () => {
    const session = {
      ...baseSession,
      workflow: {
        workflowRunId: "run-1",
        status: "completed",
        currentStepIndex: 1,
        stepCount: 2,
        completedStepCount: 2,
        steps: [
          { stepIndex: 0, stepType: "OpenApp", status: "completed" },
          { stepIndex: 1, stepType: "FocusField", status: "completed" },
        ],
      },
    };
    render(() => <GuiCognitionPanel session={session} />);
    expect(screen.getByText(/2 done/)).toBeInTheDocument();
    expect(screen.getByText(/OpenApp: completed/)).toBeInTheDocument();
    expect(screen.getByText(/FocusField: completed/)).toBeInTheDocument();
  });

  it("renders a blocked Step 10 workflow step reason", () => {
    const session = {
      ...baseSession,
      workflow: {
        workflowRunId: "run-2",
        status: "blocked",
        currentStepIndex: 0,
        stepCount: 2,
        completedStepCount: 0,
        blockedReason: "the resolved target is no longer present",
        steps: [
          {
            stepIndex: 0,
            stepType: "ClickControl",
            status: "blocked",
            blockers: ["the resolved target is no longer present"],
          },
        ],
      },
    };
    render(() => <GuiCognitionPanel session={session} />);
    expect(screen.getByText(/ClickControl: blocked/)).toBeInTheDocument();
    expect(
      screen.getAllByText(/the resolved target is no longer present/).length
    ).toBeGreaterThan(0);
  });

  it("renders Step 9 recovery status, failure kind, action, and next state", () => {
    const session = {
      ...baseSession,
      recovery: {
        status: "recovered",
        failureKind: "focus_lost",
        recoveryActionKind: "RefocusSameTarget",
        retryCount: 1,
        maxRetryCount: 1,
        safeExplanation: "Focus moved away, KRIA re-focused the same field once.",
        nextRecommendedState: "retry_original_action",
        canRetryOriginalAction: true,
        canContinueWorkflow: false,
      },
    };
    render(() => <GuiCognitionPanel session={session} />);
    expect(screen.getByText(/focus_lost/)).toBeInTheDocument();
    expect(screen.getByText(/RefocusSameTarget/)).toBeInTheDocument();
    expect(screen.getByText(/next: retry_original_action/)).toBeInTheDocument();
  });

  it("renders Step 9 blocked recovery with blockers for risky actions", () => {
    const session = {
      ...baseSession,
      recovery: {
        status: "blocked",
        failureKind: "unsafe_to_retry",
        recoveryActionKind: "Stop",
        canExecuteRecovery: false,
        requiresUserApproval: true,
        blockers: ["risky/high-impact action is never auto-recovered"],
        safeExplanation: "This is a risky action, so KRIA stops.",
      },
    };
    render(() => <GuiCognitionPanel session={session} />);
    expect(screen.getByText(/unsafe_to_retry/)).toBeInTheDocument();
    expect(
      screen.getByText(/risky\/high-impact action is never auto-recovered/)
    ).toBeInTheDocument();
  });

  it("renders Step 5 target resolution details", () => {
    render(() => (
      <GuiCognitionPanel
        session={{
          ...baseSession,
          target: {
            label: "Search",
            role: "push button",
            targetType: "button",
            controlId: "control-search",
            targetHash: "target-hash-search",
            bounds: { x: 10, y: 20, width: 120, height: 32 },
            confidence: 0.91,
          },
          targetResolution: {
            resolutionId: "resolution-1",
            planId: "plan-1",
            validationId: "validation-1",
            status: "resolved",
            confidence: 0.91,
            candidateCount: 1,
            candidates: [
              {
                candidateId: "candidate-1",
                controlId: "control-search",
                label: "Search",
                role: "push button",
                sources: ["accessibility"],
                confidence: 0.91,
              },
            ],
            ambiguityReasons: [],
            blockers: [],
            canProceedToSafetyGate: true,
            canExecute: false,
          },
        }}
      />
    ));

    expect(screen.getByText("Target Resolution")).toBeInTheDocument();
    expect(screen.getByText("resolved")).toBeInTheDocument();
    expect(screen.getByText(/ready for safety gate/)).toBeInTheDocument();
    expect(screen.getAllByText(/execution disabled/).length).toBeGreaterThan(0);
    expect(screen.getByText(/ID control-search/)).toBeInTheDocument();
    expect(screen.getByText(/Hash target-hash/)).toBeInTheDocument();
    expect(screen.getByText(/Bounds 10,20 120x32/)).toBeInTheDocument();
    expect(screen.getByText(/Candidates 1/)).toBeInTheDocument();
  });

  it("renders blocker and recovery clearly", () => {
    const onDismiss = vi.fn();
    render(() => (
      <GuiCognitionPanel
        onDismiss={onDismiss}
        session={{
          ...baseSession,
          lifecycle: "blocked",
          blocker: {
            type: "target",
            reason: "No matching accessible button/control was found.",
            options: ["Search"],
          },
          recoveryOptions: ["Re-observe screen"],
        }}
      />
    ));

    expect(screen.getAllByText("Blocked").length).toBeGreaterThan(0);
    expect(screen.getByText("No matching accessible button/control was found.")).toBeInTheDocument();
    expect(screen.getAllByText("Search").length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: /dismiss/i })).toBeInTheDocument();
  });

  it("renders paused approval state and hides dismiss while non-terminal", () => {
    render(() => (
      <GuiCognitionPanel
        onDismiss={vi.fn()}
        session={{
          ...baseSession,
          lifecycle: "awaiting_approval",
          pendingApproval: {
            reason: "Submit needs approval",
            riskLevel: "high",
          },
        }}
      />
    ));

    expect(screen.getByText("Paused for approval")).toBeInTheDocument();
    expect(screen.getByText("Submit needs approval")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /dismiss/i })).not.toBeInTheDocument();
  });

  it("renders deterministic fallback and validation blockers clearly", () => {
    render(() => (
      <GuiCognitionPanel
        session={{
          ...baseSession,
          lifecycle: "planning",
          plannerMode: "deterministic_fallback",
          llmAttempted: true,
          llmStatus: "rejected",
          llmFailureReason: "LLM planner output was rejected; deterministic fallback used.",
          plannerConfidence: 0.62,
          planValidationStatus: "blocked",
          planBlockedReasons: ["LLM plan step target is not supported by current context."],
          planWarnings: ["Risky LLM step was ignored."],
          planSteps: ["Fallback safe observation"],
        }}
      />
    ));

    expect(screen.getByText(/Planner deterministic_fallback/)).toBeInTheDocument();
    expect(screen.getByText(/LLM rejected/)).toBeInTheDocument();
    expect(screen.getByText(/Plan confidence 62%/)).toBeInTheDocument();
    expect(screen.getByText(/Validation blocked/)).toBeInTheDocument();
    expect(screen.getByText("LLM planner output was rejected; deterministic fallback used.")).toBeInTheDocument();
    expect(screen.getByText("LLM plan step target is not supported by current context.")).toBeInTheDocument();
    expect(screen.getByText("Risky LLM step was ignored.")).toBeInTheDocument();
  });

  it("renders failed state and terminal dismiss", () => {
    render(() => (
      <GuiCognitionPanel
        onDismiss={vi.fn()}
        session={{
          ...baseSession,
          lifecycle: "failed",
          blocker: {
            type: "turn",
            reason: "Observation failed",
            options: [],
          },
        }}
      />
    ));

    expect(screen.getAllByText("Failed").length).toBeGreaterThan(0);
    expect(screen.getByText("Observation failed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /dismiss/i })).toBeInTheDocument();
  });

  it("renders terminal-like focus warning", () => {
    render(() => (
      <GuiCognitionPanel
        session={{
          ...baseSession,
          observation: {
            ...baseSession.observation,
            focusSource: "gnome_terminal_adapter",
            focusAdapterStatus: "available",
            focusedApp: "GNOME Terminal",
            focusedControlLabel: "Terminal focus",
            focusedControlRole: "terminal",
            editableTargetKnown: false,
            terminalLike: true,
          },
        }}
      />
    ));

    expect(screen.getByText(/Focus source Gnome Terminal Adapter/)).toBeInTheDocument();
    expect(screen.getByText(/Terminal-like focus detected/)).toBeInTheDocument();
  });

  it("renders action backend blocker and global halt reason", () => {
    render(() => (
      <GuiCognitionPanel
        session={{
          ...baseSession,
          lifecycle: "blocked",
          actionBackend: {
            selectedBackend: "blocked_global_halt",
            haltKind: "startup_warming",
            sessionType: "wayland",
            automationEnabled: false,
            globalHaltEngaged: true,
            haltReason: "orchestrator startup",
            releaseConditions: ["Wait for vision sidecar and uinput daemon to report running."],
            backendProbeErrors: ["xdotool detected but not usable for Wayland GUI actions"],
            backendProbeStatus: "global_halt_blocked",
            backendSelectionReason: "service warming up (vision=starting, uinput=starting)",
            uinputAvailable: false,
            ydotoolAvailable: false,
            xdotoolAvailable: true,
            xdotoolUsableForActions: false,
            ydotoolUsableForActions: false,
            uinputSocketAccessible: false,
            canExecuteActions: false,
            canObserve: true,
            canPlan: true,
            blockers: ["global safety halt is engaged"],
            capabilities: {
              focus_field: false,
              fill_field: false,
              click_control: false,
            },
          },
          blocker: {
            type: "execution",
            reason: "orchestrator startup",
            options: ["global safety halt is engaged"],
          },
        }}
      />
    ));

    expect(screen.getByText(/warming up · blocked_global_halt/)).toBeInTheDocument();
    expect(screen.getByText(/Global safety halt active: orchestrator startup/)).toBeInTheDocument();
    expect(screen.getByText(/Wait for vision sidecar and uinput daemon/)).toBeInTheDocument();
    expect(screen.getAllByText("global safety halt is engaged").length).toBeGreaterThan(0);
    expect(screen.getByText(/Capabilities focus unavailable/)).toBeInTheDocument();
  });

  it("does not render hidden/raw sensitive fields", () => {
    render(() => (
      <GuiCognitionPanel
        session={{
          ...baseSession,
          planSteps: ["Use token=[redacted]"],
        } as GuiCognitionSessionState & { raw_ocr_text?: string; hidden_prompt?: string }}
      />
    ));

    expect(screen.queryByText(/raw_ocr_text/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/hidden_prompt/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/abc123/)).not.toBeInTheDocument();
  });

  it("shows a Stop control while the turn is active and invokes onStop", () => {
    const onStop = vi.fn();
    render(() => (
      <GuiCognitionPanel
        onStop={onStop}
        session={{ ...baseSession, lifecycle: "executing" }}
      />
    ));

    const stop = screen.getByRole("button", { name: /stop gui cognition/i });
    expect(stop).toBeInTheDocument();
    fireEvent.click(stop);
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it("hides the Stop control once the turn reaches a terminal state", () => {
    render(() => (
      <GuiCognitionPanel
        onStop={vi.fn()}
        session={{ ...baseSession, lifecycle: "completed" }}
      />
    ));

    expect(
      screen.queryByRole("button", { name: /stop gui cognition/i })
    ).not.toBeInTheDocument();
  });

  it("renders a clear cancelled state", () => {
    render(() => (
      <GuiCognitionPanel
        onStop={vi.fn()}
        session={{
          ...baseSession,
          lifecycle: "cancelled",
          blocker: { type: "turn", reason: "Turn cancelled by you.", options: [] },
        }}
      />
    ));

    expect(screen.getAllByText("Cancelled").length).toBeGreaterThan(0);
    expect(screen.getByText(/Cancelled — Turn cancelled by you\./)).toBeInTheDocument();
    // Cancelled is terminal → the Stop control is no longer offered.
    expect(
      screen.queryByRole("button", { name: /stop gui cognition/i })
    ).not.toBeInTheDocument();
  });

  // Task 10.4 / Requirement 16.4-16.5: layered output.
  describe("layered output (layman summary + collapsible developer detail)", () => {
    it("renders a layman summary layer above a collapsible developer detail layer", () => {
      const { container } = render(() => <GuiCognitionPanel session={baseSession} />);

      // Layman layer present.
      const summary = container.querySelector(".gui-cognition-summary");
      expect(summary).toBeTruthy();
      expect(summary?.querySelector(".gui-cognition-summary-headline")?.textContent).toBeTruthy();

      // Developer detail layer is a <details> collapsed by default.
      const details = container.querySelector<HTMLDetailsElement>("details.gui-cognition-details");
      expect(details).toBeTruthy();
      expect(details?.open).toBe(false);
      expect(details?.querySelector("summary")?.textContent).toMatch(/developer detail/i);
    });

    it("expands the developer detail layer to reveal the full technical envelope", () => {
      const { container } = render(() => <GuiCognitionPanel session={baseSession} />);
      const details = container.querySelector<HTMLDetailsElement>("details.gui-cognition-details");
      expect(details?.open).toBe(false);

      // Expand the collapsible developer layer.
      const summaryEl = details!.querySelector("summary")!;
      details!.open = true;
      fireEvent.click(summaryEl);

      // The full technical detail (raw IDs/hashes/coordinates) lives ONLY here.
      const detailRegion = details!.querySelector(".gui-cognition-detail-region");
      expect(detailRegion?.textContent).toContain("GUI Cognition");
      expect(detailRegion?.textContent).toContain("Prompt hash prompt-hash-123");
      expect(detailRegion?.textContent).toMatch(/Screen hash abcdef0123456789/);
    });

    it("never leaks hashes, internal IDs, coordinates, or secrets into the layman layer", () => {
      const { container } = render(() => (
        <GuiCognitionPanel
          session={{
            ...baseSession,
            lifecycle: "blocked",
            blocker: {
              type: "execution",
              // Deliberately leaky upstream reason: id + coordinate + hash + secret.
              reason:
                "control-search at 12,24 failed (prompt-hash-abcdef0123456789, token=sk-secret-value)",
              options: [],
            },
          }}
        />
      ));

      const laymanText = container.querySelector(".gui-cognition-summary")?.textContent ?? "";
      expect(laymanText).toContain("Stopped safely");
      // Privacy guarantees on the layman layer:
      expect(laymanText).not.toMatch(/[0-9a-f]{12,}/i); // hex digest / hash
      expect(laymanText).not.toMatch(
        /\b(control|prompt|turn|session|workflow|resolution)[-_][a-z0-9]/i,
      ); // internal id token
      expect(laymanText).not.toMatch(/\b\d{2,4}\s*,\s*\d{2,4}\b/); // coordinates
      expect(laymanText).not.toMatch(/token\s*=\s*sk-/i); // secret value
      expect(laymanText).toContain("[redacted]");
    });
  });
});
