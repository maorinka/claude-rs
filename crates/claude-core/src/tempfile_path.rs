//! Generate a temp-file path with an optional content-stable ID.
//!
//! Port of TS `src/utils/tempfile.ts`. When `content_hash` is set,
//! the returned path is stable across processes — any caller with
//! the same `content_hash` gets the same path. Use that mode when
//! the path flows into Anthropic API content (sandbox deny lists in
//! tool descriptions, etc.) because a random UUID on every
//! subprocess spawn would invalidate the prompt-cache prefix.
//!
//! Does NOT create the file — callers write to the returned path
//! themselves. This mirrors the TS semantics.

use sha2::{Digest, Sha256};
use std::path::PathBuf;
use uuid::Uuid;

/// Options controlling the generated filename's stable-ID mode.
#[derive(Debug, Default, Clone)]
pub struct TempFileOptions<'a> {
    /// When `Some`, the ID is the first 16 hex chars of
    /// `SHA-256(content_hash)`. When `None`, a random v4 UUID.
    pub content_hash: Option<&'a str>,
    /// Override `std::env::temp_dir()` — useful for tests.
    pub tmpdir_override: Option<PathBuf>,
}

/// Generate a temp-file path. Defaults: prefix=`"claude-prompt"`,
/// extension=`".md"`. The extension is joined with a leading `.` if
/// the caller didn't include one.
pub fn generate_temp_file_path(
    prefix: &str,
    extension: &str,
    options: &TempFileOptions,
) -> PathBuf {
    let id = match options.content_hash {
        Some(seed) => {
            let mut hasher = Sha256::new();
            hasher.update(seed.as_bytes());
            let digest = hasher.finalize();
            let mut s = String::with_capacity(16);
            for &b in &digest[..8] {
                s.push_str(&format!("{:02x}", b));
            }
            s
        }
        None => Uuid::new_v4().to_string(),
    };

    let ext = if extension.is_empty() || extension.starts_with('.') {
        extension.to_string()
    } else {
        format!(".{extension}")
    };
    let base = options
        .tmpdir_override
        .clone()
        .unwrap_or_else(std::env::temp_dir);
    base.join(format!("{prefix}-{id}{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable() {
        let tmp = std::env::temp_dir();
        let opt = TempFileOptions {
            content_hash: Some("stable-seed"),
            tmpdir_override: Some(tmp.clone()),
        };
        let a = generate_temp_file_path("pfx", ".md", &opt);
        let b = generate_temp_file_path("pfx", ".md", &opt);
        assert_eq!(a, b);
        assert!(a.starts_with(&tmp));
        assert!(a.to_string_lossy().ends_with(".md"));
    }

    #[test]
    fn random_id_differs_between_calls() {
        let opt = TempFileOptions::default();
        let a = generate_temp_file_path("pfx", ".md", &opt);
        let b = generate_temp_file_path("pfx", ".md", &opt);
        assert_ne!(a, b);
    }

    #[test]
    fn different_content_hashes_give_different_paths() {
        let opt_a = TempFileOptions {
            content_hash: Some("a"),
            ..Default::default()
        };
        let opt_b = TempFileOptions {
            content_hash: Some("b"),
            ..Default::default()
        };
        let a = generate_temp_file_path("pfx", ".md", &opt_a);
        let b = generate_temp_file_path("pfx", ".md", &opt_b);
        assert_ne!(a, b);
    }

    #[test]
    fn content_hash_id_is_16_hex_chars() {
        let tmp = std::env::temp_dir();
        let opt = TempFileOptions {
            content_hash: Some("seed-xyz"),
            tmpdir_override: Some(tmp.clone()),
        };
        let path = generate_temp_file_path("p", ".md", &opt);
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        // p-<16hex>.md
        assert_eq!(name.len(), 1 + 1 + 16 + 3);
        let id = &name["p-".len()..name.len() - ".md".len()];
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn extension_without_dot_is_prefixed() {
        let opt = TempFileOptions {
            content_hash: Some("x"),
            ..Default::default()
        };
        let p = generate_temp_file_path("p", "txt", &opt);
        assert!(p.to_string_lossy().ends_with(".txt"));
    }

    #[test]
    fn extension_with_dot_kept_as_is() {
        let opt = TempFileOptions {
            content_hash: Some("x"),
            ..Default::default()
        };
        let p = generate_temp_file_path("p", ".log", &opt);
        assert!(p.to_string_lossy().ends_with(".log"));
    }

    #[test]
    fn empty_extension_ok() {
        let opt = TempFileOptions {
            content_hash: Some("x"),
            ..Default::default()
        };
        let p = generate_temp_file_path("bin", "", &opt);
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(!name.contains('.'));
    }
}
