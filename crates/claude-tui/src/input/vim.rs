//! Full vim input mode implementation.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator { Delete, Change, Yank }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindType { ForwardTo, BackwardTo, ForwardBefore, BackwardAfter }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjScope { Inner, Around }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection { Forward, Backward }

#[derive(Debug, Clone, PartialEq)]
pub enum CommandState {
    Idle,
    Count { digits: String },
    Operator { op: Operator, count: usize },
    OperatorCount { op: Operator, count: usize, digits: String },
    OperatorFind { op: Operator, count: usize, find: FindType },
    OperatorTextObj { op: Operator, count: usize, scope: TextObjScope },
    Find { find: FindType, count: usize },
    G { count: usize },
    OperatorG { op: Operator, count: usize },
    Replace { count: usize },
    Indent { dir: char, count: usize },
    CommandLine { buffer: String },
    Search { direction: SearchDirection, buffer: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum VimMode {
    Insert { inserted_text: String },
    Normal { command: CommandState },
    Visual { anchor: usize },
    VisualLine { anchor_line: usize },
}

#[derive(Debug, Clone)]
pub enum RecordedChange {
    Insert { text: String },
    OperatorMotion { op: Operator, motion: String, count: usize },
    OperatorTextObj { op: Operator, obj_type: String, scope: TextObjScope, count: usize },
    OperatorFind { op: Operator, find: FindType, ch: char, count: usize },
    ReplaceChar { ch: char, count: usize },
    DeleteChar { count: usize },
    ToggleCase { count: usize },
    Indent { dir: char, count: usize },
    OpenLine { below: bool },
    Join { count: usize },
}

#[derive(Debug, Clone)]
pub struct PersistentState {
    pub last_change: Option<RecordedChange>,
    pub last_find: Option<(FindType, char)>,
    pub register: String,
    pub register_is_linewise: bool,
    pub named_registers: HashMap<char, (String, bool)>,
    pub marks: HashMap<char, usize>,
    pub last_search: Option<(SearchDirection, String)>,
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            last_change: None, last_find: None, register: String::new(),
            register_is_linewise: false, named_registers: HashMap::new(),
            marks: HashMap::new(), last_search: None,
        }
    }
}

pub const MAX_VIM_COUNT: usize = 10_000;

fn pair_for(ch: char) -> Option<(char, char)> {
    match ch {
        '(' | ')' | 'b' => Some(('(', ')')),
        '[' | ']' => Some(('[', ']')),
        '{' | '}' | 'B' => Some(('{', '}')),
        '<' | '>' => Some(('<', '>')),
        '"' => Some(('"', '"')),
        '\'' => Some(('\'', '\'')),
        '`' => Some(('`', '`')),
        _ => None,
    }
}

pub struct VimBuffer { pub text: String, pub cursor: usize }

