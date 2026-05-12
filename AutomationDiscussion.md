# KRIA GUI Automation & Desktop Assistant — Discussion Archive

This document captures the full Q&A discussion regarding KRIA's GUI Automation Intelligence Architecture, its current readiness, the gap between execution skill and cognitive intelligence, and the path toward a sovereign desktop assistant with human-in-the-loop interaction.

---

## 1. Current Readiness Assessment

**Question:** "Is my GUI Automation System completely ready and wired properly? Can I use a prompt like 'open code and write a program to print 2 to 20 table and run the program' and will it execute properly?"

**Answer:** No — not yet.

Brutally honest answer: that exact prompt **will not execute correctly end-to-end** in the current build. The system will likely *do something*, but it won't reliably open VS Code, write the program correctly, and run it.

### 1.1 What works right now

- **No more log spam:** F8 idempotent kill switch + Wayland probe + ReleaseAll syntax fix all landed and rebuilt.
- **No more false "Done!":** the discovery-stub fallback was removed; unmatched GUI prompts now fall through honestly.
- **VS Code recognition:** `generate_gui_workflow` now matches the standalone word `code` (and `vscode`, `vs code`, `visual studio code`).
- **LLM HTN planner fallback:** when the rule planner can't match, the LLM is asked for a Goal Tree JSON.
- **F12 confidence gate:** low-confidence ambiguous prompts don't get hijacked into HTN.
- **Daemon is well-behaved:** `TaskComplete` clean disconnect, dead-man heartbeat, no emergency-release storms on passive sessions.

### 1.2 What is missing for that specific prompt

Your prompt requires four logical phases:

| Phase | Status | Detail |
|-------|--------|--------|
| Open VS Code | ✅ | Rule-based path covers this (after the `code` match). |
| Create a new file / focus the editor pane | ❌ | Workflow template does not handle "new file" — it just focuses whatever element OmniParser labels `txt_main`. In VS Code with no open file, that label may not exist. Typing will go nowhere. |
| Type a correct Python multiplication-table program | ⚠️ | `ContentGenerator` (`visual_reasoning.rs`) generates *something* for "Generated" content, but quality for "print 2 to 20 table" depends on the LLM and is not validated. |
| Run the program | ❌ | No step in `build_text_editor_workflow` covers running code. There is no `run_code` / terminal / shortcut step. The workflow ends after typing. |

Even step 1 has the **target-lock-on-wrong-window bug (F3)** unfixed: `get_active_window` is called once at workflow start before VS Code has finished launching (3.5 s typical), so the lock may capture the KRIA window itself. We saw exactly this in earlier logs: `window_title=K.R.I.A.`.

### 1.3 What is wired but only as skeleton (not active in your runtime)

These are all `Noop*` placeholders behind the `gui_cognition_v2` feature flag. They are not on by default and not consumed by the runtime path:

- `IntentCompiler` — no real semantic extraction yet.
- `EnvironmentGrounder` — no environment facts feeding the planner.
- `ExecutionVerifier` — no intent-level success checks (file contents, process exit).
- `UiTrustBoundary` — OCR sanitizer compiled but not yet wired into the LLM context path.
- `kria-test-app` — sandbox stub only, no Xvfb harness running yet.

### 1.4 What will likely happen if you run that prompt right now

Most probable outcome:

1. `TurnGate` routes the prompt to GUI (`Automate` operation, confidence high enough — "open" + "write" + "code" + "run" verb cluster).
2. `generate_gui_workflow` matches because of the `code` word-boundary match → builds the editor workflow.
3. `editor` is detected as `code` → tries `xdotool windowactiv $(xdotool getactivewindow)` etc.
4. Workflow opens VS Code. Target-lock probably grabs the KRIA window or some intermediate. Step 3 (click middle of screen) lands somewhere wrong.
5. `get_screen_elements` runs OmniParser. The expected element id `txt_main` is unlikely to be present in a freshly-opened VS Code with no open file.
6. `click_element` fails or clicks the wrong thing.
7. `type_text` either fires into the wrong window or no-ops.
8. Workflow reports failure — **not** false success this time, since the verification will catch it.
9. No "run the program" step exists at all, so the task is structurally incomplete.

### 1.5 What it will take to make your exact prompt work

In priority order:

| Step | What needs to happen | Effort | Already in plan? |
|------|----------------------|--------|-----------------|
| 1 | P1 `IntentCompiler` to recognise multi-action intent ("open … and write … and run …") and emit three goals, not one | Medium | Yes, P1 |
| 2 | P3 GoalTree shape in `GuiPlanner` so multi-phase workflows can express "open → wait-ready → new-file → type → run → verify" | Medium | Yes, P3 |
| 3 | F3 `WindowSpawnTracker` — wait for the spawned VS Code window before locking | Low | Yes, P2 |
| 4 | New action: `open_new_file` (Ctrl+N) in the editor workflow template, gated by which editor is running | Low | Implicit in P3 |
| 5 | New action: `run_in_terminal` for the "and run it" leg — either Ctrl+F5 in VS Code or external `python file.py` shell call | Low | Implicit in P3 |
| 6 | P4 `ExecutionVerifier` with `DeterministicOutput` checking the terminal output for "2 x 2 = 4" etc. — the "did it actually run?" check | Medium | Yes, P4 |
| 7 | Persistent daemon session (F2) so the workflow doesn't reopen sockets between steps | Medium | Yes, in plan |

### 1.6 A prompt that will work today

These should work with the current build:

- `"open gedit and write a hello world program"` — exact pattern matches the existing editor workflow template, single-action, single-app, no run step.
- `"open the text editor and type 'KRIA test successful'"` — same.
- Anything matching the gedit/mousepad/kate/notepad/code/vscode template with a single type action and no "run it" follow-up.

These will **not** work yet:

- Anything chaining multiple actions across applications.
- Anything requiring "and run it" / "and execute it" / "and check the output".
- Anything in VS Code that needs Ctrl+N (new file) or Ctrl+S (save) before typing.
- Anything on a pure Wayland session.
- Anything where the target app takes >2 s to map a window.

### 1.7 Honest recommendation

Do **not** test with `"open code and write a program to print 2 to 20 table and run the program"` yet — it is the worst case for the current planner and will produce a confusing failure.

Test with `"open gedit and write a fibonacci program"` to confirm the P0 fixes (no log spam, no false success). Then proceed with P1 (`IntentCompiler`) and P3 (GoalTree shape) before attempting multi-phase prompts again.

The architecture is sound and the plan is implementation-ready. The execution layer is not yet wired for multi-phase tasks — that is precisely what P1–P4 deliver. Anything sooner would be theater.

---

## 2. Does the GUI Automation Skill Lack Intelligence?

**Question:** "That means GUI Automation Skill lacks Intelligence?"

**Answer:** Yes — and that's the precise diagnosis.

The GUI Automation **skill** (the mechanical execution layer) is *strong*. The **intelligence** above it is *thin*. Those are two different things, and conflating them is what made the system feel "broken" when it's actually "unfinished".

### 2.1 The split, named clearly

| Layer | What it is | State |
|-------|------------|-------|
| **Motor cortex** (skill) | Daemon, uinput, xdotool, kill switch, rate limiter, target lock, protected mode, heartbeat, clipboard backup, OmniParser bridge, OCR pipeline | **Strong.** Production-grade. Reliable when given correct instructions. |
| **Cognition** (intelligence) | Turning *"open code and write a 2-to-20 table and run it"* into the right sequence of correct instructions, verifying the *task* succeeded, recovering when reality disagrees | **Thin.** Keyword matcher + one hard-coded recipe + an unconstrained LLM fallback. |

So when you ask "is the GUI skill ready?", the honest answer splits:

- **Can it click, type, press shortcuts, manage focus, recover from stuck modifiers, refuse dangerous targets, halt on user activity?** Yes. That's the *skill*.
- **Can it understand a multi-phase natural-language request, plan it correctly, ground it against your actual desktop state, verify the intent was fulfilled, and escalate honestly when it can't?** Not yet. That's the *intelligence*.

