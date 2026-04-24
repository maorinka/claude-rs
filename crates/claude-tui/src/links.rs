pub fn file_hyperlink(path: &str) -> String {
    let abs = if path.starts_with('/') {
        path.to_string()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path).display().to_string())
            .unwrap_or_else(|_| path.to_string())
    };
    format!("\x1b]8;;file://{}\x07{}\x1b]8;;\x07", abs, path)
}

/// Find file paths in text, returning (start, end) byte positions
pub fn find_file_paths(text: &str) -> Vec<(usize, usize)> {
    let mut results = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'/' || (bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1] == b'/') {
            let start = i;
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && bytes[i] != b','
                && bytes[i] != b';'
                && bytes[i] != b')'
                && bytes[i] != b'"'
                && bytes[i] != b'\''
            {
                i += 1;
            }
            if i - start > 2 {
                results.push((start, i));
            }
        } else {
            i += 1;
        }
    }
    results
}
