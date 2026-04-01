//! Voice mode types.

use serde::{Deserialize, Serialize};

pub const RECORDING_SAMPLE_RATE: u32 = 16_000;
pub const RECORDING_CHANNELS: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    pub stt_endpoint: String,
    pub stt_api_key: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_true")]
    pub silence_detection: bool,
    #[serde(default = "default_silence_duration")]
    pub silence_duration_secs: f32,
    #[serde(default = "default_silence_threshold")]
    pub silence_threshold_pct: u8,
    #[serde(default)]
    pub keyterms: Vec<String>,
}

fn default_language() -> String { "en".into() }
fn default_true() -> bool { true }
fn default_silence_duration() -> f32 { 2.0 }
fn default_silence_threshold() -> u8 { 3 }

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt_endpoint: "https://api.openai.com/v1/audio/transcriptions".into(),
            stt_api_key: String::new(),
            language: default_language(),
            silence_detection: true,
            silence_duration_secs: default_silence_duration(),
            silence_threshold_pct: default_silence_threshold(),
            keyterms: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptionResult {
    pub text: String,
    pub is_final: bool,
    pub confidence: Option<f64>,
    pub detected_language: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingBackend { Sox, Arecord }

#[derive(Debug, Clone)]
pub struct RecordingAvailability {
    pub available: bool,
    pub reason: Option<String>,
}

pub const GLOBAL_KEYTERMS: &[&str] = &[
    "MCP", "symlink", "grep", "regex", "localhost", "codebase",
    "TypeScript", "JSON", "OAuth", "webhook", "gRPC", "dotfiles",
    "subagent", "worktree",
];

pub const MAX_KEYTERMS: usize = 50;