### 2.2 Why this gap exists structurally

Look at what `generate_gui_workflow` actually does in `htn_integration.rs:48-108`:

- Lowercase the prompt.
- `lower.contains("editor")`, `lower.contains("gedit")`, `lower.contains("code")` …
- If any matches → run `build_text_editor_workflow` — an 8-step template hard-coded for "open editor → focus → type → done".
- If nothing matches → ask the LLM for a flat `Vec<SubGoal>` with `serde_json::from_str` and no schema check.

That is **not** intelligence. It's **dispatch**. The intelligence questions the layer should answer — *what does the user actually want? what verbs are in the request? what are the success criteria? what state must the environment be in first? is the request even feasible right now?* — are not asked anywhere. The motor cortex executes whatever the dispatcher hands it, blindly.

### 2.3 What "intelligence" would look like, concretely

Same prompt — *"open code and write a 2-to-20 table and run it"* — running through the proposed cognition stack:

1. **`IntentCompiler`** (P1) parses three verbs: `Open(code)`, `Generate+Type(python:multiplication_table 2..20)`, `Run(active_buffer)`. Sets `declared_success_criteria = [ProcessLaunched("code"), DeterministicOutput("2 x 2 = 4")]`.
2. **`EnvironmentGrounder`** (P2) reports: VS Code not running, no file open, monitor 0 is primary, Python 3.11 on PATH, terminal binary `bash`.
3. **`GuiPlanner`** (P3) produces a Goal Tree: `[OpenApp(code) → WaitForWindow(class="Code") → NewFile(Ctrl+N) → Type(literal) → Save(/tmp/table.py) → RunInTerminal(python /tmp/table.py)]`, each leaf tagged with a Verifiability class.
4. **`GuiExecutor`** consumes that tree. If "WaitForWindow" times out, it injects a pre-registered fallback subtree (try `flatpak run`, then `snap run`) — bounded, not recursive.
5. **`ExecutionVerifier`** (P4) doesn't say "type_text returned ok"; it says *"file /tmp/table.py contains the substring `for i in range(2, 21)`"* and *"terminal output contains `20 x 20 = 400`"*. Either real success or real failure — no theater.
6. **`UncertaintyGovernor`** (P5) accumulates verifier outcomes. Drops below threshold → HITL escalation, not silent flailing.

None of those layers exist as runtime components yet. They exist as skeletons under `#[cfg(feature = "gui_cognition_v2")]` that were created in the earlier session. The traits compile, the `Noop*` placeholders pass tests, but the runtime path still goes through the old keyword dispatcher.

So when you say "the GUI skill lacks intelligence" — yes. More precisely:

- The **skill** is wired properly. It would not benefit from being made smarter; making the motor cortex "think" is precisely the overengineering trap §7 of the review warns against.
- The **layer above the skill** — the one that turns intent into a typed plan — is the missing piece. The plan calls this `GuiPlanner` with its `IntentCompiler` + `EnvironmentGrounder` inputs and `ExecutionVerifier` output.
- The **current substitute** for that layer is `htn_integration::generate_gui_workflow` + the LLM fallback. Both are dispatch, not cognition.

That is exactly the gap the review document and P1–P4 are designed to close, in that order, with bounded scope and zero new GPU load.

### 2.4 What this means for your near-term use

- **Skill-only prompts work today:** `"type 'hello'"`, `"press ctrl+s"`, `"click at 500,400"`, `"open gedit and write a fibonacci program"` (because the template happens to fit).
- **Intelligence-required prompts don't work today:** anything multi-phase, anything that says "and run it", anything that needs to recognise the user means VS Code's *new-file* state, anything where success means *"the program produced correct output"* rather than *"the keystrokes left the keyboard"*.

The fix is not more motor cortex. It is the small, typed, bounded cognition layer described in `GUI_INTELLIGENCE_REVIEW.md` §4 — exactly the four modules already stubbed and waiting for their real implementations in P1–P4.

---

## 3. Desktop Robotics — Full Laptop/PC GUI Control via Prompt

**Question:** "I wanted full laptop/PC GUI Control as Robotics behaviour using Prompt with intelligent handling task. Can I achieve this? I was wondering some approaches if I can use any GUI Automation Open Source tools and technologies as per latest 2026. I can also build completely own if possible. Can you suggest me some approach as per my requirements? The best suitable approaches. And high level Idea."

**Answer:** Yes — fully achievable. Frame it as *desktop robotics* and you already have most of the substrate.

You are not asking for science fiction. The "PC-as-robot driven by prompt" problem has a name in the 2024–2026 literature — **GUI Agents** / **Computer-Use Agents (CUA)** — and there are already several open-source stacks that prove it works. The honest strategic question is **build vs. integrate vs. hybrid**, not "is it possible".

### 3.1 The right mental model: this is robotics, not "automation"

Treat your laptop as a robot whose end-effector is keyboard+mouse and whose only sensor is the framebuffer. Every working robotics stack — ROS, NVIDIA Isaac, Boston Dynamics — runs the same five-stage loop:

| Robotics Stage | Desktop-Robot Equivalent | KRIA Today | OSS State-of-Art |
|----------------|--------------------------|------------|-----------------|
| **Perception** | Screen → structured UI (elements, text, layout) | OmniParser sidecar ✓ | OmniParser-v2, OS-Atlas, ShowUI |
| **World model** | "What apps exist, what's focused, what's on screen, what was the last action?" | Skeleton (`world_model`) ✗ not wired | None mature OSS; usually rolled per-project |
| **Planning** | Prompt → typed sequence of actions with success criteria | Keyword dispatcher + LLM JSON ⚠ thin | UI-TARS, Aguvis, OS-Genesis, Claude/GPT computer-use |
| **Execution** | Inject keys / clicks reliably with safety | uinput daemon + kill switch + rate limit ✓ strong | PyAutoGUI / xdotool / ydotool — primitive vs KRIA |
| **Verification** | "Did the task actually succeed?" | Surface-level only ✗ | Almost no one does this well; emerging research area |

You already have the two **hardest** layers (perception + execution) at a quality OSS projects struggle to match. The cognition gap is exactly the missing **Planning + Verification** robotics stages.

This reframing matters: **don't think "automation framework", think "robotics control loop."** It changes which OSS projects to mine.

### 3.2 State of OSS desktop-agent stacks (late 2025 → 2026)

#### 3.2.1 GUI-native foundation models (the brain candidates)

These are vision-language models trained or fine-tuned end-to-end on "screenshot → next action". They are the closest thing to plug-and-play "GUI cortex" weights in 2026.

| Model | Org | Size | License | What it does well | Fit for RTX 4050 6 GB |
|-------|-----|------|---------|-------------------|----------------------|
| **UI-TARS** | ByteDance (Seed) | 2B / 7B / 72B | Apache-2.0 | End-to-end GUI agent: screenshot in, action out, with explicit reasoning trace. The strongest fully-open desktop-agent model as of late 2025. | 7B-DPO at int4 ≈ 4.5 GB. Tight but feasible with GPU lease swap (KRIA already supports this). |
| **OS-Atlas (Pro)** | OS-Copilot collective | 4B / 7B | Apache-2.0 | Specialised GUI **grounding** (find element by description). Pairs with a planner. | 4B int4 ≈ 2.5 GB. Comfortable. |
| **ShowUI** | Microsoft + USC | 2B | MIT | Compact vision-language-action for UI. Lighter than UI-TARS but weaker reasoning. | 2B int4 ≈ 1.5 GB. Very comfortable. |
| **Aguvis** | Salesforce + HKU | 7B | Apache-2.0 | Two-stage (perception + planning) using pure vision. Strong on web; mixed on desktop. | 7B int4 ≈ 4.5 GB. Tight. |
| **CogAgent (v2)** | THUDM | 9B / 18B | Apache-2.0 (weights) | Earliest of the bunch, still respectable. | 9B int4 ≈ 5.5 GB. **At budget limit.** |
| **OmniParser-v2** | Microsoft | <1B | MIT | Perception only — structured UI extraction. You already use it. | ~1 GB. Already proven on your hardware. |

