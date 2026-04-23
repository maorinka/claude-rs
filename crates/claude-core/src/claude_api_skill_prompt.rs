//! `/claude-api` bundled-skill prompt constants + language
//! detection helpers.
//!
//! Port of TS `src/skills/bundled/claudeApi.ts`. The full skill
//! also ships ~247 KB of SDK documentation (per-language
//! README / streaming / tool-use / batches / files-api markdown
//! blobs) that is *not* bundled into the Rust binary yet —
//! that would blow the binary size up for a feature the Rust
//! port doesn't register. When the skill is eventually wired
//! up, callers should load the docs from a data directory + use
//! [`INLINE_READING_GUIDE`] + [`detect_language`] as the routing
//! layer.
//!
//! Scope of this module:
//! - `INLINE_READING_GUIDE` — the Quick-Task-Reference block that
//!   points the model to the right doc for each task type.
//! - `DetectedLanguage` + [`LANGUAGE_INDICATORS`] — language
//!   detection lookup table.
//! - [`detect_language`] — scans a directory for indicator files.
//! - [`apply_language_to_reading_guide`] — fills `{lang}` slots.

use std::path::Path;

/// Languages the `/claude-api` skill recognizes. TS
/// `DetectedLanguage` union — `'curl'` is a catch-all for users
/// who want raw HTTP examples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetectedLanguage {
    Python,
    Typescript,
    Java,
    Go,
    Ruby,
    Csharp,
    Php,
    Curl,
}

impl DetectedLanguage {
    /// Wire name matching the TS enum (snake/kebab as spelled in
    /// the TS source).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Typescript => "typescript",
            Self::Java => "java",
            Self::Go => "go",
            Self::Ruby => "ruby",
            Self::Csharp => "csharp",
            Self::Php => "php",
            Self::Curl => "curl",
        }
    }
}

/// Indicator files / extensions for each language. Port of TS
/// `LANGUAGE_INDICATORS` in claudeApi.ts:19-28. Order preserved
/// so `detect_language` matches the TS `Object.entries` scan
/// order. An entry beginning with `.` is treated as an extension
/// suffix; anything else as an exact filename.
pub const LANGUAGE_INDICATORS: &[(DetectedLanguage, &[&str])] = &[
    (
        DetectedLanguage::Python,
        &[".py", "requirements.txt", "pyproject.toml", "setup.py", "Pipfile"],
    ),
    (
        DetectedLanguage::Typescript,
        &[".ts", ".tsx", "tsconfig.json", "package.json"],
    ),
    (
        DetectedLanguage::Java,
        &[".java", "pom.xml", "build.gradle"],
    ),
    (DetectedLanguage::Go, &[".go", "go.mod"]),
    (DetectedLanguage::Ruby, &[".rb", "Gemfile"]),
    (DetectedLanguage::Csharp, &[".cs", ".csproj"]),
    (DetectedLanguage::Php, &[".php", "composer.json"]),
    (DetectedLanguage::Curl, &[]),
];

/// Detect a language by scanning `cwd` for indicator files. Port
/// of TS `detectLanguage()` in claudeApi.ts:30-53. Returns `None`
/// when no indicator matches or `cwd` can't be read. Follows the
/// first-match-wins rule of the TS original.
pub fn detect_language(cwd: &Path) -> Option<DetectedLanguage> {
    let entries = match std::fs::read_dir(cwd) {
        Ok(it) => it,
        Err(_) => return None,
    };
    let names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    for (lang, indicators) in LANGUAGE_INDICATORS {
        if indicators.is_empty() {
            continue;
        }
        for ind in *indicators {
            if ind.starts_with('.') {
                if names.iter().any(|n| n.ends_with(ind)) {
                    return Some(*lang);
                }
            } else if names.iter().any(|n| n == ind) {
                return Some(*lang);
            }
        }
    }
    None
}

/// Quick-task routing guide. Shown inline when the `/claude-api`
/// skill is invoked so the model can pick which doc to consult
/// for the user's task. Contains a `{lang}` placeholder that
/// must be substituted via [`apply_language_to_reading_guide`]
/// before the string is sent. Verbatim port of TS
/// `INLINE_READING_GUIDE` in claudeApi.ts:96-130.
pub const INLINE_READING_GUIDE: &str = "## Reference Documentation

The relevant documentation for your detected language is included below in `<doc>` tags. Each tag has a `path` attribute showing its original file path. Use this to find the right section:

### Quick Task Reference

**Single text classification/summarization/extraction/Q&A:**
→ Refer to `{lang}/claude-api/README.md`

