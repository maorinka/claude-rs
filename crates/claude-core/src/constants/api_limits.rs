//! Anthropic API limits.
//!
//! These constants define server-side limits enforced by the Anthropic API.
//! This file is dependency-free to prevent circular imports.

// =============================================================================
// IMAGE LIMITS
// =============================================================================

/// Maximum base64-encoded image size (API enforced).
/// The API rejects images where the base64 string length exceeds this value.
/// Note: This is the base64 length, NOT raw bytes. Base64 increases size by ~33%.
pub const API_IMAGE_MAX_BASE64_SIZE: usize = 5 * 1024 * 1024; // 5 MB

/// Target raw image size to stay under base64 limit after encoding.
/// Base64 encoding increases size by 4/3, so we derive the max raw size:
/// raw_size * 4/3 = base64_size -> raw_size = base64_size * 3/4
pub const IMAGE_TARGET_RAW_SIZE: usize = (API_IMAGE_MAX_BASE64_SIZE * 3) / 4; // 3.75 MB

/// Client-side maximum width for image resizing.
///
/// The API internally resizes images larger than 1568px, but this is handled
/// server-side and does not cause errors. These client-side limits (2000px)
/// are slightly larger to preserve quality when beneficial.
pub const IMAGE_MAX_WIDTH: u32 = 2000;

/// Client-side maximum height for image resizing.
pub const IMAGE_MAX_HEIGHT: u32 = 2000;

// =============================================================================
// PDF LIMITS
// =============================================================================

/// Maximum raw PDF file size that fits within the API request limit after encoding.
/// The API has a 32MB total request size limit. Base64 encoding increases size by
/// ~33% (4/3), so 20MB raw -> ~27MB base64, leaving room for conversation context.
pub const PDF_TARGET_RAW_SIZE: usize = 20 * 1024 * 1024; // 20 MB

/// Maximum number of pages in a PDF accepted by the API.
pub const API_PDF_MAX_PAGES: u32 = 100;

/// Size threshold above which PDFs are extracted into page images
/// instead of being sent as base64 document blocks. This applies to
/// first-party API only; non-first-party always uses extraction.
pub const PDF_EXTRACT_SIZE_THRESHOLD: usize = 3 * 1024 * 1024; // 3 MB

/// Maximum PDF file size for the page extraction path. PDFs larger than
/// this are rejected to avoid processing extremely large files.
pub const PDF_MAX_EXTRACT_SIZE: usize = 100 * 1024 * 1024; // 100 MB

/// Max pages the Read tool will extract in a single call with the pages parameter.
pub const PDF_MAX_PAGES_PER_READ: u32 = 20;

/// PDFs with more pages than this get the reference treatment on @ mention
/// instead of being inlined into context.
pub const PDF_AT_MENTION_INLINE_THRESHOLD: u32 = 10;

// =============================================================================
// MEDIA LIMITS
// =============================================================================

/// Maximum number of media items (images + PDFs) allowed per API request.
/// The API rejects requests exceeding this limit with a confusing error.
/// We validate client-side to provide a clear error message.
pub const API_MAX_MEDIA_PER_REQUEST: u32 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_target_size_relationship() {
        // Target raw size should be exactly 3/4 of max base64 size
        assert_eq!(IMAGE_TARGET_RAW_SIZE, 3 * 1024 * 1024 + 3 * 1024 * 1024 / 4);
        assert_eq!(
            IMAGE_TARGET_RAW_SIZE.cmp(&API_IMAGE_MAX_BASE64_SIZE),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_pdf_limits_hierarchy() {
        // Extract threshold < target raw size < max extract size
        assert_eq!(
            PDF_EXTRACT_SIZE_THRESHOLD.cmp(&PDF_TARGET_RAW_SIZE),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            PDF_TARGET_RAW_SIZE.cmp(&PDF_MAX_EXTRACT_SIZE),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_pdf_pages_limits() {
        // Per-read limit should be less than the API max
        assert_eq!(
            PDF_MAX_PAGES_PER_READ.cmp(&API_PDF_MAX_PAGES),
            std::cmp::Ordering::Less
        );
        // Inline threshold should be less than per-read limit
        assert_eq!(
            PDF_AT_MENTION_INLINE_THRESHOLD.cmp(&PDF_MAX_PAGES_PER_READ),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_image_dimensions() {
        assert_eq!(IMAGE_MAX_WIDTH, 2000);
        assert_eq!(IMAGE_MAX_HEIGHT, 2000);
    }
}
