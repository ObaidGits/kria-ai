# RFC 007: GUI System Control Architecture

**Status:** Draft  
**Author:** Obaid Gits  
**Date:** 2026-05-11  
**Classification:** Architectural Specification  

---

## Executive Summary

This document establishes the architectural master plan for transitioning KRIA from a tool-calling assistant to a Host-Level GUI Agent capable of native laptop control and eventual remote access. The specification enforces strict safety boundaries, VRAM-aware resource management, and cognitive separation between planning and execution.

---

## Section 1: Objective and Risk Model

### 1.1 Objective Definition

The primary objective is to enable KRIA to perform host-level GUI control operations including mouse interaction, keyboard input, and screen element detection. This capability extends to remote access scenarios where KRIA operates on enrolled target machines through the existing Fleet infrastructure.

Key capabilities to enable:

- Atomic GUI actions: mouse clicks, keyboard input, and shortcut combinations
- Screen comprehension: structured parsing of UI elements into actionable data
- Task automation: multi-step GUI workflows with verification and rollback
- Remote execution: secure GUI control over enrolled SSH targets

### 1.2 Risk Definition

The fundamental risk is unsupervised hallucination loops causing OS-level destruction. An LLM-based agent with GUI control can:

- Execute destructive actions through misinterpreted UI elements
- Enter infinite loops clicking incorrect coordinates
- Type sensitive data into wrong fields causing data leakage
- Cascade failures across multiple GUI operations
- Initiate irreversible system changes through administrative interfaces

The severity is elevated because GUI actions are immediate and lack the natural delay of API-based tool execution.

### 1.3 Core Mitigation Strategy

The architecture enforces two non-negotiable safeguards:

**Cognitive Separation of Planning and Execution**

The TurnGate retains exclusive authority over task decomposition. The AgentLoop executes only pre-approved sub-goals with no authority to generate new steps. This separation prevents the execution layer from hallucinating additional actions.

**Hardcoded Kill Switch**

A global asynchronous kill switch interceptor must be implemented at the AgentLoop boundary. This interceptor:

- Listens for kill signals through multiple channels: keyboard shortcut, UI button, and API endpoint
- Immediately terminates all pending GUI operations using cancellation tokens
- Forces agent state reset to idle
- Logs termination events to the audit trail with full context

The kill switch operates independently of the LLM reasoning path and cannot be bypassed by the agent.

---

## Section 2: Phase 1 - Mechanical Bridge and Immune System

### 2.1 File Structure

Create the following module:

File: crates/kria-core/src/tools/gui_automation.rs

This module contains all atomic GUI control primitives. It implements the ToolHandler trait for each GUI operation and integrates with the existing ToolRegistry.

### 2.2 Atomic Action Tools

The module implements three foundational tools using a backend abstraction layer that prioritizes ydotool (kernel-level uinput) for modern Linux Wayland compatibility. The xdotool backend is explicitly deprecated due to Wayland security blocking global input and is retained only as a legacy fallback for X11-only environments.

Privilege Isolation for ydotool:

Because ydotool requires elevated uinput kernel access, the GUI injector must run as a minimal, isolated helper process with strictly constrained permissions, rather than elevating the entire KRIA core. This helper process communicates via IPC and has no access to the main memory space or LLM inference paths.

**Tool: click_mouse**

Parameters:
- x: integer - horizontal screen coordinate
- y: integer - vertical screen coordinate
- button: string - values: left, right, middle

Behavior:
- Executes immediate mouse click at specified coordinates
- Returns success confirmation or error with coordinates
- Fails if coordinates exceed screen boundaries

**Tool: type_text**

Parameters:
- text: string - characters to input
- interval_ms: integer - optional delay between keystrokes

Behavior:
- Simulates keyboard input of specified text
- Supports Unicode characters
- Reports character count and completion status

**Tool: press_shortcut**

Parameters:
- keys: array of strings - modifier and key combination
- hold_duration_ms: integer - optional duration to hold keys

Behavior:
- Executes key combinations like control-s or alt-tab
- Releases all keys in reverse order
- Validates key names against allowed set

### 2.3 Global Kill Switch Interceptor

Implement a KillSwitchInterceptor struct that wraps all GUI tool execution:

