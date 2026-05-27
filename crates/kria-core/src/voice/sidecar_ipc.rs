//! Sidecar IPC v0.1 — Streaming STT socket protocol (§5 ENHANCED_STT.md)
//!
//! **Normative implementation** of IPC v0.1 specification.
//!
//! ## Transport
//! - AF_UNIX stream socket
//! - Path: `${XDG_RUNTIME_DIR}/kria/stt-streamer.sock`
//! - Fallback: `/tmp/kria-stt-${UID}.sock`
//!
//! ## Framing
//! - Length-prefixed JSON: `u32_be_len` + UTF-8 JSON body
//! - Max body: 256 KiB
//! - Oversized messages MUST close connection with error
//!
//! ## Session Lifecycle
//! - Every session has exactly one `session_id` (UUID)
//! - `generation` is host-owned, monotonic per session
//! - Stale generations MUST be dropped
//!
//! ## Runtime Invariants
//! - P2-R1: One session_id per session
//! - P2-R2: Generation host-owned, monotonic
//! - P2-R3: Stale generations dropped
//! - P2-R4: Messages ≤ 256 KiB
//! - P2-R7: Heartbeat ping/5s, pong/1s

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Maximum JSON body size (256 KiB per §5.2)
pub const MAX_BODY_SIZE: usize = 256 * 1024;

/// Heartbeat ping interval (5s per §5.3)
pub const HEARTBEAT_PING_INTERVAL_MS: u64 = 5_000;

/// Heartbeat pong timeout (1s per §5.3)
pub const HEARTBEAT_PONG_TIMEOUT_MS: u64 = 1_000;

/// Max audio chunk samples (250ms @ 16kHz per §5.3)
pub const MAX_CHUNK_SAMPLES: usize = 4_000;

/// Max unprocessed audio bytes (8 MiB per §5.5)
pub const MAX_AUDIO_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Max pending messages (64 per §5.5)
pub const MAX_PENDING_MESSAGES: usize = 64;

/// Backpressure block timeout (50ms per §8)
pub const BACKPRESSURE_TIMEOUT_MS: u64 = 50;

// ─── IPC Message Schema (§5.3) ────────────────────────────────────────────

/// IPC v0.1 message envelope.
///
/// All messages are tagged unions serialized as JSON with `"type"` discriminator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcMessage {
    /// Host → Sidecar: Session open
    Hello {
        proto: String,
        session_id: String,
        sample_rate: u32,
        generation: u64,
    },
    /// Sidecar → Host: Session acknowledged
    HelloAck {
        capabilities: Vec<String>,
        max_chunk_samples: usize,
    },
    /// Host → Sidecar: Audio chunk
    Audio {
        session_id: String,
        generation: u64,
        seq: u64,
        pcm: Vec<f32>,
    },
    /// Sidecar → Host: Partial transcript
    Partial {
        session_id: String,
        generation: u64,
        seq: u64,
        text: String,
        stable: bool,
    },
    /// Host → Sidecar: Heartbeat ping
    Ping { ts_ms: u64 },
    /// Sidecar → Host: Heartbeat pong
    Pong { ts_ms: u64 },
    /// Host → Sidecar: Session close
    Bye { session_id: String, generation: u64 },
    /// Sidecar → Host: Session close acknowledged
    ByeAck,
    /// Sidecar → Host: Error
    Error { code: String, fatal: bool },
    /// Host → Sidecar: Cancel current generation (optional per §11)
    Cancel { session_id: String, generation: u64 },
}

impl IpcMessage {
    /// Returns the session_id if this message carries one.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Hello { session_id, .. }
            | Self::Audio { session_id, .. }
            | Self::Partial { session_id, .. }
            | Self::Bye { session_id, .. }
            | Self::Cancel { session_id, .. } => Some(session_id),
            _ => None,
        }
    }

    /// Returns the generation if this message carries one.
    pub fn generation(&self) -> Option<u64> {
        match self {
            Self::Hello { generation, .. }
            | Self::Audio { generation, .. }
            | Self::Partial { generation, .. }
            | Self::Bye { generation, .. }
            | Self::Cancel { generation, .. } => Some(*generation),
            _ => None,
        }
    }

    /// Returns true if this message is a heartbeat (ping/pong).
    pub fn is_heartbeat(&self) -> bool {
        matches!(self, Self::Ping { .. } | Self::Pong { .. })
    }

    /// Returns true if this message is fatal (error with fatal=true or bye).
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::Error { fatal: true, .. } | Self::Bye { .. } | Self::ByeAck
        )
    }
}

// ─── Socket Path Resolution (§5.1) ────────────────────────────────────────

/// Resolve the sidecar socket path per §5.1.
///
/// Priority:
/// 1. `${XDG_RUNTIME_DIR}/kria/stt-streamer.sock`
/// 2. `/tmp/kria-stt-${UID}.sock`
pub fn resolve_socket_path() -> PathBuf {
    if let Ok(xdg_runtime) = std::env::var("XDG_RUNTIME_DIR") {
        let mut path = PathBuf::from(xdg_runtime);
        path.push("kria");
        path.push("stt-streamer.sock");
        return path;
    }

    // Fallback: /tmp/kria-stt-${UID}.sock
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/kria-stt-{}.sock", uid))
}

/// Ensure the parent directory of the socket path exists.
pub fn ensure_socket_dir(socket_path: &PathBuf) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create socket directory: {:?}", parent))?;
    }
    Ok(())
}

