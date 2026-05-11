//! StructuredExtractor — Deterministic field extraction from command output.
//!
//! # Design Principle: NO LLM Calls
//!
//! The WorkingSet must compress raw command output into a compact representation
//! for the 7B Planner. Previous approaches used LLM summarization, which destroys
//! exact data (error codes, file paths, IP addresses, numeric values) that the
//! Planner needs for debugging.
//!
//! This module uses pure regex + heuristics to extract structured fields:
//! - Error codes → preserved verbatim
//! - Exit codes → preserved verbatim
//! - File paths → preserved verbatim
//! - IP addresses → preserved verbatim
//! - Numeric values with context → preserved verbatim
//! - Key-value pairs → preserved verbatim
//! - Prose → truncated by line count (lowest priority)
//!
//! # Token Budget
//!
//! The WorkingSet has a 2048-token budget. When over budget:
//! 1. First: truncate raw_snippet (lowest priority)
//! 2. Then: remove oldest evidence entries
//! 3. Never: remove structured fields (error codes, IPs, etc.)

use regex::Regex;
use std::sync::OnceLock;

/// Compiled regex patterns for field extraction.
/// Uses `OnceLock` for thread-safe lazy initialization.
struct Patterns {
    /// Matches error codes: uppercase words with underscores/digits (e.g., ECONNREFUSED, 404, SIGKILL)
    error_code: Regex,
    /// Matches IPv4 addresses
    ipv4: Regex,
    /// Matches IPv6 addresses (simplified)
    #[allow(dead_code)]
    ipv6: Regex,
    /// Matches absolute file paths
    file_path: Regex,
    /// Matches key-value pairs like "CPU: 87%" or "MemFree: 1234 kB"
    numeric_kv: Regex,
    /// Matches exit/status codes in common formats
    exit_code: Regex,
    /// Matches JSON key-value pairs
    json_kv: Regex,
    /// Matches key: value lines (colon-separated)
    colon_kv: Regex,
}

static PATTERNS: OnceLock<Patterns> = OnceLock::new();

fn patterns() -> &'static Patterns {
    PATTERNS.get_or_init(|| Patterns {
        // Error codes: ECONNREFUSED, ENOENT, SIGKILL, 404, 500, etc.
        error_code: Regex::new(r"\b([A-Z][A-Z0-9_]{2,}|[45]\d{2})\b").unwrap(),
        // IPv4: 192.168.1.1
        ipv4: Regex::new(r"\b(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})\b").unwrap(),
        // IPv6: simplified (fe80::1, ::1, etc.)
        ipv6: Regex::new(r"\b([0-9a-fA-F:]{2,39})\b").unwrap(),
        // Absolute file paths: /usr/bin/foo, /etc/nginx/nginx.conf
        file_path: Regex::new(r"(/(?:[\w.\-]+/)+[\w.\-]+)").unwrap(),
        // Numeric KV: "CPU: 87%", "MemFree: 1234 kB", "Load: 1.23"
        numeric_kv: Regex::new(
            r"([\w][\w\s./]*?):\s*(\d+\.?\d*)\s*(%|MB|GB|KB|kB|ms|s|B|b|Mbps|GHz)?",
        )
        .unwrap(),
        // Exit codes: "exit code 1", "exit status 0", "status=1"
        exit_code: Regex::new(r"(?:exit|status|code)[=:\s]+(\d+)").unwrap(),
        // JSON-like: "key": "value" or "key": 123
        json_kv: Regex::new(r#""(\w+)":\s*(?:"([^"]*)"|(\d+\.?\d*))"#).unwrap(),
        // Colon KV: "Key: Value" (multiline mode to match per-line)
        colon_kv: Regex::new(r"(?m)^(\w[\w\s./\-]{0,48}):\s*(.+)$").unwrap(),
    })
}

// ─── Extracted Fields ───────────────────────────────────────────────────────

/// Structured fields extracted from a single command's output.
/// All fields preserve exact data — no summarization, no lossy compression.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ExtractedFields {
    /// Error codes found in output (e.g., "ECONNREFUSED", "404", "SIGKILL")
    pub error_codes: Vec<String>,
    /// IPv4 addresses found in output
    pub ipv4_addresses: Vec<String>,
    /// IPv6 addresses found in output (simplified detection)
    pub ipv6_addresses: Vec<String>,
    /// Absolute file paths found in output
    pub file_paths: Vec<String>,
    /// Numeric values with context (e.g., ("CPU", "87", "%"), ("MemFree", "1234", "kB"))
    pub numeric_values: Vec<(String, String, String)>,
    /// Exit/status codes found in output
    pub exit_codes: Vec<String>,
    /// Key-value pairs extracted from structured output
    pub kv_pairs: Vec<(String, String)>,
    /// Raw output truncated to max_lines (lowest priority)
    pub raw_snippet: String,
    /// Total lines in original output
    pub total_lines: usize,
    /// Whether the snippet was truncated
    pub truncated: bool,
}

