//! Deterministic root cause extraction from exit codes and stderr.
//!
//! # Design Principle: NO LLM Calls
//!
//! Root cause extraction is purely deterministic:
//! 1. Check exit code against known error codes
//! 2. Check stderr against compiled regex patterns
//! 3. If no match, return `RootCause::Unknown` with stderr snippet
//!
//! This ensures the Failure Analyzer never hallucinates reasons for failure.

use once_cell::sync::Lazy;
use regex::Regex;

use super::types::RootCause;

/// Extract a deterministic root cause from command output.
pub fn extract_root_cause(exit_code: i32, stderr: &str, stdout: &str, binary: &str) -> RootCause {
    // 1. Check exit code first (most reliable signal)
    if let Some(cause) = classify_exit_code(exit_code, stderr) {
        return cause;
    }

    // 2. Check stderr patterns
    if let Some(cause) = classify_stderr(stderr, binary) {
        return cause;
    }

    // 3. Check stdout for error indicators (some tools write errors to stdout)
    if let Some(cause) = classify_stdout(stdout, binary) {
        return cause;
    }

    // 4. Unknown — return snippet
    let snippet = if !stderr.is_empty() {
        truncate(stderr, 200)
    } else if !stdout.is_empty() {
        truncate(stdout, 200)
    } else {
        "(no output)".to_string()
    };

    RootCause::Unknown {
        stderr_snippet: snippet,
    }
}

/// Classify by exit code.
fn classify_exit_code(code: i32, stderr: &str) -> Option<RootCause> {
    match code {
        0 => None, // Success — not a failure

        // Standard error codes
        1 => {
            // Generic error — check stderr for more info
            if stderr.contains("Permission denied") || stderr.contains("Operation not permitted") {
                Some(RootCause::PermissionDenied {
                    path: extract_path_from_stderr(stderr),
                })
            } else if stderr.contains("No such file or directory") {
                Some(RootCause::StderrPattern {
                    pattern: "No such file or directory".into(),
                    category: "file_not_found".into(),
                })
            } else {
                None // Fall through to stderr analysis
            }
        }

        2 => Some(RootCause::ExitCode {
            code: 2,
            meaning: "Misuse of shell command (bad arguments)".into(),
        }),

        126 => Some(RootCause::PermissionDenied {
            path: extract_path_from_stderr(stderr),
        }),

        127 => {
            // Command not found — extract the command name
            let cmd = extract_command_not_found(stderr);
            Some(RootCause::ExitCode {
                code: 127,
                meaning: format!("Command not found: {}", cmd),
            })
        }

        128 => Some(RootCause::ExitCode {
            code: 128,
            meaning: "Invalid exit argument".into(),
        }),

        // Signals (128 + signal number)
        130 => Some(RootCause::ExitCode {
            code: 130,
            meaning: "Process terminated by SIGINT (Ctrl+C)".into(),
        }),
        137 => Some(RootCause::ExitCode {
            code: 137,
            meaning: "Process killed by SIGKILL (likely OOM)".into(),
        }),
        139 => Some(RootCause::ExitCode {
            code: 139,
            meaning: "Process crashed with SIGSEGV (segfault)".into(),
        }),
        143 => Some(RootCause::ExitCode {
            code: 143,
            meaning: "Process terminated by SIGTERM".into(),
        }),

        // Common application exit codes
        13 => Some(RootCause::PermissionDenied {
            path: extract_path_from_stderr(stderr),
        }),

        // Generic failure
        _ if code > 128 => Some(RootCause::ExitCode {
            code,
            meaning: format!("Killed by signal {}", code - 128),
        }),

        _ => None,
    }
}