impl VimBuffer {
    pub fn new(text: String, cursor: usize) -> Self {
        let cursor = cursor.min(text.len().saturating_sub(1).max(0));
        Self { text, cursor }
    }
    pub fn line_count(&self) -> usize { self.text.lines().count().max(1) }
    pub fn current_line(&self) -> usize { self.text[..self.cursor.min(self.text.len())].matches('\n').count() }
    pub fn line_start(&self, line: usize) -> usize {
        let mut off = 0;
        for (i, l) in self.text.split('\n').enumerate() {
            if i == line { return off; }
            off += l.len() + 1;
        }
        self.text.len()
    }
    pub fn line_end(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let rest = &self.text[start..];
        match rest.find('\n') { Some(pos) => start + pos, None => self.text.len() }
    }
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            let mut n = self.cursor - 1;
            while n > 0 && !self.text.is_char_boundary(n) { n -= 1; }
            self.cursor = n;
        }
    }
    pub fn move_right(&mut self) {
        let len = self.text.len();
        if self.cursor < len {
            let mut n = self.cursor + 1;
            while n < len && !self.text.is_char_boundary(n) { n += 1; }
            self.cursor = n.min(len.saturating_sub(1));
        }
    }
    pub fn next_word(&mut self) {
        let bytes = self.text.as_bytes(); let len = bytes.len(); let mut i = self.cursor;
        if i >= len { return; }
        while i < len && is_word_byte(bytes[i]) { i += 1; }
        while i < len && !is_word_byte(bytes[i]) && !bytes[i].is_ascii_whitespace() { i += 1; }
        while i < len && bytes[i].is_ascii_whitespace() { i += 1; }
        self.cursor = i.min(len.saturating_sub(1));
    }
    pub fn prev_word(&mut self) {
        let bytes = self.text.as_bytes();
        if self.cursor == 0 { return; }
        let mut i = self.cursor.saturating_sub(1);
        while i > 0 && bytes[i].is_ascii_whitespace() { i -= 1; }
        if i > 0 && !is_word_byte(bytes[i]) {
            while i > 0 && !is_word_byte(bytes[i]) && !bytes[i].is_ascii_whitespace() { i -= 1; }
            if is_word_byte(bytes[i]) || bytes[i].is_ascii_whitespace() { i += 1; }
        } else {
            while i > 0 && is_word_byte(bytes[i.saturating_sub(1)]) { i -= 1; }
        }
        self.cursor = i;
    }
    pub fn end_of_word(&mut self) {
        let bytes = self.text.as_bytes(); let len = bytes.len();
        let mut i = self.cursor;
        if i >= len.saturating_sub(1) { return; }
        i += 1;
        while i < len && bytes[i].is_ascii_whitespace() { i += 1; }
        if i < len && is_word_byte(bytes[i]) {
            while i + 1 < len && is_word_byte(bytes[i + 1]) { i += 1; }
        } else {
            while i + 1 < len && !is_word_byte(bytes[i + 1]) && !bytes[i + 1].is_ascii_whitespace() { i += 1; }
        }
        self.cursor = i.min(len.saturating_sub(1));
    }
    pub fn find_char(&self, ch: char, find_type: FindType, count: usize) -> Option<usize> {
        let mut found = 0;
        match find_type {
            FindType::ForwardTo | FindType::ForwardBefore => {
                let start = self.cursor + 1;
                if start >= self.text.len() { return None; }
                for (i, c) in self.text[start..].char_indices() {
                    if c == ch { found += 1; if found == count {
                        let pos = start + i;
                        return Some(if find_type == FindType::ForwardBefore { pos.saturating_sub(1).max(self.cursor) } else { pos });
                    }}
                }
                None
            }
            FindType::BackwardTo | FindType::BackwardAfter => {
                if self.cursor == 0 { return None; }
                for (i, c) in self.text[..self.cursor].char_indices().rev() {
                    if c == ch { found += 1; if found == count {
                        return Some(if find_type == FindType::BackwardAfter { (i + c.len_utf8()).min(self.cursor) } else { i });
                    }}
                }
                None
            }
        }
    }
    pub fn search(&self, pattern: &str, direction: SearchDirection) -> Option<usize> {
        if pattern.is_empty() { return None; }
        match direction {
            SearchDirection::Forward => {
                let start = (self.cursor + 1).min(self.text.len());
                self.text[start..].find(pattern).map(|i| start + i)
                    .or_else(|| self.text[..self.cursor].find(pattern))
            }
            SearchDirection::Backward => {
                self.text[..self.cursor].rfind(pattern)
                    .or_else(|| { let s = (self.cursor + 1).min(self.text.len()); self.text[s..].rfind(pattern).map(|i| s + i) })
            }
        }
    }
}

fn is_word_byte(b: u8) -> bool { b.is_ascii_alphanumeric() || b == b'_' }

// Text objects
pub fn find_text_object(text: &str, offset: usize, obj_type: char, is_inner: bool) -> Option<(usize, usize)> {
    match obj_type {
        'w' => find_word_object(text, offset, is_inner, true),
        'W' => find_word_object(text, offset, is_inner, false),
        _ => {
            if let Some((open, close)) = pair_for(obj_type) {
                if open == close { find_quote_object(text, offset, open, is_inner) }
                else { find_bracket_object(text, offset, open, close, is_inner) }
            } else { None }
        }
    }
}

fn find_word_object(text: &str, offset: usize, is_inner: bool, word_chars_only: bool) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    if offset >= bytes.len() { return None; }
    let is_wc: fn(u8) -> bool = if word_chars_only { is_word_byte } else { |b: u8| !b.is_ascii_whitespace() };
    if bytes[offset].is_ascii_whitespace() {
        let mut s = offset; while s > 0 && bytes[s-1].is_ascii_whitespace() { s -= 1; }
        let mut e = offset; while e < bytes.len() && bytes[e].is_ascii_whitespace() { e += 1; }
        return Some((s, e));
    }
    let in_word = is_wc(bytes[offset]);
    let (mut s, mut e) = (offset, offset);
    if in_word {
        while s > 0 && is_wc(bytes[s-1]) { s -= 1; }
        while e < bytes.len() && is_wc(bytes[e]) { e += 1; }
    } else {
        let ip = |b: u8| !is_word_byte(b) && !b.is_ascii_whitespace();
        while s > 0 && ip(bytes[s-1]) { s -= 1; }
        while e < bytes.len() && ip(bytes[e]) { e += 1; }
    }
    if !is_inner {
        if e < bytes.len() && bytes[e].is_ascii_whitespace() {
            while e < bytes.len() && bytes[e].is_ascii_whitespace() { e += 1; }
        } else if s > 0 && bytes[s-1].is_ascii_whitespace() {
            while s > 0 && bytes[s-1].is_ascii_whitespace() { s -= 1; }
        }
    }
    Some((s, e))
}

fn find_quote_object(text: &str, offset: usize, quote: char, is_inner: bool) -> Option<(usize, usize)> {
    let ls = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let le = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    let line = &text[ls..le];
    let pos = offset - ls;
    let positions: Vec<usize> = line.char_indices().filter(|(_, c)| *c == quote).map(|(i, _)| i).collect();
    for pair in positions.chunks(2) {
        if pair.len() == 2 && pair[0] <= pos && pos <= pair[1] {
            return if is_inner { Some((ls + pair[0] + 1, ls + pair[1])) } else { Some((ls + pair[0], ls + pair[1] + 1)) };
        }
    }
    None
}