/// Unlink stale socket if it exists (§5.4).
pub fn unlink_stale_socket(socket_path: &PathBuf) -> Result<()> {
    if socket_path.exists() {
        std::fs::remove_file(socket_path)
            .with_context(|| format!("failed to unlink stale socket: {:?}", socket_path))?;
        tracing::info!(path = ?socket_path, "unlinked stale socket");
    }
    Ok(())
}

// ─── Framing Layer (§5.2) ─────────────────────────────────────────────────

/// Write a length-prefixed JSON frame to the writer.
///
/// Framing: `u32_be_len` + UTF-8 JSON body.
/// Max body: 256 KiB (enforced).
pub async fn write_frame<W>(writer: &mut W, msg: &IpcMessage) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let json = serde_json::to_vec(msg).context("failed to serialize IPC message")?;

    if json.len() > MAX_BODY_SIZE {
        bail!(
            "message exceeds {} KiB: {} bytes",
            MAX_BODY_SIZE / 1024,
            json.len()
        );
    }

    let len = json.len() as u32;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .context("failed to write frame length")?;
    writer
        .write_all(&json)
        .await
        .context("failed to write frame body")?;
    writer.flush().await.context("failed to flush frame")?;

    Ok(())
}

/// Read a length-prefixed JSON frame from the reader.
///
/// Framing: `u32_be_len` + UTF-8 JSON body.
/// Max body: 256 KiB (enforced).
pub async fn read_frame<R>(reader: &mut R) -> Result<IpcMessage>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .context("failed to read frame length")?;

    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_BODY_SIZE {
        bail!(
            "message exceeds {} KiB: {} bytes",
            MAX_BODY_SIZE / 1024,
            len
        );
    }

    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .context("failed to read frame body")?;

    let msg: IpcMessage =
        serde_json::from_slice(&body).context("failed to deserialize IPC message")?;

    Ok(msg)
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_message_session_id_extraction() {
        let msg = IpcMessage::Hello {
            proto: "0.1".to_string(),
            session_id: "test-session".to_string(),
            sample_rate: 16000,
            generation: 0,
        };
        assert_eq!(msg.session_id(), Some("test-session"));
        assert_eq!(msg.generation(), Some(0));
    }

    #[test]
    fn ipc_message_heartbeat_detection() {
        let ping = IpcMessage::Ping { ts_ms: 1000 };
        assert!(ping.is_heartbeat());
        assert!(!ping.is_fatal());

        let pong = IpcMessage::Pong { ts_ms: 1000 };
        assert!(pong.is_heartbeat());
        assert!(!pong.is_fatal());
    }

    #[test]
    fn ipc_message_fatal_detection() {
        let error = IpcMessage::Error {
            code: "test".to_string(),
            fatal: true,
        };
        assert!(error.is_fatal());

        let bye = IpcMessage::Bye {
            session_id: "test".to_string(),
            generation: 0,
        };
        assert!(bye.is_fatal());
    }

    #[test]
    fn ipc_message_serialization() {
        let msg = IpcMessage::Hello {
            proto: "0.1".to_string(),
            session_id: "test-session".to_string(),
            sample_rate: 16000,
            generation: 0,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"hello"#));
        assert!(json.contains(r#""proto":"0.1"#));
        assert!(json.contains(r#""session_id":"test-session"#));

        let deserialized: IpcMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn socket_path_resolution() {
        let path = resolve_socket_path();
        assert!(
            path.to_string_lossy().contains("kria")
                && path.to_string_lossy().contains("stt-streamer.sock")
        );
    }

    #[tokio::test]
    async fn framing_roundtrip() {
        let msg = IpcMessage::Ping { ts_ms: 12345 };

        let mut buf = Vec::new();
        write_frame(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded = read_frame(&mut cursor).await.unwrap();

        assert_eq!(decoded, msg);
    }

    #[tokio::test]
    async fn framing_rejects_oversized() {
        // Create a message that will exceed 256 KiB when serialized
        let huge_pcm = vec![0.0f32; 100_000]; // ~400 KB
        let msg = IpcMessage::Audio {
            session_id: "test".to_string(),
            generation: 0,
            seq: 0,
            pcm: huge_pcm,
        };

        let mut buf = Vec::new();
        let result = write_frame(&mut buf, &msg).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds 256 KiB"));
    }

    #[tokio::test]
    async fn framing_rejects_oversized_on_read() {
        // Manually craft an oversized frame
        let mut buf = Vec::new();
        let len = (MAX_BODY_SIZE + 1) as u32;
        buf.extend_from_slice(&len.to_be_bytes());

        let mut cursor = std::io::Cursor::new(buf);
        let result = read_frame(&mut cursor).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds 256 KiB"));
    }

    #[test]
    fn constants_match_spec() {
        assert_eq!(MAX_BODY_SIZE, 256 * 1024, "§5.2");
        assert_eq!(HEARTBEAT_PING_INTERVAL_MS, 5_000, "§5.3");
        assert_eq!(HEARTBEAT_PONG_TIMEOUT_MS, 1_000, "§5.3");
        assert_eq!(MAX_CHUNK_SAMPLES, 4_000, "§5.3");
        assert_eq!(MAX_AUDIO_BUFFER_BYTES, 8 * 1024 * 1024, "§5.5");
        assert_eq!(MAX_PENDING_MESSAGES, 64, "§5.5");
        assert_eq!(BACKPRESSURE_TIMEOUT_MS, 50, "§8");
    }
}
