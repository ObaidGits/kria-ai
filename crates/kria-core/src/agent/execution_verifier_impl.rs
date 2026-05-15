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
                    return VerifyOutcome {
                        verified: false,
                        confidence: 0.0,
                        evidence: format!("File does not exist: {}", e),
                        latency_ms: start.elapsed().as_millis() as u32,
                    };
                }
            };

            if !metadata.is_file() {
                return VerifyOutcome {
                    verified: false,
                    confidence: 0.0,
                    evidence: "Path is not a file".into(),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }

            match kind {
                FsEffect::Exists => VerifyOutcome {
                    verified: true,
                    confidence: 0.95,
                    evidence: format!("File exists: {}", path.display()),
                    latency_ms: start.elapsed().as_millis() as u32,
                },
                FsEffect::SizeGreaterThan(min_size) => {
                    let verified = metadata.len() > min_size;
                    VerifyOutcome {
                        verified,
                        confidence: if verified { 0.95 } else { 0.1 },
                        evidence: format!(
                            "File size: {} bytes (min required: {})",
                            metadata.len(),
                            min_size
                        ),
                        latency_ms: start.elapsed().as_millis() as u32,
                    }
                }
                FsEffect::ContainsBytes(expected) => {
                    let content = match std::fs::read(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            return VerifyOutcome {
                                verified: false,
                                confidence: 0.0,
                                evidence: format!("Failed to read file: {}", e),
                                latency_ms: start.elapsed().as_millis() as u32,
                            };
                        }
                    };

                    let verified = expected.len() <= content.len()
                        && content
                            .windows(expected.len())
                            .any(|w| w == expected.as_slice());

                    VerifyOutcome {
                        verified,
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
                confidence: 0.0,
                evidence: "FileSystemEffect check panicked".into(),
                latency_ms: start.elapsed().as_millis() as u32,
            },
        }
    }

    /// Check ProcessLaunched - poll /proc for binary process.
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
                                let comm_path = pid_dir.join("comm");
                                if let Ok(comm) = std::fs::read_to_string(&comm_path) {
                                    let comm = comm.trim();
                                    if comm == binary {
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
                    confidence: 0.95,
                    evidence: format!("Process '{}' found with PID {}", binary, pid),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }

            if Instant::now() >= deadline {
                return VerifyOutcome {
                    verified: false,
                    confidence: 0.0,
                    evidence: format!("Process '{}' not found after {}ms", binary, max_wait_ms),
                    latency_ms: start.elapsed().as_millis() as u32,
                };
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Check DeterministicOutput - verify output contains expected substring.
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
                    let content = match std::fs::read_to_string(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            return VerifyOutcome {
                                verified: false,
                                confidence: 0.0,
                                evidence: format!("Failed to read file: {}", e),
                                latency_ms: start.elapsed().as_millis() as u32,
                            };
                        }
                    };

                    let verified = content.contains(&expected);
                    VerifyOutcome {
                        verified,
                        confidence: if verified { 0.95 } else { 0.1 },
                        evidence: if verified {
                            format!("Found expected output in file: {}", path.display())
                        } else {
                            format!(
                                "Expected '{}' not found in file ({} chars total)",
                                expected,
                                content.len()
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
                        confidence: 0.0,
                        evidence: "DeterministicOutput check panicked".into(),
                        latency_ms: start.elapsed().as_millis() as u32,
                    },
                }
            }
            VerifyTarget::TerminalOutput => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                evidence: "Terminal output verification requires shell integration".into(),
                latency_ms: start.elapsed().as_millis() as u32,
            },
            VerifyTarget::ActiveEditorBuffer => VerifyOutcome {
                verified: false,
                confidence: 0.0,
                evidence: "Editor buffer verification requires visual/state integration".into(),
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

        let cache = self.ocr_cache.read().await;

        let ocr_text = match cache.get(cache_key) {
            Some(t) => t,
            None => {
                return VerifyOutcome {
                    verified: false,
                    confidence: 0.0,
                    evidence: format!("No OCR cache entry for key '{}'", cache_key),
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

    /// Check UserAttested - never auto-verifies, always escalates.
    fn check_user_attested(&self, question: &str) -> VerifyOutcome {
        VerifyOutcome {
            verified: false,
            confidence: 0.0,
            evidence: format!("User attestation required: {}", question),
            latency_ms: 0,
        }
    }

    /// Check Unverifiable - always returns false with evidence.
    fn check_unverifiable(&self, reason: &str) -> VerifyOutcome {
        VerifyOutcome {
            verified: false,
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
            } => {
                tokio::time::timeout(
                    Duration::from_millis(500),
                    self.check_window_state(title_contains, class),
                )
                .await
            }
            Verifiability::FileSystemEffect { path, kind } => {
                tokio::time::timeout(
                    Duration::from_millis(500),
                    self.check_file_system_effect(path, kind),
                )
                .await
            }
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
            Verifiability::DeterministicOutput {
                expected_substring,
                in_target,
            } => {
                tokio::time::timeout(
                    Duration::from_millis(500),
                    self.check_deterministic_output(expected_substring, in_target),
                )
                .await
            }
            Verifiability::OcrTextPresent {
                text,
                case_insensitive,
            } => {
                tokio::time::timeout(
                    Duration::from_millis(500),
                    self.check_ocr_text_present(text, *case_insensitive, "default"),
                )
                .await
            }
            Verifiability::UserAttested { question } => {
                return self.check_user_attested(question);
            }
            Verifiability::Unverifiable { reason } => {
                return self.check_unverifiable(reason);
            }
        };

        result.unwrap_or_else(|_| VerifyOutcome {
            verified: false,
            confidence: 0.0,
            evidence: "Verification timed out after 500ms".into(),
            latency_ms: 500,
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