fn find_bracket_object(text: &str, offset: usize, open: char, close: char, is_inner: bool) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let (ob, cb) = (open as u8, close as u8);
    let mut depth = 0i32; let mut start = None;
    for i in (0..=offset).rev() {
        if bytes[i] == cb && i != offset { depth += 1; }
        else if bytes[i] == ob { if depth == 0 { start = Some(i); break; } depth -= 1; }
    }
    let start = start?;
    depth = 0; let mut end = None;
    for i in (start + 1)..bytes.len() {
        if bytes[i] == ob { depth += 1; }
        else if bytes[i] == cb { if depth == 0 { end = Some(i); break; } depth -= 1; }
    }
    let end = end?;
    if is_inner { Some((start + 1, end)) } else { Some((start, end + 1)) }
}

// Motions
#[allow(dead_code)]
fn is_inclusive_motion(key: &str) -> bool { matches!(key, "e" | "E" | "$") }
#[allow(dead_code)]
fn is_linewise_motion(key: &str) -> bool { matches!(key, "j" | "k" | "G" | "gg") }

#[allow(dead_code)]
fn resolve_motion(buf: &VimBuffer, key: &str, count: usize) -> usize {
    let mut pos = buf.cursor; let text = &buf.text; let bytes = text.as_bytes(); let len = bytes.len();
    for _ in 0..count {
        pos = match key {
            "h" => pos.saturating_sub(1),
            "l" => (pos + 1).min(len.saturating_sub(1)),
            "j" => {
                let _line = text[..pos].matches('\n').count();
                let col = pos - text[..pos].rfind('\n').map_or(0, |i| i + 1);
                if let Some(nl) = text[pos..].find('\n') {
                    let ns = pos + nl + 1;
                    let ne = text[ns..].find('\n').map_or(len, |i| ns + i);
                    ns + col.min(ne - ns)
                } else { pos }
            }
            "k" => {
                if pos == 0 { return 0; }
                let col = pos - text[..pos].rfind('\n').map_or(0, |i| i + 1);
                let pe = text[..pos].rfind('\n').unwrap_or(0);
                if pe == 0 && !text.starts_with('\n') { col.min(pe) }
                else { let ps = text[..pe].rfind('\n').map_or(0, |i| i + 1); ps + col.min(pe - ps) }
            }
            "w" => { let mut b = VimBuffer::new(text.to_string(), pos); b.next_word(); b.cursor }
            "b" => { let mut b = VimBuffer::new(text.to_string(), pos); b.prev_word(); b.cursor }
            "e" => { let mut b = VimBuffer::new(text.to_string(), pos); b.end_of_word(); b.cursor }
            "W" => { let mut i = pos; while i < len && !bytes[i].is_ascii_whitespace() { i += 1; } while i < len && bytes[i].is_ascii_whitespace() { i += 1; } i.min(len.saturating_sub(1)) }
            "B" => { let mut i = pos.saturating_sub(1); while i > 0 && bytes[i].is_ascii_whitespace() { i -= 1; } while i > 0 && !bytes[i.saturating_sub(1)].is_ascii_whitespace() { i -= 1; } i }
            "E" => { let mut i = pos + 1; while i < len && bytes[i].is_ascii_whitespace() { i += 1; } while i + 1 < len && !bytes[i + 1].is_ascii_whitespace() { i += 1; } i.min(len.saturating_sub(1)) }
            "0" => text[..pos].rfind('\n').map_or(0, |i| i + 1),
            "^" => { let ls = text[..pos].rfind('\n').map_or(0, |i| i + 1); let r = &text[ls..]; ls + r.len() - r.trim_start().len() }
            "$" => text[pos..].find('\n').map_or(len.saturating_sub(1), |i| (pos + i).saturating_sub(1).max(pos)),
            "G" => text.rfind('\n').map_or(0, |i| i + 1),
            _ => pos,
        };
    }
    pos
}

// Operators
pub enum OperatorEffect { None, EnterInsert { offset: usize } }

#[allow(dead_code)]
fn apply_operator(buf: &mut VimBuffer, op: Operator, from: usize, to: usize, persistent: &mut PersistentState, linewise: bool) -> OperatorEffect {
    let (from, to) = (from.min(to), from.max(to).min(buf.text.len()));
    let mut content = buf.text[from..to].to_string();
    if linewise && !content.ends_with('\n') { content.push('\n'); }
    persistent.register = content; persistent.register_is_linewise = linewise;
    match op {
        Operator::Yank => { buf.cursor = from; OperatorEffect::None }
        Operator::Delete => { buf.text = format!("{}{}", &buf.text[..from], &buf.text[to..]); buf.cursor = from.min(buf.text.len().saturating_sub(1)); OperatorEffect::None }
        Operator::Change => { buf.text = format!("{}{}", &buf.text[..from], &buf.text[to..]); OperatorEffect::EnterInsert { offset: from } }
    }
}
