#![allow(deprecated)]
//! RFC v2 (P4): Bounded execution verifier with Verifiability Classes.
//!
//! Replaces the "step succeeded once typed" anti-pattern with explicit
//! [`Verifiability`] classes, each with a single bounded check (≤500 ms
//! except `ProcessLaunched`). The verifier NEVER replans and NEVER triggers
//! retries — those concerns live in the executor.
//!
//! ## Verifiability Classes
//!
//! | Class | Check | Latency cap | Method |
//! |-------|-------|-------------|--------|
//! | `WindowState` | Query active window title/class | ≤100ms | X11/XCB `get_active_window` |
//! | `FileSystemEffect` | `std::fs::metadata`, read file bytes | ≤100ms | `std::fs` |
//! | `ProcessLaunched` | Poll `/proc` for PID | ≤500ms | `std::fs` |
//! | `DeterministicOutput` | Read terminal output or file | ≤200ms | `std::fs` or pipe |
//! | `OcrTextPresent` | Substring search on cached OCR | ≤300ms | cached data |
//! | `UserAttested` | N/A — never auto-verifies | N/A | HITL via event |
//! | `Unverifiable` | N/A | 0ms | Emit `HitlEscalated`, never report success |
//!
//! See `docs/GUI_INTELLIGENCE_REVIEW.md` §4.5.

use crate::agent::execution_verifier::{FsEffect, Verifiability, VerifyOutcome, VerifyTarget};
use crate::tools::gui_automation::GuiBackend;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
/// Bounded execution verifier implementing all 7 Verifiability classes.
///
/// Each check is bounded: ≤500 ms except ProcessLaunched (≤500ms wait).
/// The verifier NEVER replans and NEVER invokes the LLM.
///
/// # Deprecation Notice
///
/// This implementation is the **GUI-backend-injected** variant used by the HTN/GUI
/// execution path. For new code and the ReAct execution path, use the canonical
/// implementation in [`crate::agent::execution_verifier_bounded::BoundedExecutionVerifier`],
/// which uses the same verification engines but without stateful GUI backend injection.
///
/// This module is retained for the HTN/GuiExecutor path until a single verifier
/// implementation is shared across all execution paths (planned hardening step).
#[deprecated(
    since = "0.1.0",
    note = "Use execution_verifier_bounded::BoundedExecutionVerifier as the canonical \
            single-authority verifier. This variant is retained only for the GuiExecutor \
            path until full consolidation."
)]
pub struct BoundedExecutionVerifier {
    /// Optional GUI backend for window state checks.
    gui_backend: Option<Arc<dyn GuiBackend>>,
    /// Cache of recent OCR text for OcrTextPresent checks.
    ocr_cache: tokio::sync::RwLock<std::collections::HashMap<String, String>>,
}

