//! Wake-daemon IPC (Voice System v3, Wave 9).
//!
//! The daemon and the main KRIA app communicate over an `AF_UNIX` stream
//! socket using newline-delimited JSON. The MAIN APP is the listener (binds the
//! socket); the daemon connects and writes a single [`WakeSignal`] per wake
//! event. Keeping the app as the listener means the daemon never needs elevated
//! privileges and simply no-ops (or launches the app) when nothing is listening
//! (Requirement 11.2, 11.4).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Message the daemon sends to the app when the wake phrase fires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WakeSignal {
    /// Wake phrase detected — the app should launch/focus and start a session.
    Wake {
        /// Detector confidence (0.0–1.0).
        score: f32,
        /// Detector source label (e.g. "oww").
        source: String,
        /// Unix epoch milliseconds of detection.
        ts_ms: u64,
    },
    /// Liveness ping (daemon → app), optional.
    Ping { ts_ms: u64 },
}

impl WakeSignal {
    /// Encode as a single newline-terminated JSON line.
    pub fn encode_line(&self) -> Vec<u8> {
        let mut v = serde_json::to_vec(self).unwrap_or_default();
        v.push(b'\n');
        v
    }

    /// Decode one JSON line (without the trailing newline). Used by the
    /// app-side listener (and tests).
    #[allow(dead_code)] // app-side/listener + test API
    pub fn decode_line(line: &[u8]) -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(line)?)
    }

    /// Parse all complete newline-delimited signals from a buffer, returning
    /// the decoded signals and the number of bytes consumed. The app-side
    /// listener uses this to drain a stream incrementally.
    #[allow(dead_code)] // app-side/listener + test API
    pub fn drain_lines(buf: &[u8]) -> (Vec<WakeSignal>, usize) {
        let mut out = Vec::new();
        let mut consumed = 0usize;
        for line in buf.split_inclusive(|b| *b == b'\n') {
            if line.last() != Some(&b'\n') {
                break; // incomplete trailing line
            }
            consumed += line.len();
            let trimmed = &line[..line.len() - 1];
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(sig) = WakeSignal::decode_line(trimmed) {
                out.push(sig);
            }
        }
        (out, consumed)
    }
}

/// Resolve the wake-daemon socket path.
///
/// Priority: `KRIA_WAKE_SOCK` → `${XDG_RUNTIME_DIR}/kria/wake.sock` →
/// `/tmp/kria-wake-${UID}.sock`.
pub fn resolve_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("KRIA_WAKE_SOCK") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let mut path = PathBuf::from(xdg);
        path.push("kria");
        path.push("wake.sock");
        return path;
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/kria-wake-{uid}.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_signal_roundtrip() {
        let sig = WakeSignal::Wake {
            score: 0.83,
            source: "oww".into(),
            ts_ms: 1_700_000_000_000,
        };
        let line = sig.encode_line();
        assert_eq!(*line.last().unwrap(), b'\n');
        let decoded = WakeSignal::decode_line(&line[..line.len() - 1]).unwrap();
        assert_eq!(decoded, sig);
    }

    #[test]
    fn ping_roundtrip_and_tag() {
        let sig = WakeSignal::Ping { ts_ms: 42 };
        let line = sig.encode_line();
        let s = String::from_utf8(line.clone()).unwrap();
        assert!(s.contains("\"type\":\"ping\""));
        let decoded = WakeSignal::decode_line(s.trim_end().as_bytes()).unwrap();
        assert_eq!(decoded, sig);
    }

    #[test]
    fn socket_path_env_override() {
        std::env::set_var("KRIA_WAKE_SOCK", "/tmp/custom-wake.sock");
        assert_eq!(
            resolve_socket_path(),
            PathBuf::from("/tmp/custom-wake.sock")
        );
        std::env::remove_var("KRIA_WAKE_SOCK");
    }

    #[test]
    fn drain_lines_parses_complete_and_keeps_partial() {
        let a = WakeSignal::Ping { ts_ms: 1 };
        let b = WakeSignal::Wake {
            score: 0.5,
            source: "oww".into(),
            ts_ms: 2,
        };
        let mut buf = a.encode_line();
        buf.extend_from_slice(&b.encode_line());
        // Append an incomplete trailing line (no newline).
        buf.extend_from_slice(b"{\"type\":\"pi");
        let (sigs, consumed) = WakeSignal::drain_lines(&buf);
        assert_eq!(sigs, vec![a, b]);
        // Consumed everything except the incomplete trailing fragment.
        assert_eq!(consumed, buf.len() - "{\"type\":\"pi".len());
    }
}