**Verdict for your stack:** UI-TARS-7B-DPO is the most credible "drop-in brain" candidate in late-2025 OSS. OmniParser stays as the dense parser, UI-TARS handles "what to do next". They are complementary, not competing.

#### 3.2.2 Agent orchestration frameworks (the scaffolding)

These wrap a model + screen capture + input injection into a runnable loop. Most are Python.

| Project | Org | License | What it is | Should KRIA adopt it? |
|---------|-----|---------|------------|----------------------|
| **UI-TARS-desktop** | ByteDance | Apache-2.0 | Electron + Python reference agent for UI-TARS. Production-ish. | No, but **mine it.** Read its action schema and prompt templates; reuse those ideas inside KRIA. |
| **Cradle** | Tencent AI Lab | MIT | OS-level general computer control framework. Games + desktop. | No. Heavy Python, research-oriented. Mine ideas (especially their skill library and memory tiers). |
| **Self-Operating Computer Framework** | HyperWriteAI | MIT | Lightweight Python orchestrator, model-agnostic (Claude/GPT/Gemini/Ollama). | No. Useful as a reference architecture and for prompt patterns. |
| **OpenAdapt** | OpenAdapt.AI | MIT | Record-and-replay + LLM generalisation; strong on Windows. | No. Focus is workflow capture; different problem class. |
| **OpenInterpreter `--os` mode** | open-interpreter | AGPL | LLM that runs code locally, optional screen-control. | No. AGPL is a license problem for KRIA; and it's code-execution-centric, not GUI-centric. |
| **Skyvern / browser-use / Stagehand** | various | MIT/Apache | Browser-only agents (CDP/Playwright). | No — too narrow. KRIA's scope is the whole desktop. |
| **OWL** | CAMEL-AI | Apache-2.0 | Multi-agent framework, GAIA leaderboard winner among OSS. | No, but **read the prompts.** Their planner prompts for tool-using agents are public and well-tested. |
| **Anthropic Computer Use (API + ref impl)** | Anthropic | Proprietary model, MIT ref code | Frontier closed model + Docker sandbox reference. | No — sovereignty violation. Useful only as a benchmark to beat. |

**Verdict for your stack:** Don't adopt any of these wholesale. Every one of them violates one or more of KRIA's invariants (sovereign Rust core, in-process safety, no Python in the hot path, no Electron, no AGPL). Mine them for prompts, action schemas, and verification patterns.

#### 3.2.3 Datasets & benchmarks worth knowing

Useful for evaluation, not integration:

- **OSWorld** — 369 real desktop tasks, sandboxed VMs. The standard CUA benchmark.
- **WindowsAgentArena** — Microsoft's Windows equivalent.
- **GAIA** — general-assistant tasks; many are GUI-bound.
- **VisualWebArena / WebArena** — browser-only.
- **AndroidWorld / Mobile-Agent benchmarks** — Android, but action schemas transfer.

These let you *measure* whether your stack is improving rather than vibe-testing.

### 3.3 The best approach for KRIA specifically

Given KRIA's hard constraints (sovereign Rust core, RTX 4050 6 GB, 16 GB RAM, no AGPL, no Electron, audit-bound execution), there are exactly three credible paths. I rank them honestly.

#### Approach A — **Hybrid: KRIA core + UI-TARS-7B as planner-grounding model** ⭐ recommended

Keep everything you have. Replace the keyword dispatcher + LLM-JSON fallback with a single call to UI-TARS-7B running in the existing GPU lease alongside (or swapped with) OmniParser.

```
prompt ──► IntentCompiler (P1, Rust)
            ├─ typed GuiTaskSpec
            ▼
       EnvironmentGrounder (P2, Rust)
            ├─ closed-enum facts (windows, files, monitors)
            ▼
       GuiPlanner (P3, Rust)
            ├─ calls UI-TARS-7B for "given task + screenshot + facts → next action with reasoning"
            ├─ OR falls back to local Qwen2.5-7B + OmniParser if UI-TARS lease unavailable
            ▼
       GuiExecutor (existing, hardened)
            ├─ uinput daemon (yours, strong)
            ▼
       ExecutionVerifier (P4, Rust)
            ├─ Verifiability Classes (file effects, process launched, OCR substring, …)
            ▼
       UncertaintyGovernor (P5) ─► HITL or KillSwitch when warranted
```

**Why this is the right answer:**

- KRIA's motor cortex + safety frame is *already better* than anything in §2.2. Don't throw that away.
- UI-TARS replaces the weakest link (planning) with state-of-art open weights — same effort as integrating any LLM, more capability than rolling your own.
- Verification, grounding, and safety stay in Rust where they belong (auditable, fast, sovereign).
- VRAM fits: 4.5 GB UI-TARS int4 + 1 GB OmniParser ≈ 5.5 GB, with KRIA's existing GPU lease serialising them when needed.
- This is the path the GUI Intelligence Review v2 was structured to enable — P1–P4 are the integration plan.

**Cost:** P1–P4 implementation (already specified in the review doc) + a UI-TARS inference adapter in `crates/kria-core/src/llm/`. Probably 2–3 focused weeks.

#### Approach B — Self-build the brain, no foundation GUI model

Continue with general-purpose LLM (Qwen2.5/Llama) as the planner and rely on OmniParser + your own prompt engineering + the v2 cognition stack. No GUI-specialised model.

**Trade-off:** Slower to good results. You'd be re-deriving in prompts what UI-TARS already learned in 50 M+ supervised examples. The wrong choice in 2026 — there's no reason not to stand on the GUI-native model shoulders.

#### Approach C — Adopt UI-TARS-desktop or Cradle wholesale

Throw KRIA away, run someone else's stack.

**Trade-off:** Kills the sovereign-Rust thesis. Inherits their licensing, their Python supply chain, their telemetry, their Electron surface. You lose the safety guarantees you built. Not worth it.

### 3.4 The "robotics behaviour" you actually want

You used the word *robotics* and you meant it. Here is what that translates to as concrete behaviour in your stack:

| Robotics Primitive | Desktop-Robot Version | Where it lives in KRIA |
|-------------------|----------------------|-----------------------|
| **Closed-loop perception** | Screenshot every action OR on event, re-parse with OmniParser | Already wired in `tools/vision_automation.rs`; add adaptive cadence in P2 |
| **State estimation** | "Best current belief about which window is focused, which file is open, where the cursor is" | `EnvironmentGrounder` (P2) + `UncertaintyGovernor` (P5) |
| **Task decomposition** | "open code, write program, run it" → 6-leaf Goal Tree | `GuiPlanner` (P3) emitting RFC 008 §1.2 Goal Tree |
| **Reactive control** | "If the window I expected didn't appear, try fallback subtree" | `GuiExecutor` PRA injection (already in code, needs Goal Tree shape from P3) |
| **Verification & supervision** | "Did the file actually get written? Did the program actually print 2x2=4?" | `ExecutionVerifier` Verifiability Classes (P4) |
| **Safety envelope** | "Refuse near password manager, halt on user activity, release modifiers, never run with Wayland-only without fallback" | Already strong: `safety/`, `KillSwitchInterceptor`, `ProtectedModeDetector` |
| **Skill library** | "I've successfully opened gedit 47 times with this recipe — prefer it" | `OperationalMemory` tier (P9) |
| **Human-in-the-loop** | "I'm <50 % confident — ask the user instead of guessing" | `UncertaintyGovernor` + existing `HitlGateway` |

That is the robotics stack. You're 60 % of the way there. The remaining 40 % is exactly P1–P5 in the review.

### 3.5 Concrete first move (next 1–2 sessions)

Don't do everything. Do this, in order:

