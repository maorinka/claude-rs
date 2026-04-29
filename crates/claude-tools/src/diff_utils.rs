use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};

fn convert_leading_tabs_to_spaces(content: &str) -> String {
    if !content.contains('\t') {
        return content.to_string();
    }

    let mut out = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        let mut tab_count = 0usize;
        for ch in line.chars() {
            if ch == '\t' {
                tab_count += 1;
            } else {
                break;
            }
        }
        for _ in 0..tab_count {
            out.push_str("  ");
        }
        out.push_str(&line[tab_count..]);
    }
    out
}

fn diff_line(prefix: char, value: &str) -> String {
    let value = value.trim_end_matches('\n').trim_end_matches('\r');
    format!("{prefix}{value}")
}

pub(crate) fn structured_patch_for_display(file_contents: &str, new_content: &str) -> Vec<Value> {
    let old_display = convert_leading_tabs_to_spaces(file_contents);
    let new_display = convert_leading_tabs_to_spaces(new_content);
    let diff = TextDiff::from_lines(&old_display, &new_display);

    diff.grouped_ops(3)
        .into_iter()
        .filter_map(|group| {
            let mut lines = Vec::new();
            let mut old_start: Option<usize> = None;
            let mut new_start: Option<usize> = None;
            let mut old_lines = 0usize;
            let mut new_lines = 0usize;

            for op in group {
                for change in diff.iter_changes(&op) {
                    match change.tag() {
                        ChangeTag::Equal => {
                            if old_start.is_none() {
                                old_start = change.old_index().map(|idx| idx + 1);
                            }
                            if new_start.is_none() {
                                new_start = change.new_index().map(|idx| idx + 1);
                            }
                            old_lines += 1;
                            new_lines += 1;
                            lines.push(diff_line(' ', change.value()));
                        }
                        ChangeTag::Delete => {
                            if old_start.is_none() {
                                old_start = change.old_index().map(|idx| idx + 1);
                            }
                            old_lines += 1;
                            lines.push(diff_line('-', change.value()));
                        }
                        ChangeTag::Insert => {
                            if new_start.is_none() {
                                new_start = change.new_index().map(|idx| idx + 1);
                            }
                            new_lines += 1;
                            lines.push(diff_line('+', change.value()));
                        }
                    }
                }
            }

            if lines.is_empty() {
                return None;
            }

            Some(json!({
                "oldStart": old_start.unwrap_or(0),
                "oldLines": old_lines,
                "newStart": new_start.unwrap_or(0),
                "newLines": new_lines,
                "lines": lines,
            }))
        })
        .collect()
}