// ─── StructuredEvidence ─────────────────────────────────────────────────────

/// A single piece of evidence gathered from a command execution.
/// Contains the command, its output, and the extracted structured fields.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StructuredEvidence {
    /// The command that produced this evidence (e.g., "systemctl status nginx")
    pub command: String,
    /// The target environment (e.g., "local", "vm1")
    pub target: String,
    /// Exit code of the command
    pub exit_code: i32,
    /// Structured fields extracted from stdout
    pub stdout_fields: ExtractedFields,
    /// Structured fields extracted from stderr
    pub stderr_fields: ExtractedFields,
    /// Timestamp when evidence was gathered (epoch millis)
    #[serde(default)]
    pub timestamp_epoch_ms: u64,
}

// ─── StructuredExtractor ────────────────────────────────────────────────────

/// Deterministic field extractor. NO LLM calls. Pure regex + heuristics.
pub struct StructuredExtractor {
    max_snippet_lines: usize,
}

impl StructuredExtractor {
    /// Create a new extractor with default snippet limit (20 lines).
    pub fn new() -> Self {
        Self {
            max_snippet_lines: 20,
        }
    }

    /// Create a new extractor with custom snippet limit.
    pub fn with_max_lines(max_lines: usize) -> Self {
        Self {
            max_snippet_lines: max_lines,
        }
    }

    /// Extract structured fields from raw text.
    pub fn extract(&self, text: &str) -> ExtractedFields {
        let p = patterns();
        let mut fields = ExtractedFields::default();

        let lines: Vec<&str> = text.lines().collect();
        fields.total_lines = lines.len();

        // Extract structured fields from full text (preserves exact data)
        for cap in p.error_code.captures_iter(text) {
            let code = cap[1].to_string();
            if !fields.error_codes.contains(&code) && code.len() <= 30 {
                fields.error_codes.push(code);
            }
        }

        for cap in p.ipv4.captures_iter(text) {
            let addr = cap[1].to_string();
            if !fields.ipv4_addresses.contains(&addr) {
                fields.ipv4_addresses.push(addr);
            }
        }

        for cap in p.file_path.captures_iter(text) {
            let path = cap[1].to_string();
            if path.len() > 3 && !fields.file_paths.contains(&path) {
                fields.file_paths.push(path);
            }
        }

        for cap in p.numeric_kv.captures_iter(text) {
            let key = cap[1].trim().to_string();
            let value = cap[2].to_string();
            let unit = cap
                .get(3)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let entry = (key, value, unit);
            if !fields.numeric_values.contains(&entry) {
                fields.numeric_values.push(entry);
            }
        }

        for cap in p.exit_code.captures_iter(text) {
            let code = cap[1].to_string();
            if !fields.exit_codes.contains(&code) {
                fields.exit_codes.push(code);
            }
        }

        for cap in p.json_kv.captures_iter(text) {
            let key = cap[1].to_string();
            let value = if let Some(m) = cap.get(2) {
                m.as_str().to_string()
            } else if let Some(m) = cap.get(3) {
                m.as_str().to_string()
            } else {
                continue;
            };
            let entry = (key, value);
            if !fields.kv_pairs.contains(&entry) {
                fields.kv_pairs.push(entry);
            }
        }

        for cap in p.colon_kv.captures_iter(text) {
            let key = cap[1].trim().to_string();
            let value = cap[2].trim().to_string();
            // Only add if not already captured by json_kv
            let entry = (key.clone(), value.clone());
            if !fields.kv_pairs.contains(&entry) && key.len() < 50 {
                fields.kv_pairs.push(entry);
            }
        }

        // Truncate raw snippet by line count (lowest priority)
        if lines.len() > self.max_snippet_lines {
            fields.truncated = true;
            fields.raw_snippet = lines[..self.max_snippet_lines].join("\n");
        } else {
            fields.raw_snippet = text.to_string();
        }

        fields
    }

