//! Memory type taxonomy. Port of `src/memdir/memoryTypes.ts`.
//!
//! Memories are constrained to four types capturing context NOT derivable
//! from the current project state. Code patterns, architecture, git history,
//! and file structure are derivable and should NOT be saved as memories.

/// The four canonical memory types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryType {
    /// User's role, goals, preferences, expertise.
    User,
    /// Guidance the user has given on how to approach work.
    Feedback,
    /// Information about ongoing work, deadlines, incidents, decisions.
    Project,
    /// Pointers to external systems (dashboards, Linear, Slack channels).
    Reference,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }
}

pub const MEMORY_TYPES: &[MemoryType] = &[
    MemoryType::User,
    MemoryType::Feedback,
    MemoryType::Project,
    MemoryType::Reference,
];

/// Parse a raw frontmatter value into a MemoryType. Invalid / missing values
/// return `None` — legacy files without a `type:` field keep working.
/// Mirrors TS `parseMemoryType`.
pub fn parse_memory_type(raw: Option<&str>) -> Option<MemoryType> {
    let s = raw?;
    MEMORY_TYPES.iter().copied().find(|t| t.as_str() == s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_types() {
        assert_eq!(parse_memory_type(Some("user")), Some(MemoryType::User));
        assert_eq!(
            parse_memory_type(Some("feedback")),
            Some(MemoryType::Feedback)
        );
        assert_eq!(
            parse_memory_type(Some("project")),
            Some(MemoryType::Project)
        );
        assert_eq!(
            parse_memory_type(Some("reference")),
            Some(MemoryType::Reference)
        );
    }

    #[test]
    fn rejects_unknown_and_missing() {
        assert_eq!(parse_memory_type(Some("code")), None);
        assert_eq!(parse_memory_type(Some("")), None);
        assert_eq!(parse_memory_type(None), None);
    }
}
