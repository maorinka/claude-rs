//! Memory directory scan. Port of `src/memdir/memoryScan.ts`.

use std::fs;
use std::path::{Path, PathBuf};

use super::types::{parse_memory_type, MemoryType};

const MAX_MEMORY_FILES: usize = 200;
const FRONTMATTER_MAX_LINES: usize = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryHeader {
    pub filename: String,
    pub file_path: PathBuf,
    pub mtime_ms: u64,
    pub description: Option<String>,
    pub memory_type: Option<MemoryType>,
}

fn parse_frontmatter_fields(src: &str) -> (Option<String>, Option<String>) {
    // Read the first FRONTMATTER_MAX_LINES of the file and pull `description`
    // and `type` out of a YAML `--- ... ---` header.
    let trimmed = src.trim_start_matches('\u{FEFF}');
    if !trimmed.starts_with("---\n") && !trimmed.starts_with("---\r\n") {
        return (None, None);
    }
    let after_open = trimmed.split_once('\n').map(|x| x.1).unwrap_or("");
    let close = after_open
        .find("\n---\n")
        .or_else(|| after_open.find("\n---\r\n"))
        .unwrap_or(after_open.len());
    let block = &after_open[..close];
    let mut description = None;
    let mut ty = None;
    for line in block.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
            match key {
                "description" => description = Some(val),
                "type" => ty = Some(val),
                _ => {},
            }
        }
    }
    (description, ty)
}

fn read_first_lines(path: &Path, n: usize) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let mut out = String::with_capacity(raw.len().min(4096));
    for (i, line) in raw.lines().enumerate() {
        if i >= n {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    Some(out)
}

fn mtime_ms(path: &Path) -> Option<u64> {
    let md = path.metadata().ok()?;
    let t = md.modified().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// Walk `memory_dir` recursively and return headers for every `.md` file
/// except `MEMORY.md` itself. Sorted newest-first, capped at
/// `MAX_MEMORY_FILES`. Unreadable files are skipped silently.
pub fn scan_memory_files(memory_dir: &Path) -> Vec<MemoryHeader> {
    let mut headers: Vec<MemoryHeader> = Vec::new();
    walk(memory_dir, memory_dir, &mut headers);
    headers.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms));
    headers.truncate(MAX_MEMORY_FILES);
    headers
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<MemoryHeader>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ty = entry.file_type().ok();
        if ty.map(|t| t.is_dir()).unwrap_or(false) {
            walk(root, &path, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()) == Some("MEMORY.md") {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let mtime = mtime_ms(&path).unwrap_or(0);
        let first = read_first_lines(&path, FRONTMATTER_MAX_LINES).unwrap_or_default();
        let (description, ty) = parse_frontmatter_fields(&first);
        out.push(MemoryHeader {
            filename: relative,
            file_path: path,
            mtime_ms: mtime,
            description,
            memory_type: parse_memory_type(ty.as_deref()),
        });
    }
}

/// Format memory headers as a text manifest: one line per file with
/// `[type] filename (timestamp): description`. Used by the recall-selector
/// prompt in TS; retained here for future port of the selector agent.
pub fn format_memory_manifest(memories: &[MemoryHeader]) -> String {
    let mut out = String::new();
    for (i, m) in memories.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let tag = m
            .memory_type
            .map(|t| format!("[{}] ", t.as_str()))
            .unwrap_or_default();
        let iso = iso_8601(m.mtime_ms);
        match &m.description {
            Some(d) => out.push_str(&format!("- {tag}{} ({iso}): {d}", m.filename)),
            None => out.push_str(&format!("- {tag}{} ({iso})", m.filename)),
        }
    }
    out
}

/// Minimal ISO-8601 format for a ms-since-epoch timestamp.
/// Format: YYYY-MM-DDTHH:MM:SSZ (UTC).
fn iso_8601(ms: u64) -> String {
    // We avoid pulling chrono for this one format site; a few arithmetic
    // steps produce the same string chrono::DateTime::to_rfc3339 would.
    let secs = ms / 1000;
    let (y, mo, d, h, mi, s) = ymd_hms_from_unix(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn ymd_hms_from_unix(mut secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let s = (secs % 60) as u32;
    secs /= 60;
    let mi = (secs % 60) as u32;
    secs /= 60;
    let h = (secs % 24) as u32;
    let mut days = secs / 24;

    let mut y: i32 = 1970;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let yd = if leap { 366 } else { 365 };
        if days >= yd as u64 {
            days -= yd as u64;
            y += 1;
        } else {
            break;
        }
    }
    let months_len = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let mut mo: u32 = 1;
    for (i, &len) in months_len.iter().enumerate() {
        let adj = if i == 1 && leap { len + 1 } else { len };
        if days >= adj as u64 {
            days -= adj as u64;
            mo += 1;
        } else {
            break;
        }
    }
    let d = (days + 1) as u32;
    (y, mo, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scans_md_files_and_parses_frontmatter() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("user_role.md"),
            "---\ndescription: user is a staff eng\ntype: user\n---\n\nbody\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("no_frontmatter.md"), "# Heading\n\nbody\n").unwrap();
        std::fs::write(tmp.path().join("MEMORY.md"), "entrypoint should be skipped").unwrap();

        let headers = scan_memory_files(tmp.path());
        assert_eq!(headers.len(), 2);
        let user = headers
            .iter()
            .find(|h| h.filename == "user_role.md")
            .unwrap();
        assert_eq!(user.description.as_deref(), Some("user is a staff eng"));
        assert_eq!(user.memory_type, Some(MemoryType::User));
    }

    #[test]
    fn manifest_formats_entries() {
        let h = vec![MemoryHeader {
            filename: "a.md".into(),
            file_path: PathBuf::from("/tmp/a.md"),
            mtime_ms: 0,
            description: Some("test".into()),
            memory_type: Some(MemoryType::Feedback),
        }];
        let m = format_memory_manifest(&h);
        assert!(m.contains("[feedback]"));
        assert!(m.contains("a.md"));
        assert!(m.contains("test"));
    }

    #[test]
    fn iso_format_matches_epoch() {
        assert_eq!(iso_8601(0), "1970-01-01T00:00:00Z");
    }
}