1. **Land P0 verification** (already done in this PR). Confirm clean logs on a real `"open gedit and write a fibonacci program"` run. This is your baseline.
2. **Stand up a UI-TARS-7B inference adapter** in `crates/kria-core/src/llm/ui_tars.rs`. Use `llama.cpp` or `mistral.rs` for int4 inference; reuse the existing GPU-lease semaphore. **Pure adapter — no agent logic yet.** Verify VRAM headroom on your 4050.
3. **Implement P1 `IntentCompiler`** — replace the keyword dispatch. Smallest, highest-leverage cognition module.
4. **Implement P4 `ExecutionVerifier` *before* P3.** This is counterintuitive but correct: honest success/failure signals are worth more than richer plans. Once verification is honest, you can iterate the planner against real signal instead of vibes.
5. **Implement P3 `GuiPlanner` using UI-TARS as the planning model.** Now you have a robotics loop: perceive (OmniParser) → plan (UI-TARS via P3) → execute (motor cortex) → verify (P4) → repeat.
6. **Benchmark on a tiny OSWorld subset.** 10 tasks, sandboxed, your own laptop via Xvfb (per Appendix D of the review). That's your regression suite.

Skip everything else (P5–P10) until 1–6 demonstrably work end-to-end.

### 3.6 Honest reality check

- **Can you achieve "PC-as-robot via prompt" locally on RTX 4050 + 16 GB in 2026?** Yes — at roughly UI-TARS-7B's capability level, which is genuinely useful for ~60–70 % of single-app desktop tasks per OSWorld results.
- **Will it match Anthropic Computer Use or OpenAI Operator?** No — those use frontier closed models 20–100× larger. You will lag on long-horizon novel tasks. You will *match or beat* them on safety, latency, and sovereignty.
- **Is "make this my major purpose" rational?** Yes, but scope it: *"sovereign, audit-bound, local desktop robot for the 100 most common workflows on my Linux box"* is achievable in months. *"General PC agent that does anything any user can do"* is a frontier-lab research project.
- **Build vs integrate?** **Integrate model weights, build the system.** Use UI-TARS / OmniParser / Qwen as commodities. Build the cognition, safety, verification, and audit layers yourself — that's where the differentiated value is, and it's where KRIA already leads.

The intelligence is buildable. The robotics framing is correct. The OSS landscape in 2026 finally has the foundation models you need. Your motor cortex is already production-grade. The plan is on disk.

Now go implement P1 + P4.

---

## 4. Prompt Capability Envelope

**Question:** "The above suggestion you gave after following and implementing that — can I use these prompts and expect correct behaviour?"

- `"Open Whats app and text Faizan Hye"`
- `"Open Write a program to print fibonacci and Run it"`

**What type of prompts would I be able to use?"

### 4.1 Honest answer: prompt by prompt, with the actual ceiling

