pub mod audio_enhance;
pub mod capture;
pub mod metrics;
pub mod pipeline;
pub mod playback;
pub mod pre_commit_policy;
pub mod reconcile;
pub mod refiner;
#[cfg(test)]
mod refiner_integration_tests;
pub mod runtime_bridge;
pub mod runtime_telemetry;
pub mod sidecar_ipc;
pub mod sidecar_session;
pub mod sidecar_supervisor;
pub mod stt;
pub mod stt_trace;
pub mod streaming_asr;
pub mod streaming_decoder;
pub mod tier;
pub mod transcript_authority;
pub mod tts;
pub mod turn_ownership;
pub mod ux_refinement;
pub mod v2;
pub mod vad;
pub mod vad_profile;

pub use capture::{default_input_device_name, list_input_devices, AudioCapture};
pub use metrics::{MetricsBuilder, OverrunTracker, VoiceMetrics};
pub use pipeline::{VoicePipeline, VoicePipelineEvent, VoicePipelineState, VoiceTranscriptFrame};
pub use playback::{default_output_device_name, list_output_devices, AudioPlayer};
pub use pre_commit_policy::{
    enforce_pre_commit_action, guard_filesystem_write, guard_llm_generation,
    guard_network_action, guard_tool_execution, is_whitelisted_action, PolicyViolation,
    PreCommitAction, POLICY_DOC_REF,
};
pub use reconcile::{reconcile_ts_whisper, ReconcileKind, ReconcileOutcome};
pub use refiner::{RefinementResult, WhisperRefiner, REFINEMENT_PROMPT};
pub use stt::SpeechToText;
pub use tier::{VoiceTier, VoiceTierProfile};
pub use tts::TextToSpeech;
pub use vad::VoiceActivityDetector;
pub use vad_profile::{
    normalize_label, silero_threshold, tail_and_min_speech_ms, v2_capture_rms_thresholds,
    VadProfileConfig,
};
