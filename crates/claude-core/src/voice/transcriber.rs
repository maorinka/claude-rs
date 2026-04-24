//! Speech-to-text transcription client.

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::multipart;
use serde::Deserialize;
use super::types::*;

#[derive(Debug, Deserialize)]
struct WhisperResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

pub fn build_keyterms(extra: &[String]) -> Vec<String> {
    let mut terms: Vec<String> = GLOBAL_KEYTERMS.iter().map(|s| s.to_string()).collect();
    for t in extra {
        if terms.len() >= MAX_KEYTERMS { break; }
        if !terms.contains(t) { terms.push(t.clone()); }
    }
    terms.truncate(MAX_KEYTERMS);
    terms
}

pub fn split_identifier(name: &str) -> Vec<String> {
    let mut spaced = String::with_capacity(name.len() + 8);
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch.is_uppercase() && prev_lower { spaced.push(' '); }
        spaced.push(ch);
        prev_lower = ch.is_lowercase();
    }
    spaced.split(|c: char| c == '-' || c == '_' || c == '.' || c == '/' || c.is_whitespace())
        .map(|w| w.trim().to_string())
        .filter(|w| w.len() > 2 && w.len() <= 20)
        .collect()
}

pub async fn transcribe(audio_pcm: &[u8], config: &VoiceConfig) -> Result<TranscriptionResult> {
    if audio_pcm.is_empty() {
        return Ok(TranscriptionResult { text: String::new(), is_final: true, confidence: None, detected_language: None });
    }
    let wav_data = pcm_to_wav(audio_pcm);
    let mut headers = HeaderMap::new();
    if !config.stt_api_key.is_empty() {
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {}", config.stt_api_key)).context("invalid API key")?);
    }
    let file_part = multipart::Part::bytes(wav_data).file_name("audio.wav").mime_str("audio/wav")?;
    let mut form = multipart::Form::new()
        .text("model", "whisper-1")
        .text("language", config.language.clone())
        .part("file", file_part);
    let keyterms = build_keyterms(&config.keyterms);
    if !keyterms.is_empty() { form = form.text("prompt", keyterms.join(", ")); }
    let client = reqwest::Client::new();
    let resp = client.post(&config.stt_endpoint).headers(headers).multipart(form)
        .timeout(std::time::Duration::from_secs(30)).send().await.context("STT request failed")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("STT API returned {status}: {body}");
    }
    let whisper: WhisperResponse = resp.json().await.context("parsing STT response")?;
    Ok(TranscriptionResult { text: whisper.text, is_final: true, confidence: None, detected_language: whisper.language })
}

fn pcm_to_wav(pcm: &[u8]) -> Vec<u8> {
    let sample_rate: u32 = RECORDING_SAMPLE_RATE;
    let channels: u16 = RECORDING_CHANNELS;
    let bits_per_sample: u16 = 16;
    let byte_rate: u32 = sample_rate * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align: u16 = channels * bits_per_sample / 8;
    let data_size = pcm.len() as u32;
    let file_size = 36 + data_size;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcm_to_wav_header() {
        let pcm = vec![0u8; 320];
        let wav = pcm_to_wav(&pcm);
        assert_eq!(wav.len(), 44 + 320);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn test_build_keyterms() {
        let terms = build_keyterms(&[]);
        assert!(terms.contains(&"MCP".to_string()));
        assert!(terms.len() <= MAX_KEYTERMS);
    }

    #[test]
    fn test_build_keyterms_deduplicates() {
        let extra = vec!["MCP".to_string(), "custom".to_string()];
        let terms = build_keyterms(&extra);
        assert_eq!(terms.iter().filter(|t| *t == "MCP").count(), 1);
        assert!(terms.contains(&"custom".to_string()));
    }

    #[test]
    fn test_split_identifier() {
        assert_eq!(split_identifier("camelCaseWord"), vec!["camel", "Case", "Word"]);
        assert_eq!(split_identifier("kebab-case-word"), vec!["kebab", "case", "word"]);
    }

    #[tokio::test]
    async fn test_transcribe_empty() {
        let config = VoiceConfig::default();
        let result = transcribe(&[], &config).await.unwrap();
        assert!(result.text.is_empty());
        assert!(result.is_final);
    }
}