**Chat UI or real-time response display:**
→ Refer to `{lang}/claude-api/README.md` + `{lang}/claude-api/streaming.md`

**Long-running conversations (may exceed context window):**
→ Refer to `{lang}/claude-api/README.md` — see Compaction section

**Prompt caching / optimize caching / \"why is my cache hit rate low\":**
→ Refer to `shared/prompt-caching.md` + `{lang}/claude-api/README.md` (Prompt Caching section)

**Function calling / tool use / agents:**
→ Refer to `{lang}/claude-api/README.md` + `shared/tool-use-concepts.md` + `{lang}/claude-api/tool-use.md`

**Batch processing (non-latency-sensitive):**
→ Refer to `{lang}/claude-api/README.md` + `{lang}/claude-api/batches.md`

**File uploads across multiple requests:**
→ Refer to `{lang}/claude-api/README.md` + `{lang}/claude-api/files-api.md`

**Agent with built-in tools (file/web/terminal) (Python & TypeScript only):**
→ Refer to `{lang}/agent-sdk/README.md` + `{lang}/agent-sdk/patterns.md`

**Error handling:**
→ Refer to `shared/error-codes.md`

**Latest docs via WebFetch:**
→ Refer to `shared/live-sources.md` for URLs";

/// Substitute `{lang}` in [`INLINE_READING_GUIDE`]. Port of the
/// TS `INLINE_READING_GUIDE.replace(/\{lang\}/g, lang)` pattern.
/// Pass `"unknown"` when no language was detected — that's what
/// TS does at claudeApi.ts:157.
pub fn apply_language_to_reading_guide(lang: &str) -> String {
    INLINE_READING_GUIDE.replace("{lang}", lang)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn reading_guide_has_quick_reference_anchors() {
        assert!(INLINE_READING_GUIDE.starts_with("## Reference Documentation"));
        assert!(INLINE_READING_GUIDE.contains("### Quick Task Reference"));
        // Sample routing rules must survive.
        assert!(INLINE_READING_GUIDE.contains("streaming.md"));
        assert!(INLINE_READING_GUIDE.contains("tool-use.md"));
        assert!(INLINE_READING_GUIDE.contains("Prompt caching"));
    }

    #[test]
    fn apply_language_fills_every_slot() {
        let filled = apply_language_to_reading_guide("python");
        assert!(!filled.contains("{lang}"));
        assert!(filled.contains("python/claude-api/README.md"));
        assert!(filled.contains("python/agent-sdk/patterns.md"));
    }

    #[test]
    fn apply_language_handles_unknown() {
        let u = apply_language_to_reading_guide("unknown");
        // TS passes literal "unknown" when no language detected
        // (claudeApi.ts:157).
        assert!(u.contains("unknown/claude-api/README.md"));
    }

    #[test]
    fn detect_language_returns_none_for_missing_dir() {
        let missing = Path::new("/does/not/exist/xyz-42");
        assert_eq!(detect_language(missing), None);
    }

    #[test]
    fn detect_language_by_exact_filename() {
        let tmp = tempfile::tempdir().unwrap();
        File::create(tmp.path().join("go.mod")).unwrap();
        assert_eq!(detect_language(tmp.path()), Some(DetectedLanguage::Go));
    }

    #[test]
    fn detect_language_by_extension_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        File::create(tmp.path().join("main.py")).unwrap();
        assert_eq!(detect_language(tmp.path()), Some(DetectedLanguage::Python));
    }

    #[test]
    fn detect_language_respects_indicator_order() {
        // TS scans LANGUAGE_INDICATORS in insertion order; python
        // comes before typescript, so a `.py` + `package.json`
        // folder resolves to python.
        let tmp = tempfile::tempdir().unwrap();
        File::create(tmp.path().join("main.py")).unwrap();
        File::create(tmp.path().join("package.json")).unwrap();
        assert_eq!(detect_language(tmp.path()), Some(DetectedLanguage::Python));
    }

    #[test]
    fn detect_language_ignores_curl_has_no_indicators() {
        // TS skips languages with empty indicators list
        // (claudeApi.ts:43). An empty tmpdir stays None even
        // though DetectedLanguage::Curl is in the table.
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(detect_language(tmp.path()), None);
    }

    #[test]
    fn language_as_str_matches_ts_keys() {
        assert_eq!(DetectedLanguage::Typescript.as_str(), "typescript");
        assert_eq!(DetectedLanguage::Csharp.as_str(), "csharp");
        assert_eq!(DetectedLanguage::Curl.as_str(), "curl");
    }
}