    /// Extract structured evidence from a command execution.
    pub fn extract_evidence(
        &self,
        command: &str,
        target: &str,
        exit_code: i32,
        stdout: &str,
        stderr: &str,
    ) -> StructuredEvidence {
        StructuredEvidence {
            command: command.to_string(),
            target: target.to_string(),
            exit_code,
            stdout_fields: self.extract(stdout),
            stderr_fields: self.extract(stderr),
            timestamp_epoch_ms: epoch_millis(),
        }
    }

    /// Estimate token count for an ExtractedFields (rough: 1 token ≈ 4 chars).
    pub fn estimate_tokens(fields: &ExtractedFields) -> usize {
        let mut total = 0usize;

        // Structured fields (compact)
        total += fields
            .error_codes
            .iter()
            .map(|s| s.len() + 10)
            .sum::<usize>(); // "ERR: X\n"
        total += fields
            .ipv4_addresses
            .iter()
            .map(|s| s.len() + 5)
            .sum::<usize>();
        total += fields.file_paths.iter().map(|s| s.len() + 5).sum::<usize>();
        total += fields
            .numeric_values
            .iter()
            .map(|(k, v, u)| k.len() + v.len() + u.len() + 5)
            .sum::<usize>();
        total += fields
            .exit_codes
            .iter()
            .map(|s| s.len() + 10)
            .sum::<usize>();
        total += fields
            .kv_pairs
            .iter()
            .map(|(k, v)| k.len() + v.len() + 5)
            .sum::<usize>();

        // Raw snippet
        total += fields.raw_snippet.len();

        total / 4 // rough token estimate
    }

    /// Estimate token count for a StructuredEvidence.
    pub fn estimate_evidence_tokens(evidence: &StructuredEvidence) -> usize {
        let mut total = evidence.command.len() + evidence.target.len() + 20;
        total += Self::estimate_tokens(&evidence.stdout_fields);
        total += Self::estimate_tokens(&evidence.stderr_fields);
        total / 4
    }
}