Responsibilities:
- Maintain a cross-platform kill signal listener
- Inject cancellation token checks before every atomic action
- Force immediate termination on signal receipt
- Prevent partial execution of multi-step operations
- Enforce hardware-level rate limiting to prevent event loop starvation

Implementation Requirements:
- Register with the AgentLoop as a pre-execution middleware
- Use tokio cancellation tokens for async termination
- Maintain separate kill channels: mpsc for code signals, hotkey listener for user input
- Log all kill events to the audit system with timestamp and triggering context
- Enforce hard rate limit: maximum 2 actions per second with minimum 500ms inter-action delay
- Guarantee kill switch thread always has CPU cycles by throttling LLM-driven input spam
- Modifier Key OS Lockup Prevention: teardown sequence MUST explicitly broadcast global Key Up events for all modifier keys (Shift, Ctrl, Alt, Super) upon any signal termination to prevent permanently locking the user's OS keyboard state
- Realistic Kill-Switch SLA: best-effort sub-250ms interruption between atomic actions (accounting for OS scheduler pressure)
- Human-Interrupt Detection: execution loop must pause or safely abort if human mouse movement or keyboard activity is detected during an agent automation sequence

### 2.4 PolicyEngine Updates

All GUI automation tools receive RiskLevel classification:

**click_mouse: RED tier**

Rationale: Direct manipulation of UI elements can trigger destructive actions. Misclicks may delete files, close applications, or modify system settings.

HITL Requirements:
- Require PIN confirmation for every click
- Display target coordinates and intended action
- Enforce 30-second approval timeout

**type_text: RED tier**

Rationale: Keyboard input can modify data, submit forms, or execute commands. Hallucinated text may cause data corruption.