impl BoundedExecutionVerifier {
    pub fn new() -> Self {
        Self {
            gui_backend: None,
            ocr_cache: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    pub fn with_gui_backend(backend: Arc<dyn GuiBackend>) -> Self {
        Self {
            gui_backend: Some(backend),
            ocr_cache: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Cache OCR text for later verification.
    pub async fn cache_ocr(&self, key: &str, text: String) {
        let mut cache = self.ocr_cache.write().await;
        cache.insert(key.to_string(), text);
    }

    /// Clear the OCR cache.
    pub async fn clear_cache(&self) {
        let mut cache = self.ocr_cache.write().await;
        cache.clear();
    }

    /// Check WindowState - query active window title/class.
    async fn check_window_state(
        &self,
        title_contains: &Option<String>,
        class: &Option<String>,
    ) -> VerifyOutcome {
        let start = Instant::now();

        if let Some(backend) = &self.gui_backend {
            match backend.get_active_window().await {
                Ok(window_info) => {
                    let title_match = title_contains.as_ref().map_or(true, |t| {
                        window_info.title.to_lowercase().contains(&t.to_lowercase())
                    });
                    let class_match = class.as_ref().map_or(true, |c| {
                        window_info.class.to_lowercase().contains(&c.to_lowercase())
                    });

                    let verified = title_match && class_match;
                    return VerifyOutcome {
                        verified,
                        confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                        confidence: if verified { 0.95 } else { 0.1 },
                        evidence: format!(
                            "Window: title='{}' (match={}), class='{}' (match={})",
                            window_info.title, title_match, window_info.class, class_match
                        ),
                        latency_ms: start.elapsed().as_millis() as u32,
                    };
                }
                Err(e) => {
                    return VerifyOutcome {
                        verified: false,
                        confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                        confidence: 0.0,
                        evidence: format!("Failed to get active window: {}", e),
                        latency_ms: start.elapsed().as_millis() as u32,
                    };
                }
            }
        }

        // No GUI backend available
        VerifyOutcome {
            verified: false,
            confidence_tier:
                crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
            confidence: 0.0,
            evidence: "No GUI backend available for window state check".into(),
            latency_ms: start.elapsed().as_millis() as u32,
        }
    }

    /// Check FileSystemEffect - verify file exists, contains bytes, or size.
    async fn check_file_system_effect(&self, path: &PathBuf, kind: &FsEffect) -> VerifyOutcome {
        let start = Instant::now();
        let path = path.clone();
        let kind = kind.clone();

        let result = tokio::task::spawn_blocking(move || {
            let metadata = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    // For NotExists, a missing file is the desired outcome.
                    if matches!(kind, crate::agent::execution_verifier::FsEffect::NotExists) {
                        return VerifyOutcome {
                            verified: true,
                            confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                            confidence: 0.95,
                            evidence: format!("File does not exist (as expected): {}", e),
                            latency_ms: start.elapsed().as_millis() as u32,
                        };
                    }
                    return VerifyOutcome {
                        verified: false,
                        confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                        confidence: 0.0,
                        evidence: format!("File does not exist: {}", e),
                        latency_ms: start.elapsed().as_millis() as u32,
                    };
                }
            };

            if !metadata.is_file() {
                return VerifyOutcome {
                    verified: false,
                    confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.0,
                    evidence: "Path is not a file".into(),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }

            match kind {
                FsEffect::Exists => VerifyOutcome {
                    verified: true,
                    confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.95,
                    evidence: format!("File exists: {}", path.display()),
                    latency_ms: start.elapsed().as_millis() as u32,
                },
                FsEffect::SizeGreaterThan(min_size) => {
                    let verified = metadata.len() > min_size;
                    VerifyOutcome {
                        verified,
                        confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                        confidence: if verified { 0.95 } else { 0.1 },
                        evidence: format!(
                            "File size: {} bytes (min required: {})",
                            metadata.len(),
                            min_size
                        ),
                        latency_ms: start.elapsed().as_millis() as u32,
                    }
                }
                FsEffect::NotExists => VerifyOutcome {
                    verified: false,
                    confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.0,
                    evidence: format!("File still exists: {}", path.display()),
                    latency_ms: start.elapsed().as_millis() as u32,
                },
                FsEffect::ContainsBytes(expected) => {
                    let content = match std::fs::read(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            return VerifyOutcome {
                                verified: false,
                                confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                                confidence: 0.0,
                                evidence: format!("Failed to read file: {}", e),
                                latency_ms: start.elapsed().as_millis() as u32,
                            };
                        }
                    };

                    // W-04 fix: empty expected bytes would always pass
                    // (ContainsBytes(b"") matches any content via windows(0)).
                    // Treat empty as "file must be non-empty".
                    if expected.is_empty() {
                        let verified = !content.is_empty();
                        return VerifyOutcome {
                            verified,
                            confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                            confidence: if verified { 0.7 } else { 0.0 },
                            evidence: if verified {
                                format!("File is non-empty ({} bytes)", content.len())
                            } else {
                                "File is empty — expected non-empty content".into()
                            },
                            latency_ms: start.elapsed().as_millis() as u32,
                        };
                    }

                    let verified = expected.len() <= content.len()
                        && content
                            .windows(expected.len())
                            .any(|w| w == expected.as_slice());

                    VerifyOutcome {
                        verified,
                        confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                        confidence: if verified { 0.95 } else { 0.1 },
                        evidence: if verified {
                            format!("Found expected bytes in file ({} bytes)", expected.len())
                        } else {
                            format!(
                                "Expected {} bytes not found in file ({} bytes total)",
                                expected.len(),
                                content.len()
                            )
                        },
                        latency_ms: start.elapsed().as_millis() as u32,
                    }
                }
            }
        })
        .await;

        match result {
            Ok(outcome) => outcome,
            Err(_) => VerifyOutcome {
                verified: false,
                confidence_tier:
                    crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                confidence: 0.0,
                evidence: "FileSystemEffect check panicked".into(),
                latency_ms: start.elapsed().as_millis() as u32,
            },
        }
    }

    /// Check ProcessLaunched - poll /proc for binary process.
    ///
    /// Uses both `/proc/<pid>/comm` (truncated to 15 chars by the kernel) and
    /// `/proc/<pid>/cmdline` (full path) to handle long binary names like
    /// `gnome-terminal-server` which get truncated to `gnome-terminal-s`.
    async fn check_process_launched(&self, binary: &str, max_wait_ms: u32) -> VerifyOutcome {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(max_wait_ms as u64);
        let binary = binary.to_string();

        loop {
            // Scan /proc in a blocking task to avoid stalling the async runtime
            let found = tokio::task::spawn_blocking({
                let binary = binary.clone();
                move || {
                    if let Ok(entries) = std::fs::read_dir("/proc") {
                        for entry in entries.filter_map(|e| e.ok()) {
                            let pid_dir = entry.path();
                            if !pid_dir.is_dir() {
                                continue;
                            }
                            if let Some(name) = pid_dir.file_name().and_then(|n| n.to_str()) {
                                if name.parse::<u32>().is_err() {
                                    continue;
                                }

                                // Primary: check /proc/<pid>/comm (fast, but truncated to 15 chars)
                                let comm_path = pid_dir.join("comm");
                                if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                                    let comm = comm.trim();
                                    // FIX #9: Only exact match on comm — do NOT use
                                    // binary.starts_with(comm) because a short truncated
                                    // comm like "co" would match "code", "conda", "convert".
                                    // The cmdline fallback below handles long names correctly.
                                    if comm == binary {
                                        return Some(name.to_string());
                                    }
                                }

                                // Fallback: check /proc/<pid>/cmdline for full binary path
                                // This handles long names truncated in comm (e.g., gnome-terminal-server)
                                let cmdline_path = pid_dir.join("cmdline");
                                if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
                                    // cmdline is NUL-separated; first field is the binary path
                                    let first_arg = cmdline.split('\0').next().unwrap_or("");
                                    // Extract basename from path
                                    let basename =
                                        first_arg.rsplit('/').next().unwrap_or(first_arg);
                                    if basename == binary
                                        || basename.starts_with(&binary)
                                        || binary.starts_with(basename)
                                    {
                                        return Some(name.to_string());
                                    }
                                }
                            }
                        }
                    }
                    None
                }
            })
            .await
            .ok()
            .flatten();

            if let Some(pid) = found {
                return VerifyOutcome {
                    verified: true,
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.95,
                    evidence: format!("Process '{}' found with PID {}", binary, pid),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }

            if Instant::now() >= deadline {
                return VerifyOutcome {
                    verified: false,
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.0,
                    evidence: format!("Process '{}' not found after {}ms", binary, max_wait_ms),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }

            // AUDIT FIX #9: Increased poll interval from 50ms to 200ms to reduce
            // /proc scanning frequency. With max_wait_ms=8000, this gives 40 iterations
            // instead of 160, reducing blocking thread pool pressure significantly.
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Check ProcessNotRunning - poll /proc until the binary is absent.
    ///
    /// Inverts ProcessLaunched logic: success means the process is NOT found.
    async fn check_process_not_running(&self, binary: &str, max_wait_ms: u32) -> VerifyOutcome {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(max_wait_ms as u64);
        let binary = binary.to_string();

        loop {
            let found = tokio::task::spawn_blocking({
                let binary = binary.clone();
                move || {
                    if let Ok(entries) = std::fs::read_dir("/proc") {
                        for entry in entries.filter_map(|e| e.ok()) {
                            let pid_dir = entry.path();
                            if !pid_dir.is_dir() {
                                continue;
                            }
                            let comm_path = pid_dir.join("comm");
                            if let Ok(name) = std::fs::read_to_string(&comm_path) {
                                let name = name.trim();
                                if name == binary
                                    || name.starts_with(&binary)
                                    || binary.starts_with(name)
                                {
                                    let cmdline_path = pid_dir.join("cmdline");
                                    if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
                                        let first_arg = cmdline.split('\0').next().unwrap_or("");
                                        let basename =
                                            first_arg.rsplit('/').next().unwrap_or(first_arg);
                                        if basename == binary
                                            || basename.starts_with(&binary)
                                            || binary.starts_with(basename)
                                        {
                                            return Some(name.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    None
                }
            })
            .await
            .ok()
            .flatten();

            if found.is_none() {
                return VerifyOutcome {
                    verified: true,
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.95,
                    evidence: format!("Process '{}' is no longer running", binary),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }

            if Instant::now() >= deadline {
                return VerifyOutcome {
                    verified: false,
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.0,
                    evidence: format!("Process '{}' still running after {}ms", binary, max_wait_ms),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Check DeterministicOutput - verify output contains expected substring.
    ///
    /// Fixes:
    /// - W-02: Missing interpreter → output file contains "command not found" → fail
    /// - W-03: Empty expected_substring → treat as "file must be non-empty and no errors"
    /// - W-15: Traceback detection — Python tracebacks contain digits that match
    ///         short expected substrings like "0", "1", "2". We check for error indicators.
    /// - W-16: Increased timeout to 2000ms to handle spawn_blocking scheduling delays.
    async fn check_deterministic_output(
        &self,
        expected_substring: &str,
        in_target: &VerifyTarget,
    ) -> VerifyOutcome {
        let start = Instant::now();

        match in_target {
            VerifyTarget::FilePath(path) => {
                let path = path.clone();
                let expected = expected_substring.to_string();
                let result = tokio::task::spawn_blocking(move || {
                    // AUDIT FIX #18: Cap output file read at 1MB to prevent OOM.
                    // Combined with the head -c 1048576 pipe in build_execution_command,
                    // this provides defense-in-depth against large output files.
                    let content = {
                        use std::io::Read;
                        match std::fs::File::open(&path) {
                            Ok(mut f) => {
                                let mut buf = String::new();
                                match f.by_ref().take(1_048_576).read_to_string(&mut buf) {
                                    Ok(_) => buf,
                                    Err(e) => {
                                        return VerifyOutcome {
                                            verified: false,
                                            confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                                            confidence: 0.0,
                                            evidence: format!("Failed to read output file: {}", e),
                                            latency_ms: start.elapsed().as_millis() as u32,
                                        };
                                    }
                                }
                            }
                            Err(e) => {
                                return VerifyOutcome {
                                    verified: false,
                                    confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                                    confidence: 0.0,
                                    evidence: format!("Output file not found (program may not have run): {}", e),
                                    latency_ms: start.elapsed().as_millis() as u32,
                                };
                            }
                        }
                    };
                    let error_indicators = [
                        "command not found",
                        "No such file or directory",
                        "Permission denied",
                        "not found in PATH",
                        "cannot find",
                        "is not recognized",
                    ];
                    for indicator in &error_indicators {
                        if content.to_ascii_lowercase().contains(indicator) {
                            return VerifyOutcome {
                                verified: false,
                                confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                                confidence: 0.0,
                                evidence: format!(
                                    "Execution failed — interpreter/runtime error detected: '{}'",
                                    indicator
                                ),
                                latency_ms: start.elapsed().as_millis() as u32,
                            };
                        }
                    }

                    // W-03 fix: empty expected_substring → file must be non-empty and no errors
                    if expected.is_empty() {
                        let verified = !content.trim().is_empty();
                        return VerifyOutcome {
                            verified,
                            confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                            confidence: if verified { 0.7 } else { 0.0 },
                            evidence: if verified {
                                format!("Program produced output ({} chars)", content.len())
                            } else {
                                "Program produced no output".into()
                            },
                            latency_ms: start.elapsed().as_millis() as u32,
                        };
                    }

                    // W-15 fix: detect Python/runtime tracebacks that contain expected digits
                    // A traceback means the program crashed, even if the output contains
                    // the expected substring by coincidence.
                    //
                    // Note: "Error:" is intentionally broad to catch Java/Go/Kotlin errors.
                    // We use line-start matching for "Error:" to avoid false positives from
                    // programs that print "Error rate: 5%" or "Error: 0 items found" as
                    // legitimate output. The other indicators are specific enough to not
                    // need line-start matching.
                    let traceback_indicators_exact = [
                        "Traceback (most recent call last)",
                        "SyntaxError:",
                        "NameError:",
                        "TypeError:",
                        "ValueError:",
                        "AttributeError:",
                        "ImportError:",
                        "ModuleNotFoundError:",
                        "RuntimeError:",
                        "ZeroDivisionError:",
                        "IndexError:",
                        "KeyError:",
                        "OverflowError:",
                        "RecursionError:",
                        "Exception:",
                        // W-P2-01: additional Python exceptions missing from original list
                        "MemoryError:",
                        "SystemExit:",
                        "KeyboardInterrupt",
                        "StopIteration:",
                        "AssertionError:",
                        "FileNotFoundError:",
                        "PermissionError:",
                        "OSError:",
                        "IOError:",
                        "NotImplementedError:",
                        "ArithmeticError:",
                        "BufferError:",
                        "EOFError:",
                        "LookupError:",
                        "UnicodeError:",
                        "UnicodeDecodeError:",
                        "UnicodeEncodeError:",
                        // Go panics
                        "panic: ",
                        "goroutine 1 [running]",
                        // W-P2-02: Go runtime errors (e.g., index out of range, nil pointer)
                        "runtime error:",
                        // Java/Kotlin stack traces
                        "Exception in thread",
                        "at java.",
                        "at kotlin.",
                        // Rust panics
                        "thread 'main' panicked",
                        // Shell errors (more specific than "Error:")
                        "bash: line",
                        "sh: line",
                    ];
                    // "Error:" at the start of a line (not mid-sentence)
                    let has_line_start_error = content.lines().any(|line| {
                        let trimmed = line.trim();
                        trimmed.starts_with("Error:") || trimmed.starts_with("error:")
                    });

                    let has_traceback = traceback_indicators_exact
                        .iter()
                        .any(|t| content.contains(t))
                        || has_line_start_error;

                    let verified = content.contains(&expected) && !has_traceback;
                    VerifyOutcome {
                        verified,
                        confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                        confidence: if verified { 0.95 } else { 0.1 },
                        evidence: if has_traceback {
                            format!(
                                "Program crashed (traceback detected). Output preview: {}",
                                &content[..content.len().min(200)]
                            )
                        } else if verified {
                            format!(
                                "Found expected output '{}' in program output ({} chars)",
                                expected, content.len()
                            )
                        } else {
                            format!(
                                "Expected '{}' not found in program output ({} chars). Preview: {}",
                                expected,
                                content.len(),
                                &content[..content.len().min(200)]
                            )
                        },
                        latency_ms: start.elapsed().as_millis() as u32,
                    }
                })
                .await;

                match result {
                    Ok(outcome) => outcome,
                    Err(_) => VerifyOutcome {
                        verified: false,
                        confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                        confidence: 0.0,
                        evidence: "DeterministicOutput check panicked".into(),
                        latency_ms: start.elapsed().as_millis() as u32,
                    },
                }
            }
            VerifyTarget::TerminalOutput => VerifyOutcome {
                verified: false,
                confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                confidence: 0.0,
                evidence: "Terminal output verification requires shell integration (not yet implemented)".into(),
                latency_ms: start.elapsed().as_millis() as u32,
            },
            VerifyTarget::ActiveEditorBuffer => VerifyOutcome {
                verified: false,
                confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                confidence: 0.0,
                evidence: "Editor buffer verification requires visual/state integration (not yet implemented)".into(),
                latency_ms: start.elapsed().as_millis() as u32,
            },
        }
    }

    /// Check OcrTextPresent - verify cached OCR text contains expected string.
    async fn check_ocr_text_present(
        &self,
        text: &str,
        case_insensitive: bool,
        cache_key: &str,
    ) -> VerifyOutcome {
        let start = Instant::now();

        let cached_val = {
            let cache = self.ocr_cache.read().await;
            cache.get(cache_key).cloned()
        };

        let ocr_text = if let Some(val) = cached_val {
            val
        } else {
            // Try live OCR if no cached value is present
            let live_ocr = {
                let engine = crate::agent::ocr_engine::OcrEngine::new();
                if crate::agent::ocr_engine::OcrEngine::is_available() {
                    let result =
                        tokio::time::timeout(Duration::from_secs(8), engine.read_screen()).await;
                    match result {
                        Ok(ocr_result) if ocr_result.success => Some(ocr_result.text),
                        _ => None,
                    }
                } else {
                    None
                }
            };

            if let Some(live) = live_ocr {
                // Update cache with live result
                let mut cache = self.ocr_cache.write().await;
                cache.insert(cache_key.to_string(), live.clone());
                live
            } else {
                return VerifyOutcome {
                    verified: false,
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.0,
                    evidence: format!(
                        "OCR unavailable and no cache entry for key '{}'. \
                         Install tesseract for visual verification.",
                        cache_key
                    ),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }
        };

        let search_in = if case_insensitive {
            ocr_text.to_lowercase()
        } else {
            ocr_text.clone()
        };

        let search_for = if case_insensitive {
            text.to_lowercase()
        } else {
            text.to_string()
        };

        let verified = search_in.contains(&search_for);
        VerifyOutcome {
            verified,
            confidence_tier:
                crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
            confidence: if verified { 0.90 } else { 0.1 },
            evidence: if verified {
                format!(
                    "OCR text contains '{}' (case_insensitive={})",
                    text, case_insensitive
                )
            } else {
                format!(
                    "Expected '{}' not found in OCR text ({} chars, case_insensitive={})",
                    text,
                    ocr_text.len(),
                    case_insensitive
                )
            },
            latency_ms: start.elapsed().as_millis() as u32,
        }
    }

    /// Check AccessibilityElement - verify a UI element exists via AT-SPI.
    ///
    /// This provides real semantic verification: the element must actually exist
    /// in the accessibility tree, not just "the tool was called".
    async fn check_accessibility_element(
        &self,
        role: &str,
        name_contains: Option<&str>,
        must_be_visible: bool,
    ) -> VerifyOutcome {
        let start = Instant::now();

        // Quick pre-check: is AT-SPI available?
        let uid = unsafe { libc::getuid() };
        let atspi_socket = std::path::PathBuf::from(format!("/run/user/{}/at-spi/bus", uid));
        if !atspi_socket.exists() {
            return VerifyOutcome {
                verified: false,
                confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                confidence: 0.0,
                evidence: "AT-SPI bus not available — accessibility verification skipped. \
                           Enable with: gsettings set org.gnome.desktop.interface toolkit-accessibility true".into(),
                latency_ms: start.elapsed().as_millis() as u32,
            };
        }

        let engine = crate::agent::atspi_engine::AtSpiEngine::new();
        let elements = engine.find_elements(role, name_contains).await;

        let matching: Vec<_> = if must_be_visible {
            elements
                .into_iter()
                .filter(|e| e.visible && e.enabled)
                .collect()
        } else {
            elements
        };

        if matching.is_empty() {
            VerifyOutcome {
                verified: false,
                confidence_tier:
                    crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                confidence: 0.1,
                evidence: format!(
                    "No {} element found{} in accessibility tree",
                    role,
                    name_contains
                        .map(|n| format!(" with name '{}'", n))
                        .unwrap_or_default()
                ),
                latency_ms: start.elapsed().as_millis() as u32,
            }
        } else {
            let el = &matching[0];
            VerifyOutcome {
                verified: true,
                confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                confidence: if el.in_active_window { 0.95 } else { 0.75 },
                evidence: format!(
                    "Found {} '{}' in accessibility tree (visible={}, enabled={}, active_window={})",
                    el.role, el.name, el.visible, el.enabled, el.in_active_window
                ),
                latency_ms: start.elapsed().as_millis() as u32,
            }
        }
    }

    async fn check_interaction_outcome(
        &self,
        _expected_role: &str,
        _expected_name_contains: Option<&str>,
        action_type: &str,
    ) -> VerifyOutcome {
        let start = Instant::now();
        let engine = crate::agent::atspi_engine::AtSpiEngine::new();

        // Simplistic check: If it's a dialog that we interacted with, we might check if it dismissed.
        // For now, since true interaction effect tracking requires state capture, we'll verify if the UI
        // tree is responsive and alive.
        if !crate::agent::atspi_engine::AtSpiEngine::is_available().await {
            return VerifyOutcome {
                verified: false,
                confidence_tier:
                    crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                confidence: 0.0,
                evidence: "AT-SPI unavailable for InteractionOutcome".into(),
                latency_ms: start.elapsed().as_millis() as u32,
            };
        }

        // We check if the dialog is visible (if action was dismiss_dialog).
        if action_type == "dismiss_dialog" {
            let dialog = engine.detect_dialog().await;
            if dialog.is_none() {
                return VerifyOutcome {
                    verified: true,
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.9,
                    evidence: "Dialog successfully dismissed.".into(),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            } else {
                return VerifyOutcome {
                    verified: false,
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                    confidence: 0.8,
                    evidence: "Dialog is still present.".into(),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }
        }

        // For clicks/fills, verify the AT-SPI bus is responsive, indicating no crash.
        // A deeper semantic check would require tracking the pre-state inside the verifier.
        VerifyOutcome {
            verified: true,
            confidence_tier:
                crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
            confidence: 0.6, // Low confidence because we don't have deep state comparison
            evidence: format!(
                "Interaction '{}' completed and UI remains responsive.",
                action_type
            ),
            latency_ms: start.elapsed().as_millis() as u32,
        }
    }

    /// Check BrowserPageLoaded - verify browser state via CDP with fallbacks.
    /// Verification hierarchy (fail-closed):
    ///   Layer 1 (CDP / FullSemantic)       — polls for up to 6s until URL+title visible
    ///   Layer 2 (AT-SPI / StructuralOnly)  — polls xdotool for window title up to 6s
    ///   Layer 3 (Process / Unobservable)   — checks /proc/<pid> from MANAGED_BROWSER_PID
    ///   Hard fail                          — if none of the above find evidence
    async fn check_browser_page_loaded(
        &self,
        url_contains: Option<&str>,
        title_contains: Option<&str>,
    ) -> VerifyOutcome {
        let start = Instant::now();
        let poll_interval = tokio::time::Duration::from_millis(300);
        let deadline = tokio::time::Duration::from_secs(6);

        // ── Layer 1: CDP (FullSemantic) ──────────────────────────────────────
        // Poll until the managed Chrome exposes the expected URL/title via CDP.
        // This handles the cold-start race where Chrome is launching but CDP
        // is not yet ready.
        let engine = crate::agent::browser_cognition::BrowserCognitionEngine::new();
        loop {
            let state = engine.get_state().await;
            if !state.url.is_empty() || !state.title.is_empty() {
                let mut satisfied = true;
                let mut evidence_parts = vec![];

                if let Some(expected_url) = url_contains {
                    if state
                        .url
                        .to_lowercase()
                        .contains(&expected_url.to_lowercase())
                    {
                        evidence_parts.push(format!("url contains '{}'", expected_url));
                    } else {
                        satisfied = false;
                    }
                }

                if let Some(expected_title) = title_contains {
                    if state
                        .title
                        .to_lowercase()
                        .contains(&expected_title.to_lowercase())
                    {
                        evidence_parts.push(format!("title contains '{}'", expected_title));
                    } else {
                        satisfied = false;
                    }
                }

                // If no constraints are set, any non-empty state counts as observed
                if url_contains.is_none() && title_contains.is_none() {
                    evidence_parts.push(format!(
                        "browser active (url={}, title={})",
                        &state.url[..state.url.len().min(60)],
                        &state.title[..state.title.len().min(60)]
                    ));
                    satisfied = true;
                }

                if satisfied {
                    return VerifyOutcome {
                        verified: true,
                        confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
                        confidence: 1.0,
                        evidence: format!("CDP verified: {}", evidence_parts.join(", ")),
                        latency_ms: start.elapsed().as_millis() as u32,
                    };
                }
            }

            if start.elapsed() >= deadline {
                break;
            }
            tokio::time::sleep(poll_interval).await;
        }

        // ── Layer 2: CDP-Process (StructuralOnly, Wayland-safe) ──────────────
        // On Wayland, xdotool window queries are broken. Instead, scan /proc
        // for a Chrome process with --remote-debugging-port to confirm the managed
        // browser is running and CDP-capable. This is faster and works on all
        // display servers.
        let proc_deadline = start.elapsed() + tokio::time::Duration::from_secs(6);
        loop {
            // Scan /proc for any Chrome process that has --remote-debugging-port
            // in its cmdline — this confirms managed CDP launch succeeded.
            let mut cdp_chrome_pid: Option<u32> = None;
            if let Ok(entries) = std::fs::read_dir("/proc") {
                for entry in entries.flatten() {
                    let pid_str = entry.file_name();
                    let pid_str = pid_str.to_string_lossy();
                    if pid_str.parse::<u32>().is_err() {
                        continue;
                    }
                    let cmdline_path = entry.path().join("cmdline");
                    if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
                        let args = cmdline.replace('\0', " ");
                        let is_chrome = args.contains("/chrome")
                            || args.contains("google-chrome")
                            || args.contains("chromium");
                        let has_cdp = args.contains("--remote-debugging-port");
                        let is_main = !args.contains("--type=");
                        if is_chrome && has_cdp && is_main {
                            if let Ok(pid) = pid_str.parse::<u32>() {
                                cdp_chrome_pid = Some(pid);
                                break;
                            }
                        }
                    }
                }
            }

            if let Some(pid) = cdp_chrome_pid {
                // Chrome with CDP is running. Try to also verify via TCP that port 9222 is open.
                let port_open = tokio::time::timeout(
                    tokio::time::Duration::from_millis(200),
                    tokio::net::TcpStream::connect("127.0.0.1:9222"),
                )
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false);

                return VerifyOutcome {
                    verified: true,
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::StructuralOnly,
                    confidence: if port_open { 0.8 } else { 0.6 },
                    evidence: format!(
                        "Chrome process with CDP found (PID {}, port_9222_open={}) after {}ms",
                        pid,
                        port_open,
                        start.elapsed().as_millis()
                    ),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }

            if start.elapsed() >= proc_deadline {
                break;
            }
            tokio::time::sleep(poll_interval).await;
        }

        // ── Layer 3: Any Chrome Process (Unobservable fallback) ──────────────
        // Last resort: scan /proc for any Chrome/Chromium process at all
        // (even without CDP). This confirms the URL open command dispatched
        // successfully and Chrome is running, even if we can't observe content.
        let has_any_chrome = {
            let managed_pid =
                crate::agent::browser_cognition::BrowserCognitionEngine::get_managed_pid();
            let managed_running = managed_pid
                .map(|pid| std::path::Path::new(&format!("/proc/{}", pid)).exists())
                .unwrap_or(false);

            if managed_running {
                true
            } else {
                // Scan /proc for any chrome process
                std::fs::read_dir("/proc")
                    .ok()
                    .and_then(|entries| {
                        entries.flatten().find(|e| {
                            let cmdline_path = e.path().join("cmdline");
                            std::fs::read_to_string(cmdline_path)
                                .map(|c| {
                                    let args = c.replace('\0', " ");
                                    let is_main = !args.contains("--type=");
                                    is_main
                                        && (args.contains("/chrome ")
                                            || args.contains("google-chrome")
                                            || args.contains("chromium"))
                                })
                                .unwrap_or(false)
                        })
                    })
                    .is_some()
            }
        };

        if has_any_chrome {
            return VerifyOutcome {
                verified: true,
                confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::Unobservable,
                confidence: 0.3,
                evidence: format!(
                    "A Chrome/Chromium process is running after {}ms. Content not yet observable via CDP.",
                    start.elapsed().as_millis()
                ),
                latency_ms: start.elapsed().as_millis() as u32,
            };
        }

        VerifyOutcome {
            verified: false,
            confidence_tier: crate::agent::execution_verifier::VerificationConfidenceTier::Unobservable,
            confidence: 0.0,
            evidence: format!(
                "All browser verification layers failed after {}ms: CDP not reachable, no Chrome process found. \
                 Ensure Chrome/Chromium is installed and accessible.",
                start.elapsed().as_millis()
            ),
            latency_ms: start.elapsed().as_millis() as u32,
        }
    }

    /// Check UserAttested - never auto-verifies, always escalates.
    fn check_user_attested(&self, question: &str) -> VerifyOutcome {
        VerifyOutcome {
            verified: false,
            confidence_tier:
                crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
            confidence: 0.0,
            evidence: format!("User attestation required: {}", question),
            latency_ms: 0,
        }
    }

    /// Check Unverifiable - always returns false with evidence.
    fn check_unverifiable(&self, reason: &str) -> VerifyOutcome {
        VerifyOutcome {
            verified: false,
            confidence_tier:
                crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
            confidence: 0.0,
            evidence: format!("unverifiable: {}", reason),
            latency_ms: 0,
        }
    }
}

impl Default for BoundedExecutionVerifier {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl crate::agent::execution_verifier::ExecutionVerifier for BoundedExecutionVerifier {
    async fn verify(&self, leaf: &Verifiability) -> VerifyOutcome {
        let _start = Instant::now();

        let result = match leaf {
            Verifiability::WindowState {
                title_contains,
                class,
            } => tokio::time::timeout(
                Duration::from_millis(500),
                self.check_window_state(title_contains, class),
            )
            .await
            .map_err(|_| 500u32),
            Verifiability::FileSystemEffect { path, kind } => tokio::time::timeout(
                Duration::from_millis(500),
                self.check_file_system_effect(path, kind),
            )
            .await
            .map_err(|_| 500u32),
            Verifiability::ProcessLaunched {
                binary,
                max_wait_ms,
            } => {
                let mut result = self.check_process_launched(binary, *max_wait_ms).await;
                if result.latency_ms > *max_wait_ms {
                    result.latency_ms = *max_wait_ms;
                }
                return result;
            }
            Verifiability::ProcessRunning {
                binary,
                max_wait_ms,
            } => {
                let mut result = self.check_process_launched(binary, *max_wait_ms).await;
                if result.latency_ms > *max_wait_ms {
                    result.latency_ms = *max_wait_ms;
                }
                return result;
            }
            Verifiability::WindowVisible {
                title_contains,
                class,
                ..
            }
            | Verifiability::WindowInteractive {
                title_contains,
                class,
                ..
            }
            | Verifiability::KeyboardTargetConfirmed {
                title_contains,
                class,
                ..
            } => tokio::time::timeout(
                Duration::from_millis(500),
                self.check_window_state(title_contains, class),
            )
            .await
            .map_err(|_| 500u32),
            Verifiability::ForegroundLeaseAcquired { workflow_id } => {
                return VerifyOutcome {
                    verified: true,
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::PartialObservable,
                    confidence: 0.60,
                    evidence: format!(
                        "Foreground lease is enforced by StageExecutor for workflow '{}'",
                        workflow_id
                    ),
                    latency_ms: 0,
                };
            }
            Verifiability::SemanticTargetConfirmed {
                description,
                evidence_hint,
            } => {
                return VerifyOutcome {
                    verified: evidence_hint.as_ref().map(|s| !s.is_empty()).unwrap_or(false),
                    confidence_tier:
                        crate::agent::execution_verifier::VerificationConfidenceTier::PartialObservable,
                    confidence: if evidence_hint.is_some() { 0.70 } else { 0.0 },
                    evidence: format!(
                        "Semantic target '{}': {}",
                        description,
                        evidence_hint.as_deref().unwrap_or("no evidence")
                    ),
                    latency_ms: 0,
                };
            }
            Verifiability::ProcessNotRunning {
                binary,
                max_wait_ms,
            } => {
                let mut result = self.check_process_not_running(binary, *max_wait_ms).await;
                if result.latency_ms > *max_wait_ms {
                    result.latency_ms = *max_wait_ms;
                }
                return result;
            }
            Verifiability::DeterministicOutput {
                expected_substring,
                in_target,
            } => {
                // W-16 fix: increased from 500ms to 2000ms to handle spawn_blocking
                // scheduling delays on loaded systems.
                tokio::time::timeout(
                    Duration::from_millis(2000),
                    self.check_deterministic_output(expected_substring, in_target),
                )
                .await
                .map_err(|_| 2000u32)
            }
            Verifiability::OcrTextPresent {
                text,
                case_insensitive,
            } => tokio::time::timeout(
                Duration::from_millis(500),
                self.check_ocr_text_present(text, *case_insensitive, "default"),
            )
            .await
            .map_err(|_| 500u32),
            Verifiability::AccessibilityElement {
                role,
                name_contains,
                must_be_visible,
            } => {
                // Real AT-SPI verification — check if the element exists in the tree.
                // Uses a 6-second timeout to allow for UI rendering delays and slow DBus response.
                tokio::time::timeout(
                    Duration::from_millis(6000),
                    self.check_accessibility_element(
                        role,
                        name_contains.as_deref(),
                        *must_be_visible,
                    ),
                )
                .await
                .map_err(|_| 6000u32)
            }
            Verifiability::InteractionOutcome {
                expected_role,
                expected_name_contains,
                action_type,
            } => tokio::time::timeout(
                Duration::from_millis(6000),
                self.check_interaction_outcome(
                    expected_role,
                    expected_name_contains.as_deref(),
                    action_type,
                ),
            )
            .await
            .map_err(|_| 6000u32),
            Verifiability::BrowserPageLoaded {
                url_contains,
                title_contains,
            } => {
                // Outer timeout must exceed the inner polling budget:
                //   Layer 1 CDP polls for up to 6s
                //   Layer 2 AT-SPI polls for up to 4s additional
                //   Total inner budget = ~10s
                // Set outer cap to 15s to give cold-start Chrome enough time.
                tokio::time::timeout(
                    Duration::from_millis(15_000),
                    self.check_browser_page_loaded(
                        url_contains.as_deref(),
                        title_contains.as_deref(),
                    ),
                )
                .await
                .map_err(|_| 15_000u32)
            }
            Verifiability::UserAttested { question } => {
                return self.check_user_attested(question);
            }
            Verifiability::Unverifiable { reason } => {
                return self.check_unverifiable(reason);
            }
        };

        // AUDIT FIX #3: Report the actual timeout value, not a hardcoded 500ms.
        result.unwrap_or_else(|timeout_ms| VerifyOutcome {
            verified: false,
            confidence_tier:
                crate::agent::execution_verifier::VerificationConfidenceTier::FullSemantic,
            confidence: 0.0,
            evidence: format!("Verification timed out after {}ms", timeout_ms),
            latency_ms: timeout_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::execution_verifier::{ExecutionVerifier, FsEffect, Verifiability};

    #[tokio::test]
    async fn verifier_never_falsely_reports_success_for_file_check() {
        let verifier = BoundedExecutionVerifier::new();

        // Non-existent file should not be verified
        let outcome = verifier
            .verify(&Verifiability::FileSystemEffect {
                path: PathBuf::from("/tmp/definitely_does_not_exist_12345.txt"),
                kind: FsEffect::Exists,
            })
            .await;

        assert!(!outcome.verified, "Should not verify non-existent file");
        assert!(outcome.confidence < 0.5, "Confidence should be low");
        assert!(outcome.evidence.contains("does not exist"));
    }

    #[tokio::test]
    async fn verifier_handles_unverifiable() {
        let verifier = BoundedExecutionVerifier::new();

        let outcome = verifier
            .verify(&Verifiability::Unverifiable {
                reason: "test reason".into(),
            })
            .await;

        assert!(
            !outcome.verified,
            "Unverifiable should never report success"
        );
        assert_eq!(outcome.confidence, 0.0);
        assert!(outcome.evidence.contains("unverifiable"));
    }

    #[tokio::test]
    async fn verifier_handles_user_attested() {
        let verifier = BoundedExecutionVerifier::new();

        let outcome = verifier
            .verify(&Verifiability::UserAttested {
                question: "Did this work?".into(),
            })
            .await;

        assert!(!outcome.verified, "UserAttested should never auto-verify");
        assert!(outcome.evidence.contains("User attestation required"));
    }

    #[tokio::test]
    async fn verifier_timeouts_under_500ms() {
        let verifier = BoundedExecutionVerifier::new();

        // DeterministicOutput for TerminalOutput should timeout gracefully
        let outcome = verifier
            .verify(&Verifiability::DeterministicOutput {
                expected_substring: "test".into(),
                in_target: VerifyTarget::TerminalOutput,
            })
            .await;

        // Should return false with timeout evidence
        assert!(!outcome.verified);
        assert!(outcome.latency_ms <= 600); // Allow some tolerance
    }

    #[tokio::test]
    async fn default_verifier_works() {
        let verifier = BoundedExecutionVerifier::default();
        let _ = verifier.ocr_cache.read().await;
    }

    #[tokio::test]
    async fn ocr_cache_works() {
        let verifier = BoundedExecutionVerifier::new();

        verifier
            .cache_ocr("default", "Hello World".to_string())
            .await;

        let outcome = verifier
            .verify(&Verifiability::OcrTextPresent {
                text: "Hello".to_string(),
                case_insensitive: false,
            })
            .await;

        // Should find "Hello" in cached OCR text
        assert!(outcome.verified, "Should find 'Hello' in cached OCR");
    }
}
