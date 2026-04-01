//! Audio recorder using system tools (SoX rec / ALSA arecord).

use std::process::Stdio;
use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing;
use super::types::*;

pub fn has_command(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

pub fn check_recording_availability() -> RecordingAvailability {
    if has_command("rec") {
        return RecordingAvailability { available: true, reason: None };
    }
    if cfg!(target_os = "linux") && has_command("arecord") {
        return RecordingAvailability { available: true, reason: None };
    }
    let hint = if cfg!(target_os = "macos") {
        "Install SoX with: brew install sox"
    } else if cfg!(target_os = "linux") {
        "Install SoX with: sudo apt-get install sox"
    } else {
        "Voice recording requires SoX"
    };
    RecordingAvailability { available: false, reason: Some(hint.to_string()) }
}

pub fn detect_backend() -> Option<RecordingBackend> {
    if has_command("rec") { return Some(RecordingBackend::Sox); }
    if cfg!(target_os = "linux") && has_command("arecord") { return Some(RecordingBackend::Arecord); }
    None
}

pub struct ActiveRecording { child: Option<Child> }

impl ActiveRecording {
    pub fn stop(&mut self) {
        if let Some(ref mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}

impl Drop for ActiveRecording {
    fn drop(&mut self) { self.stop(); }
}

fn sox_args(config: &VoiceConfig) -> Vec<String> {
    let mut args = vec![
        "-q".into(), "--buffer".into(), "1024".into(),
        "-t".into(), "raw".into(),
        "-r".into(), RECORDING_SAMPLE_RATE.to_string(),
        "-e".into(), "signed".into(), "-b".into(), "16".into(),
        "-c".into(), RECORDING_CHANNELS.to_string(), "-".into(),
    ];
    if config.silence_detection {
        args.extend([
            "silence".into(), "1".into(), "0.1".into(),
            format!("{}%", config.silence_threshold_pct),
            "1".into(),
            format!("{}", config.silence_duration_secs),
            format!("{}%", config.silence_threshold_pct),
        ]);
    }
    args
}

fn arecord_args() -> Vec<String> {
    vec![
        "-f".into(), "S16_LE".into(),
        "-r".into(), RECORDING_SAMPLE_RATE.to_string(),
        "-c".into(), RECORDING_CHANNELS.to_string(),
        "-t".into(), "raw".into(), "-q".into(), "-".into(),
    ]
}

pub async fn start_recording(
    config: &VoiceConfig,
) -> Result<(ActiveRecording, mpsc::Receiver<Vec<u8>>)> {
    let backend = detect_backend()
        .context("no recording backend available (install sox or arecord)")?;
    let (cmd, args) = match backend {
        RecordingBackend::Sox => ("rec".to_string(), sox_args(config)),
        RecordingBackend::Arecord => ("arecord".to_string(), arecord_args()),
    };
    tracing::debug!(backend = ?backend, "starting audio recording");
    let mut child = Command::new(&cmd)
        .args(&args)
        .stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null())
        .spawn().with_context(|| format!("spawning {cmd}"))?;
    let stdout = child.stdout.take().context("no stdout")?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
    tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(stdout);
        let mut buf = vec![0u8; 4096];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => { if tx.send(buf[..n].to_vec()).await.is_err() { break; } }
                Err(_) => break,
            }
        }
    });
    Ok((ActiveRecording { child: Some(child) }, rx))
}

pub async fn speak(text: &str) -> Result<()> {
    let (cmd, args): (&str, Vec<String>) = if cfg!(target_os = "macos") {
        ("say", vec![text.to_string()])
    } else if has_command("espeak") {
        ("espeak", vec![text.to_string()])
    } else {
        return Ok(());
    };
    Command::new(cmd).args(&args).stdout(Stdio::null()).stderr(Stdio::null())
        .status().await.with_context(|| format!("running {cmd}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sox_args_with_silence() {
        let config = VoiceConfig::default();
        let args = sox_args(&config);
        assert!(args.contains(&"-q".to_string()));
        assert!(args.contains(&"silence".to_string()));
    }

    #[test]
    fn test_sox_args_without_silence() {
        let mut config = VoiceConfig::default();
        config.silence_detection = false;
        assert!(!sox_args(&config).contains(&"silence".to_string()));
    }

    #[test]
    fn test_arecord_args() {
        let args = arecord_args();
        assert!(args.contains(&"S16_LE".to_string()));
        assert!(args.contains(&"raw".to_string()));
    }

    #[test]
    fn test_check_recording_availability() {
        let _ = check_recording_availability();
    }
}