impl Default for StructuredExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current time in milliseconds since epoch.
fn epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_error_codes() {
        let ext = StructuredExtractor::new();
        let text = "Error: ECONNREFUSED\nConnection failed with ENOENT";
        let fields = ext.extract(text);
        assert!(fields.error_codes.contains(&"ECONNREFUSED".to_string()));
        assert!(fields.error_codes.contains(&"ENOENT".to_string()));
    }

    #[test]
    fn extracts_http_status_codes() {
        let ext = StructuredExtractor::new();
        let text = "HTTP 404 Not Found\nServer returned 500";
        let fields = ext.extract(text);
        assert!(fields.error_codes.contains(&"404".to_string()));
        assert!(fields.error_codes.contains(&"500".to_string()));
    }

    #[test]
    fn extracts_ipv4_addresses() {
        let ext = StructuredExtractor::new();
        let text = "Server: 192.168.1.100\nGateway: 10.0.0.1";
        let fields = ext.extract(text);
        assert!(fields.ipv4_addresses.contains(&"192.168.1.100".to_string()));
        assert!(fields.ipv4_addresses.contains(&"10.0.0.1".to_string()));
    }

    #[test]
    fn extracts_file_paths() {
        let ext = StructuredExtractor::new();
        let text = "Config: /etc/nginx/nginx.conf\nLog: /var/log/syslog";
        let fields = ext.extract(text);
        assert!(fields.file_paths.iter().any(|p| p.contains("nginx.conf")));
        assert!(fields.file_paths.iter().any(|p| p.contains("syslog")));
    }

    #[test]
    fn extracts_numeric_values() {
        let ext = StructuredExtractor::new();
        let text = "CPU: 87%\nMemFree: 1234 kB\nLoad: 1.23";
        let fields = ext.extract(text);
        assert!(fields
            .numeric_values
            .iter()
            .any(|(k, v, u)| k == "CPU" && v == "87" && u == "%"));
        assert!(fields
            .numeric_values
            .iter()
            .any(|(k, v, u)| k == "MemFree" && v == "1234" && u == "kB"));
    }

    #[test]
    fn extracts_exit_codes() {
        let ext = StructuredExtractor::new();
        let text = "Process exited with exit code 1\nstatus=0";
        let fields = ext.extract(text);
        assert!(fields.exit_codes.contains(&"1".to_string()));
        assert!(fields.exit_codes.contains(&"0".to_string()));
    }

    #[test]
    fn extracts_colon_kv_pairs() {
        let ext = StructuredExtractor::new();
        let text = "State: active (running)\nMain PID: 1234\nMemory: 45.2M";
        let fields = ext.extract(text);
        assert!(fields
            .kv_pairs
            .iter()
            .any(|(k, v)| k == "State" && v == "active (running)"));
        assert!(fields
            .kv_pairs
            .iter()
            .any(|(k, v)| k == "Main PID" && v == "1234"));
    }

    #[test]
    fn truncates_raw_snippet() {
        let ext = StructuredExtractor::with_max_lines(5);
        let text = (0..20)
            .map(|i| format!("Line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let fields = ext.extract(&text);
        assert!(fields.truncated);
        assert_eq!(fields.raw_snippet.lines().count(), 5);
        assert_eq!(fields.total_lines, 20);
    }

    #[test]
    fn does_not_truncate_short_output() {
        let ext = StructuredExtractor::with_max_lines(20);
        let text = "Line 1\nLine 2\nLine 3";
        let fields = ext.extract(text);
        assert!(!fields.truncated);
        assert_eq!(fields.raw_snippet, text);
    }

    #[test]
    fn deduplicates_error_codes() {
        let ext = StructuredExtractor::new();
        let text = "ECONNREFUSED\nECONNREFUSED\nECONNREFUSED";
        let fields = ext.extract(text);
        assert_eq!(fields.error_codes.len(), 1);
    }

    #[test]
    fn extracts_systemctl_output() {
        let ext = StructuredExtractor::new();
        let text = r#"nginx.service - A high performance web server
     Loaded: loaded (/lib/systemd/system/nginx.service; enabled; vendor preset: enabled)
     Active: active (running) since Mon 2026-05-08 10:00:00 IST; 2h ago
       Docs: man:nginx(8)
    Process: 1234 ExecStart=/usr/sbin/nginx (code=exited, status=0/SUCCESS)
   Main PID: 1235 (nginx)
      Tasks: 5 (limit: 4915)
     Memory: 12.5M
        CPU: 123ms
     CGroup: /system.slice/nginx.service"#;

        let fields = ext.extract(text);
        // Should extract numeric values
        assert!(
            fields
                .numeric_values
                .iter()
                .any(|(k, _, _)| k.contains("Memory")),
            "Should extract Memory numeric value"
        );
        assert!(
            fields
                .numeric_values
                .iter()
                .any(|(k, _, _)| k.contains("Tasks")),
            "Should extract Tasks numeric value"
        );
        // Should extract file paths
        assert!(
            fields
                .file_paths
                .iter()
                .any(|p| p.contains("nginx.service")),
            "Should extract nginx.service path"
        );
    }

    #[test]
    fn extracts_top_output() {
        let ext = StructuredExtractor::new();
        let text = r#"top - 10:30:00 up 5 days,  3:42,  1 user,  load average: 2.50, 1.80, 1.20
Tasks: 256 total,   2 running, 254 sleeping,   0 stopped,   0 zombie
%Cpu(s): 45.2 us, 12.3 sy,  0.0 ni, 40.1 id,  1.5 wa,  0.0 hi,  0.9 si,  0.0 st
MiB Mem :  15921.4 total,   2345.6 free,   8765.4 used,   4810.4 buff/cache
MiB Swap:   2048.0 total,   2048.0 free,      0.0 used.   6543.2 avail Mem

    PID USER      PR  NI    VIRT    RES    SHR S  %CPU  %MEM     TIME+ COMMAND
   1234 root      20   0  123456  45678   1234 S  87.5   0.3   10:23.45 nginx"#;

        let fields = ext.extract(text);
        // Should extract numeric values like "load average", "%Cpu", etc.
        assert!(!fields.numeric_values.is_empty());
    }

    #[test]
    fn estimate_tokens_works() {
        let ext = StructuredExtractor::new();
        let text = "Short output";
        let fields = ext.extract(text);
        let tokens = StructuredExtractor::estimate_tokens(&fields);
        assert!(tokens > 0);
        assert!(tokens < 100); // Short text should be small
    }

    #[test]
    fn extract_evidence_combines_stdout_stderr() {
        let ext = StructuredExtractor::new();
        let evidence = ext.extract_evidence(
            "systemctl status nginx",
            "local",
            0,
            "Active: active (running)",
            "",
        );
        assert_eq!(evidence.command, "systemctl status nginx");
        assert_eq!(evidence.exit_code, 0);
        assert!(evidence
            .stdout_fields
            .kv_pairs
            .iter()
            .any(|(k, _)| k.contains("Active")));
    }
}