HITL Requirements:
- Require PIN for all type_text invocations
- Display full text preview before execution
- Enforce Active Window Verification: confirm target window title/class matches expected context before injecting any input
- Enforce Protected Mode: hard-block automation if active window is a password manager, banking site, or system authentication dialog
- Enforce Control Character Blacklist: strict blocking of shell operators (newline, pipe, greater-than, ampersand) unless explicit HITL approval is granted, especially when active window is identified as a terminal
- Implement Clipboard Isolation: atomic typing tools must either avoid clipboard reliance entirely, or explicitly snapshot clipboard state before use, utilize clipboard for input, and restore original clipboard state immediately after to prevent secret leakage
- Clipboard Atomic Backup: clipboard snapshot must be committed to a persistent, atomic backup variable held by the main thread (outside the agent's async task scope) before modification, ensuring safe restoration during panic unwinding if the agent is killed mid-typing
- Protected Mode Specifics: detection heuristics use an explicit allowlist/blocklist matching active window titles and browser URLs (for example: blocking "KeePass", "1Password", "chase.com", "wellsfargo.com", "sudo" dialogs), rather than relying on unreliable generic UI detection patterns

**press_shortcut: RED tier**

Rationale: Shortcuts can trigger system commands, close windows, or modify state. Dangerous combinations like control-alt-delete must be blacklisted.

HITL Requirements:
- Require PIN for all shortcuts
- Maintain blacklist of system-critical combinations
- Log shortcut execution with context

### 2.5 Safety Invariants

The following invariants apply to Phase 1:

1. No GUI tool executes without active cancellation token registration
2. Kill switch provides best-effort sub-250ms interruption between atomic actions
3. All GUI coordinates are validated against current screen dimensions
4. Text input is sanitized to prevent injection of control characters
5. Blacklist patterns are enforced at the PolicyEngine layer
6. Audit logs capture pre-action intent and post-action result

---

## Section 3: Phase 2 - OmniParser Vision Bridge

### 3.1 VRAM Constraint Strategy

The architecture explicitly rejects generic Vision Language Models for screen understanding. Models like GPT-4V or Qwen-VL consume excessive VRAM and introduce hallucination risks through open-ended interpretation.

Constraint Requirements:

- Maximum VRAM allocation for screen parsing: 2GB
- Target inference time: under 500ms per screenshot
- No dependency on cloud-based vision APIs
- Structured output only, no free-form text generation

### 3.2 OmniParser Integration

OmniParser converts raw UI screenshots into structured JSON representations containing bounding boxes and element types. This approach eliminates LLM hallucination by providing deterministic screen parsing.

Integration Architecture:

- Deploy OmniParser as a sidecar service alongside the existing Python sidecar
- Accept PNG screenshot bytes through IPC channel
- Return structured JSON with element annotations
- Cache parsed results for 5 seconds to reduce redundant processing

OmniParser Output Schema:

```json
{
  "elements": [
    {
      "id": "element_001",
      "type": "button",
      "label": "Save",
      "label_wrapped": "<evidence>Save</evidence>",
      "bbox": [120, 340, 200, 380],
      "confidence": 0.94,
      "monitor_id": 0,
      "dpi_scale": 1.0
    }
  ],
  "screen_dimensions": [1920, 1080],
  "monitor_dimensions": [[1920, 1080]],
  "timestamp": 1715432100,
  "visual_hash": "a3f7c2d9"
}
```

Visual Prompt Injection Defense:

All text extracted by OmniParser must be wrapped in EvidenceWrapper XML boundaries (<evidence>text</evidence>) per v3 specification. The LLM prompt must strictly treat parsed OCR text as untrusted string data to prevent malicious UI text (for example: "Ignore constraints") from hijacking the agent. The evidence wrapper serves as a semantic container that the LLM is trained to recognize as potentially adversarial.

Cognitive Poisoning Defense:

Small models may still obey OCR text even inside XML wrappers. The Prompt Injection Defense must aggressively truncate extracted OCR text to minimal length (maximum 100 characters) and place it at the absolute lowest trust tier in the prompt hierarchy. The LLM system prompt must explicitly state that evidence-wrapped text is potentially adversarial and should never override safety constraints.

### 3.3 Vision Tools

**Tool: get_screen_elements**

Parameters:
- filter_type: string - optional filter for element types
- min_confidence: float - minimum confidence threshold

Behavior:
- Captures current screenshot
- Routes through OmniParser sidecar
- Returns structured element list with bounding boxes
- Fails gracefully if parsing exceeds timeout

**Tool: click_element**

Parameters:
- element_id: string - ID from get_screen_elements output
- button: string - left, right, middle

Behavior:
- Retrieves cached element coordinates by ID
- Performs Visual Hash Verification: immediately before executing click, capture 50x50 micro-screenshot of target coordinates
- Compare micro-screenshot to original OmniParser crop using Perceptual Hashing (pHash) or Structural Similarity Index (SSIM) with similarity threshold greater than 0.90
- Abort click if pHash/SSIM similarity is less than 0.90, indicating UI has shifted
- Translates to center of bounding box only after hash verification passes
- Invokes click_mouse with calculated coordinates
- Validates element still exists through fresh screenshot on miss

Cache Invalidation:

The 5-second OmniParser cache must be instantly invalidated the moment any state-changing action (click or type) occurs. This prevents stale element references from being used after UI updates.

### 3.4 Resource Management

OmniParser execution requires GPU lease management:

- Request GpuLease before inference
- Release lease immediately after JSON generation
- Fallback to CPU-only parsing if lease unavailable
- Degrade to coordinate-only mode if parsing fails

### 3.5 Safety Invariants

Phase 2 invariants:

1. Bounding box coordinates are validated against current screen resolution
2. Element IDs expire after 10 seconds to prevent stale clicks
3. Confidence threshold minimum is 0.8 for automatic execution
4. Low-confidence elements require explicit coordinate confirmation
5. OmniParser failures trigger immediate CPU fallback without user blocking
6. OmniParser cache invalidates immediately on any state-changing GUI action
7. Visual Hash Verification (pHash/SSIM > 0.90) is mandatory before all element clicks
8. All extracted text is wrapped in EvidenceWrapper XML boundaries
9. Multi-monitor coordinates include monitor ID and DPI normalization to prevent wrong-screen clicks

---

## Section 4: Phase 3 - HTN and Remote Interface

### 4.1 Deprecation of Linear ReAct

The existing ReAct loop is insufficient for GUI automation due to:

- Unbounded iteration count risking infinite loops
- Dynamic step generation enabling hallucination
- Lack of structured verification between actions
- No built-in recovery from UI state changes

Phase 3 replaces ReAct with Hierarchical Task Networks for all GUI operations.

### 4.2 TurnGate HTN Output

The TurnGate generates rigid, sequential JSON sub-goals without runtime modification authority:

Task Network Schema:

```json
{
  "task_id": "gui_workflow_001",
  "max_duration_sec": 120,
  "sub_goals": [
    {
      "step": 1,
      "action": "get_screen_elements",
      "params": {"filter_type": "button"},
      "verify": "elements_found"
    },
    {
      "step": 2,
      "action": "click_element",
      "params": {"element_id": "btn_save"},
      "verify": "screen_changed"
    }
  ],
  "safe_abort_steps": [
    {"action": "press_shortcut", "params": {"keys": ["esc"]}}
  ]
]
```

Constraints:

- Sub-goals are immutable after TurnGate emission
- No branching logic permitted in execution layer
- Verification checkpoints required after state-changing actions
- Safe Abort Sequence defined upfront for failure recovery (note: GUI state often cannot be reversed, for example sending an email cannot be undone with the Esc key; the system attempts a graceful halt rather than guaranteed time-reversal)

### 4.3 GUI Executor Loop

Implement a dedicated GUI Executor that processes HTN sub-goals:

Execution Flow:

1. Receive HTN JSON from TurnGate
2. Initialize cancellation token tree
3. For each sub-goal in sequence:
   - Check kill switch status
   - Execute atomic action
   - Run verification check with Bounded Micro-Retries
   - On verification failure after retries exhausted: execute safe abort sequence and abort
4. Report completion or failure to AgentLoop

Bounded Micro-Retries:

While the HTN plan remains immutable, the executor is permitted Bounded Micro-Retries for verification checks. This accounts for natural UI rendering latency without triggering catastrophic safe aborts on timing-sensitive operations.

Retry Policy:
- Maximum 3 retry attempts per verification check
- Bounded Exponential Retry Window: initial delay 250ms, doubling each attempt (250ms, 500ms, 1000ms)
- Retry counter resets for each sub-goal
- Immediate safe abort on final retry failure

Verification Strategies:

- screen_changed: Localized Perceptual Diff comparing only the bounding box of the targeted UI element plus 10px padding margin (using pHash or SSIM), not full-screen hash to avoid chronic false negatives from clocks and notifications
- elements_found: non-empty element list
- text_present: OCR or element label match
- window_state: title or geometry verification

### 4.4 Remote Interface Security

Before enabling GUI control over external targets, the following prerequisites are mandatory:

**Connection Security**

- All remote GUI sessions require signed connection leases
- Lease contains target fingerprint, expiration, and allowed actions
- SSH tunneling mandatory for all remote GUI traffic
- No direct GUI protocol exposure to public networks

**Authentication Requirements**

- Remote GUI commands require secondary PIN verification
- Target enrollment must be completed before GUI operations
- Audit logs are synchronized to both host and target
- Failed authentication triggers automatic session termination
- Remote GUI operations must start in strict VIEW-ONLY mode
- Secondary out-of-band confirmation required for every active click or type event until telemetry proves environment stability

**Bandwidth and Latency**

- Remote GUI limited to targets with latency under 200ms
- Screenshot compression required for bandwidth constraints
- Frame rate throttling to prevent network saturation
- Automatic fallback to command-only mode on degradation

### 4.5 Safety Invariants

Phase 3 invariants:

1. HTN plans are generated once and never modified during execution
2. GUI Executor refuses any sub-goal not present in original plan
3. Verification failures trigger immediate safe abort sequence, not retry
4. Remote GUI sessions auto-terminate on lease expiration
5. Maximum GUI task duration is capped at 5 minutes
6. All remote GUI actions are logged to both host and target audit trails

---

## Appendix A: Implementation Dependencies

Required crates and tools:

- enigo or xdotool for atomic GUI actions
- OmniParser for screen element detection
- tokio for async cancellation handling
- Existing kria-core infrastructure for ToolRegistry and PolicyEngine

---

## Appendix B: Success Criteria

Phase 1 completion criteria:

- click_mouse, type_text, press_shortcut tools implemented
- Kill switch responds within 100ms
- All GUI tools classified as RED tier with HITL

Phase 2 completion criteria:

- OmniParser sidecar operational
- get_screen_elements and click_element tools functional
- VRAM usage under 2GB for vision operations

Phase 3 completion criteria:

- HTN task generation in TurnGate
- GUI Executor processing sub-goals with verification
- Remote GUI control secured with lease authentication

---

**End of Specification**
