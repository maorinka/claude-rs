//! Binary file extension detection and file type constants.
//!
//! These are used for text-based operations to skip files that cannot
//! be meaningfully compared or read as text.

use once_cell::sync::Lazy;
use std::collections::HashSet;

/// Binary file extensions to skip for text-based operations.
/// These files cannot be meaningfully compared as text and are often large.
///
/// Note: PDF and supported image formats (png, jpg, jpeg, gif, webp) are
/// intentionally included here but should be excluded at the call site
/// when the tool supports native rendering of those formats.
static BINARY_EXTENSIONS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        // Images
        "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "tiff", "tif", // Videos
        "mp4", "mov", "avi", "mkv", "webm", "wmv", "flv", "m4v", "mpeg", "mpg", // Audio
        "mp3", "wav", "ogg", "flac", "aac", "m4a", "wma", "aiff", "opus", // Archives
        "zip", "tar", "gz", "bz2", "7z", "rar", "xz", "z", "tgz", "iso",
        // Executables/binaries
        "exe", "dll", "so", "dylib", "bin", "o", "a", "obj", "lib", "app", "msi", "deb", "rpm",
        // Documents (PDF is here; tools should exclude it when they can render PDFs)
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp",
        // Fonts
        "ttf", "otf", "woff", "woff2", "eot", // Bytecode / VM artifacts
        "pyc", "pyo", "class", "jar", "war", "ear", "node", "wasm", "rlib",
        // Database files
        "sqlite", "sqlite3", "db", "mdb", "idx", // Design / 3D
        "psd", "ai", "eps", "sketch", "fig", "xd", "blend", "3ds", "max", // Flash
        "swf", "fla", // Lock/profiling data
        "lockb", "dat", "data",
    ]
    .into_iter()
    .collect()
});

/// Image extensions that the API can accept as multimodal input.
pub const API_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Number of bytes to read for binary content detection.
pub const BINARY_CHECK_SIZE: usize = 8192;

/// Threshold proportion of non-printable characters above which content
/// is classified as binary.
pub const BINARY_CONTENT_THRESHOLD: f64 = 0.1;

/// Check if a file path has a binary extension.
///
/// Extracts the extension from the path, lowercases it, and checks
/// against the known binary extensions set.
pub fn has_binary_extension(file_path: &str) -> bool {
    std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| BINARY_EXTENSIONS.contains(ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Check if a file extension indicates an API-supported image format.
pub fn is_api_image_extension(ext: &str) -> bool {
    API_IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

/// Check if a buffer contains binary content by looking for null bytes
/// or a high proportion of non-printable characters.
///
/// Examines the first [`BINARY_CHECK_SIZE`] bytes (or the full buffer
/// if smaller). A null byte is an immediate indicator of binary content.
/// Otherwise, if more than 10% of the bytes are non-printable and
/// non-whitespace, the content is classified as binary.
pub fn is_binary_content(buffer: &[u8]) -> bool {
    let check_size = buffer.len().min(BINARY_CHECK_SIZE);
    if check_size == 0 {
        return false;
    }

    let mut non_printable = 0usize;
    for &byte in &buffer[..check_size] {
        // Null byte is a strong indicator of binary
        if byte == 0 {
            return true;
        }
        // Count non-printable, non-whitespace bytes.
        // Printable ASCII is 32-126, plus common whitespace (9=tab, 10=newline, 13=CR).
        if byte < 32 && byte != 9 && byte != 10 && byte != 13 {
            non_printable += 1;
        }
    }

    // If more than 10% non-printable, likely binary
    (non_printable as f64 / check_size as f64) > BINARY_CONTENT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_binary_extension() {
        assert!(has_binary_extension("/foo/bar.zip"));
        assert!(has_binary_extension("/foo/bar.exe"));
        assert!(has_binary_extension("/foo/bar.pdf"));
        assert!(has_binary_extension("/foo/bar.wasm"));
        assert!(has_binary_extension("/foo/bar.sqlite3"));
        // Case insensitive
        assert!(has_binary_extension("/foo/bar.MP3"));
        assert!(has_binary_extension("/foo/bar.Zip"));
        // Non-binary
        assert!(!has_binary_extension("/foo/bar.txt"));
        assert!(!has_binary_extension("/foo/bar.rs"));
        assert!(!has_binary_extension("/foo/bar.ts"));
        assert!(!has_binary_extension("/foo/bar.json"));
        assert!(!has_binary_extension("/foo/bar.html"));
    }

    #[test]
    fn test_is_api_image_extension() {
        assert!(is_api_image_extension("png"));
        assert!(is_api_image_extension("jpg"));
        assert!(is_api_image_extension("jpeg"));
        assert!(is_api_image_extension("gif"));
        assert!(is_api_image_extension("webp"));
        assert!(is_api_image_extension("PNG")); // case insensitive
        assert!(!is_api_image_extension("bmp")); // not API-supported
        assert!(!is_api_image_extension("tiff"));
        assert!(!is_api_image_extension("svg"));
    }

    #[test]
    fn test_is_binary_content() {
        // Empty buffer is not binary
        assert!(!is_binary_content(b""));
        // Plain text is not binary
        assert!(!is_binary_content(b"Hello, world!\nThis is text.\n"));
        // Text with tabs
        assert!(!is_binary_content(b"col1\tcol2\tcol3\n"));
        // Null byte is binary
        assert!(is_binary_content(b"Hello\x00world"));
        // High proportion of non-printable = binary
        let mut binary_buf = vec![0x01u8; 100];
        binary_buf[0] = b'H'; // Add one printable char so it's not 100% non-printable
        assert!(is_binary_content(&binary_buf));
        // Low proportion of non-printable = not binary
        let mut mostly_text = vec![b'A'; 100];
        mostly_text[0] = 0x01; // Only 1% non-printable
        assert!(!is_binary_content(&mostly_text));
    }

    #[test]
    fn test_extension_count_matches_ts() {
        // The TS source has categories of extensions. Verify we have enough.
        // Count unique extensions in our set.
        let count = BINARY_EXTENSIONS.len();
        // TS has: images(9) + videos(10) + audio(9) + archives(10) +
        // executables(13) + documents(10) + fonts(5) + bytecode(9) +
        // database(5) + design(9) + flash(2) + lock(3) = 94 extensions
        // We should have all of them.
        assert!(
            count >= 90,
            "Expected at least 90 binary extensions, got {count}"
        );
    }
}