The headline: **even after P1–P4 + UI-TARS-7B integration, those two specific prompts have problems independent of model quality.** One has a missing parameter; the other has a typo that creates real ambiguity. The cognition layer (the very thing you'd be building) would *correctly refuse to guess on both* and ask you to clarify. That refusal is the feature, not the limitation.

### 4.2 Your two specific prompts, traced through the proposed stack

#### Prompt 1: `"Open WhatsApp and text Faizan Hye"`

What the stack does, step by step:

1. **`IntentCompiler`** parses: `Verb=Open(WhatsApp) + Verb=Send(message, recipient=Faizan Hye)`.
2. It immediately flags **`Ambiguity::ContentScopeUnclear`** — *"text Faizan Hye **what**?"* You said *who*, not *what*. The compiler refuses to invent a message.
3. Returns `ClarifyRequest { question: "What message should I send to Faizan Hye?" }`.
4. You answer, e.g. `"on my way"`. Now the spec is complete.
5. **`EnvironmentGrounder`** checks: is WhatsApp (Desktop / Web / via browser) actually installed? It records what it finds.
6. **`GuiPlanner`** (UI-TARS) emits a Goal Tree: `OpenApp(whatsapp) → WaitForReady → FocusSearchBox → Type("Faizan Hye") → ClickFirstContact → FocusMessageBox → Type("on my way") → ⏸ HITL_CONFIRM_BEFORE_SEND → PressEnter`.
7. The send step is **RED tier** under your existing `PolicyEngine` — KRIA *will* ask you to confirm before pressing Enter, because sending a message to a real human is irreversible.
8. **`ExecutionVerifier`** verifies each leaf with appropriate classes: `ProcessLaunched("whatsapp")`, `OcrTextPresent("Faizan Hye")` in conversation header, `DeterministicOutput("on my way")` in message draft.

**Will it succeed?** Probably **60–75%**, assuming:
- WhatsApp Desktop or Web is installed and logged in.
- A contact literally named "Faizan Hye" exists (no fuzzy match against "Faisal Hyder" etc — the planner won't disambiguate without help).
- WhatsApp's UI doesn't pop a "what's new" modal or QR re-auth screen (long-horizon recovery is hard).
- You confirm the HITL prompt before send.

**Honest residual risks:**
- Multiple "Faizan"s in contacts → planner clicks the first hit. The verifier should catch this via OCR on the conversation header, but if the wrong-Faizan's name also happens to start with the same letters, false-positive is possible.
- WhatsApp Web in a browser tab adds another window-grounding layer (which browser? which tab?).
- "Send to a human" is forever a HITL-gated action in KRIA. Full autonomy on this is **deliberately disallowed** by your safety frame, and you should not relax that.

#### Prompt 2: `"Open Write a program to print fibonacci and Run it"`

There's a typo — `"Open Write"` likely meant `"Open <editor>, write…"`. The cognition layer catches this:

1. **`IntentCompiler`** sees `Verb=Open` with no clear target, then `Verb=Write(program: fibonacci)`, then `Verb=Run(active_buffer)`.
2. Flags **`Ambiguity::AppNotSpecified`** — *"Which editor should I open? gedit, VS Code, or your terminal with nano?"*
3. Returns `ClarifyRequest` with the candidate list derived from `EnvironmentGrounder`'s app inventory.
4. You answer `"VS Code"`. Now the spec is complete.
5. **`GuiPlanner`** emits: `OpenApp(code) → WaitForWindow(class="Code") → NewFile(Ctrl+N) → SetLanguage(python) → Type(generated_fibonacci_program) → Save(/tmp/fib.py) → OpenTerminal(Ctrl+\`) → Type("python /tmp/fib.py\n") → VerifyOutput`.
6. **`ExecutionVerifier`** runs `DeterministicOutput("0, 1, 1, 2, 3, 5, 8")` against terminal output — *real* success, not "the keystrokes left the keyboard".

**Will it succeed?** **70–85%** for VS Code, **85–95%** for gedit + external `python` shell call. Higher for gedit because the UI is simpler and there's no terminal-pane state to manage.

**Residual risks:**
- VS Code's "select Python interpreter" popup on first run can derail.
- The generated fibonacci code quality depends on UI-TARS's coding ability (mediocre) or whichever LLM you use for `ContentClass::Generated`.
- If `python` isn't on PATH, the verifier reports honest failure → HITL.

### 4.3 What you'll actually be able to ask for (capability envelope)

After P1–P4 + UI-TARS-7B, here's the realistic ceiling. I'm being conservative because OSWorld results for *any* open-weights model in late 2025 hover at **22–40%** end-to-end success — frontier closed models reach ~50%. KRIA's safety + verification frame should push your *useful* success rate higher than raw OSWorld because honest failure is preferred over silent wrong-doing.

#### Tier 1 — works reliably (~85–95%)

Single-app, deterministic targets, no semantic person/entity lookup, no irreversible side effects.

- `"open gedit and write a hello world program"`
- `"open the terminal and run ls -la ~/Downloads"`
- `"open Firefox and go to github.com/anthropics"`
- `"take a screenshot and save it to ~/Desktop"`
- `"open the file manager and navigate to /tmp"`
- `"Press Ctrl+Alt+T"`, `"type 'echo hello'"`, `"click at 500,400"` (primitive passthrough — was already working)
- `"open VS Code and write a Python fibonacci program in /tmp/fib.py"` (file path specified, no terminal step → easier)
- `"open settings and turn on dark mode"` — if the model has seen GNOME/KDE settings before. Hit or miss.

#### Tier 2 — works with HITL confirmation on irreversible steps (~70–85%)

Multi-phase, deterministic verification possible, irreversible action at the end.

- `"open VS Code, write a fibonacci program, and run it"` — multi-phase, terminal output verifiable.
- `"open WhatsApp and message Faizan: 'on my way'"` — content provided, HITL on send.
- `"reply to the last email from Alice with 'thanks, will review'"` — HITL on send.
- `"open Spotify and play 'Bohemian Rhapsody'"` — small risk of wrong match.
- `"compose a tweet saying 'shipping KRIA v2'"` — HITL on post.
- `"find the file called 'budget.xlsx' and open it"` — assuming filesystem search works.
- `"open the calendar and create an event called 'standup' tomorrow at 10am"` — calendar-specific UI variability.

#### Tier 3 — works sometimes, expect to debug (~40–60%)

Cross-app coordination, multi-window juggling, semantic disambiguation, or apps with chaotic UI states.

- `"take the data from report.csv and paste it into a new email to my boss"` — three apps, clipboard handoff, recipient lookup.
- `"find the PDF I was reading yesterday and continue from page 12"` — requires recent-files reasoning.
- `"reorder the windows so VS Code is on the left monitor"` — window-manager-specific.
- `"sign me into AWS Console with my work profile"` — login flows often have anti-bot detection.
- `"open Steam, find Hades, and launch it"` — Steam UI is famously model-hostile.

#### Tier 4 — will not work reliably, don't ship these as supported

Long-horizon, ambiguous, or requiring real reasoning over screen content.

- `"organise my desktop"` (what does "organise" mean to whom?)
- `"summarise everything on my screen and tell me what's important"`
- `"play a song I'd enjoy right now"`
- `"edit the photo to look more professional"`
- `"apply for that job posting I bookmarked"` (long form fills, novel sites, captchas)
- `"debug this code that's failing in CI"` (requires actual cognition over external state)
- `"watch this video and tell me when the speaker mentions kernels"` (real-time perception, hours)
- Anything in a 3D game, video editor, CAD, DAW.

#### Forever-prohibited (KRIA's safety frame, not a model limitation)

- Anything inside a password manager (`ProtectedModeDetector` blocks).
- Anything in banking sites (blocked by default in `safety/`).
- Auto-confirming destructive shell commands (`rm -rf`, `dd`, etc. — `BlacklistChecker` blocks).
- Autonomous sending of messages/emails/posts to humans without HITL.
- Anything on the user's *real* desktop during testing (Appendix D harness rules).

### 4.4 The pattern: what makes a prompt Tier-1 vs Tier-4

Three orthogonal axes determine where any prompt lands:

| Axis | Tier-1 end | Tier-4 end |
|------|------------|------------|
| **Target specificity** | Exact app name, exact file path, exact text | Pronouns, "the thing", "what matters" |
| **Verifiability** | File exists, process exited 0, exact substring in terminal | "Looks nice", "is appropriate", "sounds good" |
| **Time horizon** | <30 seconds, <8 actions | Minutes to hours, dozens of windows |

A useful rule of thumb for you: **if you can describe the success criterion in one sentence that a `grep` or `stat` could check, it's probably Tier-1 or Tier-2.** If success requires a human to look and nod, it's Tier-3 at best.

### 4.5 How to expand the envelope over time

After the v2 core lands, three optional investments push the ceiling upward:

1. **Skill library / `OperationalMemory` (P9).** Every successful Tier-1/2 task becomes a cached recipe. Next time the same pattern is asked, you skip planning and replay. This is how Cradle and UI-TARS-desktop boost reliability over time.
2. **Per-app affordance hints.** Hand-curated YAML files like *"WhatsApp's search box is in the top-left, message box is at the bottom"*. UI-TARS doesn't need them, but they're cheap insurance and convert flaky Tier-2 prompts into reliable Tier-1.
3. **Recorded demonstration ingest.** Future work: capture one human run of a workflow, replay-with-generalisation. Big lift; defer.

You do **not** need a bigger model to expand the envelope. You need the skill library + verification feedback loop. That's why P4 (verifier) is put before P3 (richer planner) in the implementation order.

### 4.6 Net answer

- **`"Open WhatsApp and text Faizan Hye"`** — after asking you "what message?", will work ~60–75% of the time with HITL on send, **which is the correct behaviour for sending messages to humans**. Full autonomy on this is not on the roadmap and shouldn't be.
- **`"Open Write a program to print fibonacci and Run it"`** — after asking you "which editor?", will work ~70–85% of the time. The "and run it" part is a real capability you don't have today and will gain in P3.
- **The realistic envelope is broad enough to be genuinely useful:** essentially any single-app deterministic workflow on your machine, plus most multi-phase code-and-run workflows, plus most messaging/posting workflows with a final HITL confirm.
- **The envelope ceiling is real:** novel apps, ambiguous intent, long horizons, and games are out of scope for *any* 2026 open-weights stack — not just yours.

If you accept that ceiling and treat the HITL prompts as *features* rather than friction, this is a major-purpose-worthy system. If you expect frontier closed-model "Operator" behaviour, you'll be disappointed — by them too, frankly, on the harder tasks.

---

## 5. Human-in-the-Loop as a First-Class Pattern

**Question:** "I am free if it adds human to the loop. Something like Alexa or Google Assistant or Siri?"

**Answer:** That changes everything — for the better.

If you're comfortable with HITL as a *frequent first-class interaction* rather than a last-resort safety brake, you've just described a **conversational agent with computer-control** — Siri/Alexa/Google Assistant in spirit, but local, sovereign, and not chained to one vendor's APIs. This is the right framing, and it's *more achievable* than what you were aiming at before.

### 5.1 Three shifts when welcoming the loop

1. **Ambiguity stops being a failure mode and becomes a conversation turn.** *"text Faizan Hye"* missing the message? The system asks. That's not a degraded experience; that's how Siri works too.
2. **Destructive actions stop being blocked and start being previewed.** *"send this message"* shows the draft, you say yes. Same pattern as Alexa confirming a purchase.
3. **Long-horizon tasks become collaborative.** *"book me a flight to Tokyo"* becomes a guided dialog, not a black-box gamble.

Architecturally, **you already have the substrate for this.** `HitlGateway`, `PinGuard`, `PolicyEngine` RED-tier gating, the audio pipeline (`voice-whisper-rs`, `voice-piper-rs`, wake-word loop in `Cargo.toml`) — these aren't decorations, they're an Alexa-style UX waiting to be exposed.

### 5.2 How KRIA-with-HITL compares to Alexa / Siri / Google Assistant

Honest, not flattering.

| Dimension | Alexa / Siri / Google | KRIA + HITL (your stack) |
|-----------|----------------------|--------------------------|
| **Interface** | Voice-first, narrow text | Voice ✓ + text ✓ + on-screen (you have all three) |
| **Backend** | Cloud APIs (calendar, music, messaging) | GUI emulation of any app on your machine — far broader scope |
| **Intent breadth** | Curated skill catalog (~thousands), hand-trained | Open-ended, anything UI-TARS + your verifier can handle |
| **Reliability in narrow lane** | 95%+ (set timer, play song, call contact) | Initially 60–85%, climbing with skill library |
| **Reliability outside lane** | Flat refusal ("I don't know how to do that") | Best-effort attempt with HITL fallback |
| **Sovereignty / privacy** | Vendor-owned, every utterance leaves the device | Fully local, audit-logged, your data |
| **Ecosystem lock-in** | Apple/Amazon/Google ecosystem required | Linux-native, no vendor dependency |
| **Adding a new skill** | App developer + platform approval | Edit a YAML, restart agent |
| **Multi-step task** | Limited; mostly one-shot | First-class via Goal Tree + HITL between phases |
| **Knows your apps' state** | No — guesses via APIs | Yes — sees the screen via OmniParser |
| **Can do anything *you* can do at the keyboard** | No, only API-exposed actions | Yes, by design |

**Where Alexa/Siri win:** voice latency, polish, contact-list integration, "did I add milk to the list" reliability, music & smart-home native paths.

**Where KRIA wins:** ability to operate apps Alexa has never heard of (your IDE, your VPN client, your specific Linux desktop), full local privacy, no cloud round-trip, *radically* broader long-tail.

This is not a worse Siri. It's a *different category* — a **personal sovereign agent that drives your actual desktop**, with Siri's conversational ergonomics layered on top.

### 5.3 What your two prompts look like in this paradigm

#### "Open WhatsApp and text Faizan Hye"

```
KRIA: Got it — opening WhatsApp and starting a message to Faizan Hye.
      What should the message say?
You:  on my way
KRIA: (opens WhatsApp, searches "Faizan Hye", finds 1 match)
      I found one contact matching "Faizan Hye". Draft is ready:
        To:   Faizan Hye (last seen today)
        Body: on my way
      Send? (yes / edit / cancel)
You:  yes
KRIA: Sent. ✓
```

If there were two Faizans, the middle step becomes:

```
KRIA: Two contacts match "Faizan":
        1. Faizan Hye         (last chat: yesterday)
        2. Faizan Ahmed       (last chat: 3 weeks ago)
      Which one?
You:  1
KRIA: …
```

That's Siri-level UX with deeper GUI reach. Achievable after P1 + P4 + UI-TARS + a thin HITL UX shell.

#### "Open Write a program to print fibonacci and Run it"

```
KRIA: I'll open an editor, write a fibonacci program, and run it.
      Which editor — gedit, VS Code, or terminal+nano?
You:  vs code
KRIA: Python or another language?
You:  python
KRIA: (opens VS Code, creates new file, writes program, saves to /tmp/fib.py)
      Program written. First 10 fibonacci numbers it should print:
        0, 1, 1, 2, 3, 5, 8, 13, 21, 34
      Run it now? (yes / show me the code first / cancel)
You:  yes
KRIA: Ran in terminal:
        0 1 1 2 3 5 8 13 21 34
      Looks correct. ✓
```

Multi-phase, verified, with confirmation between phases. Tier-1 reliability under this UX.

### 5.4 Updated capability tiers (HITL is welcome)

Every previous tier moves up one notch because HITL converts **refusals into clarifications** and **destructive blocks into previews**.

| New Tier | What works | Examples |
|----------|------------|----------|
| **Conversational core (~90–95%)** | Anything where the *worst case* is "ask a question and try again" | `"open spotify and play bohemian rhapsody"`, `"text Faizan 'on my way'"`, `"send the slack message in #eng saying X"`, `"schedule a meeting tomorrow at 3pm with Alice"`, `"find that PDF I had open yesterday"` |
| **Guided multi-phase (~80–90%)** | Multi-step tasks with HITL between phases | `"open VS Code, write a fibonacci program, and run it"`, `"summarise the email from Bob and draft a reply"`, `"download the latest report from the team drive and open it"` |
| **Best-effort with rollback offer (~60–75%)** | Cross-app or novel-UI tasks; HITL on every irreversible step | `"fill this job application using my resume"`, `"post the photo from clipboard to instagram with caption X"`, `"apply the formatting from row 2 to all rows in this spreadsheet"` |
| **Out of scope (refuse politely)** | Things no 2026 stack can do | `"be creative for me"`, `"watch this 2-hour video and tell me what's important"`, `"play this game and win"` |

Tier 1 is now genuinely large. That's the size of the catalog Alexa publicly markets — and you'd cover it more flexibly, locally, and across any app you own.

### 5.5 The UX you'd design (Alexa-pattern, but for desktop)

Four conversational moves, each already mappable to an event in your bus:

| Move | When | Existing primitive |
|------|------|-------------------|
| **Acknowledge** | After parsing intent | `GuiEvent::IntentCompiled` → TTS via `voice-piper-rs` |
| **Clarify** | On `ClarifyRequest` from `IntentCompiler` | `ApprovalRequest` style prompt, voice or text |
| **Preview-then-confirm** | Before any RED-tier leaf in the Goal Tree | `HitlGateway::request_approval` → "Send this? Yes/No" |
| **Report** | After `ExecutionVerifier` outcome | `GuiEvent::TaskCompleted` → TTS summary |

The pattern is so consistent that you can write a single `ConversationalShell` module in `kria-desktop` that subscribes to `GuiEvent`/`SafetyEvent` and orchestrates these four moves. It's a UX layer, not an AI layer — a few hundred lines.

### 5.6 One important honesty check

Alexa/Siri feel magical for *narrow* things because they have ten years of intent training and tight API hooks. KRIA-with-HITL will feel magical for **breadth** but will *occasionally* ask clarifying questions Siri wouldn't, simply because Siri has been trained on millions of phrasings of "set a timer" and you haven't.

That's a feature in your context, not a bug — KRIA's clarifying questions are how it stays safe and how it learns your patterns (via `OperationalMemory` / skill library, P9). Each clarified prompt becomes a cached recipe and stops asking.

After ~50 successful interactions of any given pattern, KRIA stops clarifying that pattern. After ~500 across all patterns, it starts feeling like Siri — but it's *your* Siri, on *your* machine, controlling *your* apps.

### 5.7 Net recommendation

**Yes, target a personal-assistant-with-GUI-control. It's the right product shape for what you have.**

Concrete implications for the roadmap:

1. **Keep P1 (`IntentCompiler`) but expand its `ClarifyRequest` surface.** It becomes the conversational engine, not just a fallback. Treat clarifications as the *common* path.
2. **Make the HITL prompts feel like Siri** — single short question, voice-readable, "yes / no / edit" affordances. This is a UX patch on `HitlGateway`, not new architecture.
3. **Wire your existing voice stack** (`voice-whisper-rs` for STT, `voice-piper-rs` for TTS, wake-word) to the new event bus so the loop is voice-friendly by default. Text/click works in parallel.
4. **Build the skill library / `OperationalMemory` early** (move P9 forward). Every confirmed pattern becomes a cached recipe. This is how the system stops asking the same question twice and starts feeling assistant-like rather than interrogative.
5. **Settle on UI-TARS-7B as the planner-grounding model** as before. Now its job is even cleaner: it produces a Goal Tree, the HITL shell handles the chatty parts, the verifier handles correctness, the safety frame handles risk.

You went from *"can it do everything autonomously?"* (frontier-research-hard) to *"can it be a competent local assistant that asks when unsure?"* (genuinely achievable on your hardware, in months not years).

That second target is the right one. Aim there.

---

## 6. Open-Source Technology Landscape & Strategic Position

**Question:** "I think you got the correct Idea that I want a desktop Assistant with full control like Siri or Alexa Home and etc. Answer my questions below as per intelligence and updates.

1. Is there any open source tool or tech as per latest 2026 which I can integrate with my KRIA assistant?
2. Where do I stand at current — how much changes, updates, integrations needed? Will I have to remove existing functionalities or how much effort will require to follow the best path?
3. Should I use open source tech if available or built fully from scratch or Hybrid?"

### 6.1 Open-source pieces you can integrate (late 2025 → 2026, KRIA-compatible)

The assistant you want is a **stack of four pipelines**, not one tool. For each pipeline, here's what's mature, open, and license-compatible with KRIA's sovereign Rust core. Items you **already have wired in `Cargo.toml`** are marked ✓.

#### Pipeline A — Speech in / out (the "Alexa surface")

| Component | Project | License | Status for KRIA |
|-----------|---------|---------|----------------|
| Wake word | **openWakeWord** | Apache-2.0 | ✓ Feature flag `voice-wake-oww` already in tree |
| Speech-to-text | **Whisper.cpp** (via `whisper-rs`) | MIT | ✓ Feature flag `voice-whisper-rs` (+ CUDA/Vulkan variants) |
| Voice activity / turn-taking | **Silero-VAD** | MIT | Not wired yet. Tiny ONNX, runs CPU, easy to add |
| Text-to-speech | **Piper TTS** (via `piper-rs`/`ort`) | MIT | ✓ Feature flag `voice-piper-rs` |
| Higher-quality TTS (optional) | **Kokoro-82M** (released Jan 2025) | Apache-2.0 | Very fast, very natural, 82M params, runs CPU in <100 ms. Drop-in upgrade from Piper for "Siri-grade" voice. |
| Echo cancellation | **webrtc-audio-processing** | BSD | ✓ Feature flag `voice-aec` |

**Verdict:** Voice in/out is essentially solved. KRIA already has every layer; mostly a matter of turning the feature flags on and connecting them to the event bus.

#### Pipeline B — Vision / GUI perception

| Component | Project | License | Status |
|-----------|---------|---------|--------|
| Element parsing | **OmniParser-v2** | MIT | ✓ Running as sidecar |
| Optional grounding upgrade | **OS-Atlas-Pro-7B** | Apache-2.0 | Use when you need "find the element matching this description" beyond OmniParser's icon set |
| Optional screen QA | **InternVL2 / Qwen2-VL-7B** | various OSS | Heavy; defer unless needed |

**Verdict:** OmniParser is enough for now. Don't add a second vision model until you measure a perception bottleneck.

#### Pipeline C — Planning / agent brain

| Component | Project | License | Status |
|-----------|---------|---------|--------|
| GUI-native action model | **UI-TARS-7B-DPO** (ByteDance, 2025) | Apache-2.0 | Not wired. The single biggest leverage point. Drop-in for the planner. |
| General-purpose LLM (fallback / dialogue) | **Qwen2.5-7B-Instruct** or **Llama-3.1-8B-Instruct** | Apache-2.0 / Llama license | Use whichever you already have for L1Text. Stays in current GPU lease budget. |
| Grammar-constrained decoding | **llama.cpp GBNF** | MIT | ✓ Already exposed in `llm/local.rs::chat_with_grammar` |
| Inference runtime | **llama.cpp** or **mistral.rs** | MIT / Apache-2.0 | `mistral.rs` is Rust-native; better fit for sovereign stack. |

**Verdict:** UI-TARS-7B is the single addition with the largest delta. Everything else you have.

#### Pipeline D — Conversational orchestration (the "voice loop" layer)

This is where I want to be careful — there are popular OSS frameworks here, but most are **bad fits for KRIA** because they assume Python and cloud LLMs.

| Project | License | Verdict for KRIA |
|---------|---------|-----------------|
| **LiveKit Agents** | Apache-2.0 | Mine for ideas. Python; assumes cloud. Their barge-in / interruption logic is worth studying. |
| **Pipecat** (Daily.co) | BSD | Mine for ideas. Same — Python, but very clean state machine for conversational turn-taking. |
| **Vocode** | MIT | Same category. Python. |
| **Self-Operating Computer Framework** | MIT | Mine for prompts. Reference orchestrator. Python. |
| **UI-TARS-desktop** | Apache-2.0 | Mine for the action schema + prompt templates. Electron + Python; we keep the schema, drop the runtime. |
| **OWL** (CAMEL-AI, 2025) | Apache-2.0 | Mine for planner prompts. GAIA-leaderboard-winning OSS prompts. |
| **Cradle** (Tencent) | MIT | Mine for skill library shape. Their memory tiers are worth borrowing. |

**Verdict:** Don't adopt any framework wholesale. Read 4–5 of them in a weekend, lift the patterns, **write the conversational shell yourself in Rust** (~500–1000 lines). It's the layer that defines your product; you do not want a Python/Electron dependency at the top.

#### What's *not* worth integrating

For honesty's sake, things you might be tempted by but shouldn't pursue:

- **LangChain / LlamaIndex / AutoGen / CrewAI** — Python, abstraction-heavy, wrong layer of stack for KRIA. Use them as reading material, not dependencies.
- **Anthropic Computer Use / OpenAI Operator** — closed, cloud, violates sovereignty thesis.
- **Rabbit OS, Humane Cosmos, Adept ACT-1** — proprietary or defunct.
- **OpenInterpreter** — AGPL is a license problem if you ever distribute KRIA.

### 6.2 Where you stand right now — honest inventory

Concrete percentages, based on what's in the tree today.

| Capability | % done | What's done | What's missing |
|-----------|--------|-------------|----------------|
| **Motor cortex** (input injection, safety, daemon, kill switch) | **95%** | uinput daemon, kill switch, rate limit, protected mode, target lock, heartbeat, TaskComplete | F3 window-spawn tracker, F9 multi-monitor |
| **Voice I/O** | **70%** | All feature flags exist; engines compile; wake-word/STT/TTS implementations done | Not wired to the agent loop's event bus; no conversational state machine yet |
| **Perception** | **80%** | OmniParser sidecar running, pHash/SSIM verification, GPU lease | UI-TARS not integrated; perception cadence not adaptive |
| **Safety / HITL** | **85%** | HitlGateway, PinGuard, PolicyEngine RED-tier, audit, blacklist | HITL prompts are CLI-flavoured, not Siri-flavoured; no voice preview-and-confirm |
| **Cognition (planning)** | **20%** | Keyword dispatcher + LLM-JSON fallback; skeletons for IntentCompiler / EnvironmentGrounder / ExecutionVerifier already in tree behind `gui_cognition_v2` flag | Real implementations of all four; UI-TARS adapter; Goal Tree shape in executor |
| **Verification** | **15%** | Surface-level only (window match, pHash) | Verifiability Classes (file effect, process launched, deterministic output) |
| **Memory / skills** | **10%** | MemoryManager exists, SQLite ready | OperationalMemory tier; skill library; per-app launch profiles |
| **Conversational UX shell** | **5%** | Event types defined in skeleton; broadcast bus designed | Actual `ConversationalShell` module subscribing to GuiEvent/SafetyEvent and driving TTS/HITL |
| **Test harness (sandboxed)** | **20%** | kria-test-app skeleton, justfile targets, Appendix D spec | Xvfb integration, adversarial scenarios, `--features e2e-xvfb` wiring |
| **Audit / observability** | **75%** | tracing spans, audit log, trace NDJSON spec | Per-cognition-layer counters, GUI event tracing |

**Aggregate state:** roughly **55–60%** of an "Alexa-class personal desktop assistant" is in tree, mostly in the form of strong substrate (motor cortex, safety, voice engines, perception) plus skeletons of the missing cognition layers.

### 6.3 What needs to be **removed** (very little)

- The discovery-stub fallback in `gui_wiring` → already removed in this PR. ✓
- The keyword dispatcher in `htn_integration::generate_gui_workflow` → **demoted, not deleted.** It becomes the `RuleGuiPlanner` strategy inside the new `GuiPlanner` trait. Useful as a fast path.
- The flat `Vec<SubGoal>` in `GuiWorkflow` → **replaced by Goal Tree shape**, but as a structural refactor, not a feature removal.
- The per-tick reconnecting heartbeat loop in `gui_wiring.rs` → **replaced by persistent session connection (F2).** Functionality preserved, plumbing changed.

**Nothing user-visible needs to be removed.** Every existing capability either stays or graduates to a hardened variant.

### 6.4 Effort estimate (solo dev with AI pair, focused)

Honest hours and weeks, not optimistic ones:

| Phase | Scope | Effort | Output you can demo |
|-------|-------|--------|----------------------|
| P0 | Bug fixes + Wayland probe + idempotent kill | Done | Clean logs on Tier-1 prompts |
| P1 | `IntentCompiler` real implementation | 1 week | Clarifying questions on ambiguous prompts |
| P4 | `ExecutionVerifier` + Verifiability Classes | 1 week | Honest success/failure signals |
| UI-TARS adapter | Inference path + GPU-lease integration | 1 week | LLM planner now uses GUI-native model |
| P3 | `GuiPlanner` v2 emitting Goal Tree | 2 weeks | Multi-phase workflows ("write and run it") |
| P2 | `EnvironmentGrounder` + window spawn tracker + monitor map | 1 week | Correct target lock on slow-launching apps |
| Conversational shell | Wire voice + HITL + TTS to event bus | 1–2 weeks | Siri-style preview-and-confirm dialogs |
| Skill library (P9 brought forward) | `OperationalMemory` + cached recipes | 1 week | Reduced HITL friction after repeated patterns |
| Sandboxed E2E harness | Xvfb + kria-test-app GUI + adversarial tests | 1 week | Regression-tested capabilities |
| **Total to "competent Tier-1 assistant"** | | **~9–10 focused weeks** | |
| Tier-2 reliability polish | + ~50 hand-curated skill recipes | + 4–6 weeks | Genuinely Siri-comparable on your machine |
| **Total to "Siri-comparable for your workflows"** | | **~3–4 months focused, ~6 months part-time** | |

That's the real number. Not "a weekend"; not "two years". 3–4 focused months for a solo developer to reach a desktop assistant you'd actually use daily.

### 6.5 Build vs integrate vs hybrid — **Hybrid is the only correct answer**

The other two paths fail for specific reasons:

**Fully from scratch fails because:**
- Training a GUI-native model (UI-TARS-equivalent) requires millions of supervised examples. ByteDance, OpenAI, Anthropic spent $millions on this. You will lose.
- Writing your own STT/TTS from scratch is a 5-year research career. Whisper and Piper exist; use them.
- Reinventing OmniParser is months of computer-vision work. Microsoft did it.

**Adopting OSS wholesale fails because:**
- Every framework above is Python or Electron — kills KRIA's sovereign-Rust thesis.
- They include cloud assumptions, telemetry, vendor LLM defaults.
- They have weaker safety frames than KRIA's — you'd regress in your strongest area.
- You'd inherit licensing problems (AGPL in OpenInterpreter, etc.).

**Hybrid (the right path) has clear lanes:**

| Lane | Strategy | Concrete examples |
|------|----------|-------------------|
| **Use OSS verbatim** | Things that are pure capability you don't differentiate on | Whisper-cpp, Piper, openWakeWord, Silero-VAD, OmniParser, llama.cpp, mistral.rs |
| **Use OSS weights, write the Rust runtime** | Foundation models — adopt the model, control the inference path | UI-TARS-7B weights, Qwen2.5/Llama for dialogue |
| **Mine OSS patterns, write your own** | Things where the architecture is your product | Conversational shell, planner prompts, action schema, skill library, HITL UX, audit |
| **Build entirely yourself** | Differentiating layers, safety-critical paths, sovereignty boundary | Motor cortex, kill switch, policy engine, intent compiler, environment grounder, verifier, audit log |

**Translation:** weights and engines are commodities. **The system around them is your product.** That's where KRIA already wins and should keep winning.

### 6.6 Concrete next move

If you want momentum this week without committing to the full roadmap:

1. **Flip `voice-whisper-rs` + `voice-piper-rs` + `voice-wake-oww` on** in a dev build and wire wake → STT → existing text agent path → TTS. You instantly have an Alexa-shaped device that runs your current keyword dispatcher. Bad answers, but you can talk to it. This validates the voice loop independent of the cognition work.
2. **Pick one Tier-1 prompt** (e.g. *"open gedit and write a hello world program"*) and run it end-to-end via voice. Measure where it breaks.
3. **Start P1 (`IntentCompiler` real impl) next.** Smallest, highest-leverage cognition module. Pays off the first time it says "what message?" instead of failing.

After step 3, the rest of the roadmap (UI-TARS adapter, P3, P4) layers on without architectural surprises.

That's the path. Hybrid, sovereign, voice-fronted, 3–4 focused months to a daily-driver assistant. Genuinely achievable. Worth the bet.

---

## 7. LLM Strategy: Local vs Cloud vs Hybrid

**Question:** "I got it that Siri, Alexa is output of 5-10 years of training and data collection but I can overcome or try to overcome this using an LLM Model to understand and act as brain, right? I know your concern might be that qwen2.5 7b might not be enough but what if I use models like minimax 2.5 or any such (free APIs but strong model) provided by OpenAI API, Gemini, Anthropic or etc. What if I use this same qwen 7b, isn't it capable at all? Answer in brief for all."

### 7.1 Can an LLM substitute for Alexa's 10 years of training data?

**Yes — that's exactly the bet, and it's a sound one.**

Alexa/Siri's "training data" is mostly hand-curated **intent grammars** for ~thousands of skills. An LLM replaces that with general language understanding learned from internet-scale text. The trade-off:

- **Alexa:** 99% accurate on its 5,000 known intents, 0% outside.
- **LLM-as-brain:** 80–90% accurate on **any** intent, including ones you never anticipated.

For a *personal* assistant (one user, your phrasings, your apps), the LLM path wins on day one. You don't need Alexa's coverage; you need *your* coverage.

### 7.2 Frontier free-API models as the brain

**Capability: yes, dramatically better than any 7B local model.** A frontier model will plan multi-phase desktop tasks far more reliably (~90% Tier-1, ~75% Tier-2 → probably 95% / 90% with Claude/GPT-5-class).

**But four caveats you must accept openly:**

| Concern | Reality |
|---------|---------|
| **Sovereignty** | Every prompt + screenshot leaves your machine. Violates KRIA's founding thesis. Reversible only by switching back to local. |
| **"Free" tiers expire** | OpenAI/Gemini/Anthropic free quotas are throttled or rate-limited; "free APIs" today become paid tomorrow. MiniMax/DeepSeek/Qwen-API are genuinely cheap but not free at scale. |
| **Latency** | 800–2500 ms per turn vs 200–500 ms local. Conversational feel suffers. |
| **Vision payload** | Sending screenshots to a cloud model is the privacy worst-case. Your file paths, message contents, passwords-on-screen all leave the device. |

**Pragmatic recommendation:** make the brain **pluggable** — `Backend::Local(UI-TARS)` vs `Backend::Cloud(Claude/GPT/Gemini/MiniMax)`. KRIA already has the `LocalBackend` / `RemoteBackend` abstraction in `llm/`. Default to local; offer cloud as an opt-in toggle in settings for hard tasks. Users who don't care about sovereignty get frontier quality; users who do, keep their privacy. You don't have to choose; you have to architect for both.

### 7.3 Is Qwen2.5-7B alone "enough"?

**Honest answer: enough for ~70% of what you want, not enough for ~30%.**

- **Where Qwen2.5-7B-Instruct holds up:** intent parsing, clarification dialogs, generating literal text (fibonacci code, hello world), simple multi-step plans, tool-call JSON, conversational replies. This is Tier-1 territory.
- **Where it visibly weakens:** novel GUI reasoning, long-horizon planning (>8 steps), recovering from unexpected UI states, disambiguating semantic queries ("the report from last Tuesday").
- **The fix isn't a bigger Qwen — it's a *specialised* model for the GUI part.** Qwen2.5-7B for **dialogue + intent**, UI-TARS-7B for **screen-grounded planning**. Two specialists beat one generalist at the same total VRAM. This is exactly what UI-TARS was trained to be paired with.

### 7.4 The clean architecture this implies

```
voice/text in
   ↓
Dialogue LLM (Qwen2.5-7B local  ◇  Claude/GPT cloud, pluggable)
   ↓                    ← clarification, conversation, content generation
IntentCompiler → GuiTaskSpec
   ↓
EnvironmentGrounder → OperationalFacts
   ↓
GUI Planning Model (UI-TARS-7B local  ◇  Claude-vision cloud, pluggable)
   ↓                    ← screenshot → next action
GuiExecutor → motor cortex
   ↓
ExecutionVerifier → success/failure
   ↓
TTS out
```

Two model slots, both **pluggable local-or-cloud**. Defaults local for sovereignty; users who want frontier quality flip a toggle. You ship one system, two reliability tiers.

### 7.5 Bottom line

- **LLM-as-brain** replaces 10 years of Alexa data. Sound bet.
- **Frontier free/cheap APIs** make the system *better* at the cost of *sovereignty*. Make them an opt-in backend, not the default.
- **Qwen2.5-7B alone** is *adequate for dialogue*, *weak for GUI planning*. Pair it with UI-TARS-7B and you have a local stack that punches well above its weight. Add cloud frontier as the escape hatch.