/// Stderr pattern matching.
static STDERR_PATTERNS: Lazy<Vec<StderrPattern>> = Lazy::new(|| {
    vec![
        // Network errors
        StderrPattern {
            regex: r"ECONNREFUSED|Connection refused",
            category: "connection_refused",
            root_cause: |_, stderr| RootCause::NetworkUnreachable {
                target: extract_host(stderr).unwrap_or_default(),
            },
        },
        StderrPattern {
            regex: r"ETIMEDOUT|Connection timed out|connect timed out",
            category: "connection_timeout",
            root_cause: |_, _| RootCause::Timeout { seconds: 0 },
        },
        StderrPattern {
            regex: r"ENETUNREACH|Network is unreachable",
            category: "network_unreachable",
            root_cause: |_, _| RootCause::NetworkUnreachable {
                target: String::new(),
            },
        },
        StderrPattern {
            regex: r"DNS resolution failed|Name or service not known",
            category: "dns_failure",
            root_cause: |_, stderr| RootCause::NetworkUnreachable {
                target: extract_host(stderr).unwrap_or_default(),
            },
        },
        // Permission errors
        StderrPattern {
            regex: r"Permission denied|EACCES|Operation not permitted",
            category: "permission_denied",
            root_cause: |_, stderr| RootCause::PermissionDenied {
                path: extract_path_from_stderr(stderr),
            },
        },
        StderrPattern {
            regex: r"authentication failed|Auth fail|Permission denied \(publickey\)",
            category: "auth_failed",
            root_cause: |_, _| RootCause::PermissionDenied { path: None },
        },
        // Resource errors
        StderrPattern {
            regex: r"OOM|Out of memory|Cannot allocate memory|oom-killer",
            category: "oom",
            root_cause: |_, _| RootCause::ResourceExhausted {
                resource: "memory".into(),
            },
        },
        StderrPattern {
            regex: r"No space left on device|ENOSPC|disk full",
            category: "disk_full",
            root_cause: |_, _| RootCause::ResourceExhausted {
                resource: "disk".into(),
            },
        },
        StderrPattern {
            regex: r"Too many open files|EMFILE",
            category: "fd_exhausted",
            root_cause: |_, _| RootCause::ResourceExhausted {
                resource: "file_descriptors".into(),
            },
        },
        // Service errors
        StderrPattern {
            regex: r"Unit .+ not found|not-found",
            category: "service_not_found",
            root_cause: |_, stderr| RootCause::ServiceNotRunning {
                service: extract_service_name(stderr).unwrap_or_default(),
            },
        },
        StderrPattern {
            regex: r"inactive \(dead\)|service .* not running",
            category: "service_inactive",
            root_cause: |_, stderr| RootCause::ServiceNotRunning {
                service: extract_service_name(stderr).unwrap_or_default(),
            },
        },
        // Config errors
        StderrPattern {
            regex: r"(?i)syntax error in|invalid configuration|config.*error|parse error",
            category: "config_error",
            root_cause: |_, stderr| RootCause::ConfigError {
                file: extract_path_from_stderr(stderr),
                detail: truncate(stderr, 100),
            },
        },
        StderrPattern {
            regex: r"Job for .+ failed because the control process exited with error",
            category: "service_config_error",
            root_cause: |_, stderr| RootCause::ConfigError {
                file: None,
                detail: truncate(stderr, 100),
            },
        },
        // Package management
        StderrPattern {
            regex: r"Unable to locate package|E: Package .+ has no installation candidate",
            category: "package_not_found",
            root_cause: |_, stderr| RootCause::ConfigError {
                file: None,
                detail: truncate(stderr, 100),
            },
        },
        StderrPattern {
            regex: r"dpkg was interrupted|E: Could not get lock",
            category: "package_lock",
            root_cause: |_, _| RootCause::ResourceExhausted {
                resource: "package_manager_lock".into(),
            },
        },
        // Timeout
        StderrPattern {
            regex: r"timed out|timeout|TIMEOUT",
            category: "timeout",
            root_cause: |_, _| RootCause::Timeout { seconds: 0 },
        },
    ]
});

struct StderrPattern {
    regex: &'static str,
    #[allow(dead_code)]
    category: &'static str,
    root_cause: fn(&str, &str) -> RootCause,
}

fn classify_stderr(stderr: &str, _binary: &str) -> Option<RootCause> {
    for pattern in STDERR_PATTERNS.iter() {
        if let Ok(re) = Regex::new(pattern.regex) {
            if re.is_match(stderr) {
                return Some((pattern.root_cause)("", stderr));
            }
        }
    }
    None
}

fn classify_stdout(stdout: &str, binary: &str) -> Option<RootCause> {
    // Some tools write errors to stdout (e.g., curl with -S flag)
    if binary == "curl" && stdout.contains("Could not resolve host") {
        return Some(RootCause::NetworkUnreachable {
            target: extract_host(stdout).unwrap_or_default(),
        });
    }
    None
}

// ── Helper extractors ─────────────────────────────────────────────────────

fn extract_path_from_stderr(stderr: &str) -> Option<String> {
    static PATH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"['"]?(/[^\s'":]+)['"]?"#).unwrap());
    PATH_RE
        .captures(stderr)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn extract_command_not_found(stderr: &str) -> String {
    static CMD_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"command not found:\s*(\S+)|(\S+): command not found|(\S+): not found").unwrap()
    });
    CMD_RE
        .captures(stderr)
        .and_then(|c| {
            c.get(1)
                .or_else(|| c.get(2))
                .or_else(|| c.get(3))
                .map(|m| m.as_str().to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

fn extract_host(stderr: &str) -> Option<String> {
    static HOST_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?:host|Host|connect to)\s+(\S+)").unwrap());
    HOST_RE
        .captures(stderr)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn extract_service_name(stderr: &str) -> Option<String> {
    static SVC_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:Unit|service)\s+(\S+)").unwrap());
    SVC_RE
        .captures(stderr)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", &s[..max_chars])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_127_command_not_found() {
        let cause = extract_root_cause(127, "bash: nginx: command not found", "", "bash");
        assert!(matches!(cause, RootCause::ExitCode { code: 127, .. }));
        assert!(cause.description().contains("nginx"));
    }

    #[test]
    fn exit_code_126_permission_denied() {
        let cause = extract_root_cause(126, "/usr/bin/script: Permission denied", "", "bash");
        assert!(matches!(cause, RootCause::PermissionDenied { .. }));
    }

    #[test]
    fn stderr_econnrefused() {
        let cause = extract_root_cause(
            1,
            "curl: (7) Failed to connect to localhost port 8080: ECONNREFUSED",
            "",
            "curl",
        );
        assert!(matches!(cause, RootCause::NetworkUnreachable { .. }));
    }

    #[test]
    fn stderr_oom() {
        let cause = extract_root_cause(137, "Out of memory: Killed process 1234", "", "python3");
        // Exit code 137 takes priority (SIGKILL)
        assert!(matches!(cause, RootCause::ExitCode { code: 137, .. }));
    }

    #[test]
    fn stderr_disk_full() {
        let cause = extract_root_cause(1, "write error: No space left on device", "", "dd");
        assert!(matches!(cause, RootCause::ResourceExhausted { resource } if resource == "disk"));
    }

    #[test]
    fn stderr_config_error() {
        let cause = extract_root_cause(
            1,
            "nginx: [emerg] syntax error in /etc/nginx/nginx.conf:42",
            "",
            "nginx",
        );
        assert!(matches!(cause, RootCause::ConfigError { .. }));
    }

    #[test]
    fn unknown_error_returns_snippet() {
        let cause = extract_root_cause(42, "something weird happened", "", "mytool");
        assert!(matches!(cause, RootCause::Unknown { .. }));
        assert!(cause.description().contains("something weird"));
    }

    #[test]
    fn exit_code_0_is_not_a_failure() {
        let cause = extract_root_cause(0, "", "", "ls");
        // Should fall through to Unknown (since there's no error)
        assert!(matches!(cause, RootCause::Unknown { .. }));
    }
}
