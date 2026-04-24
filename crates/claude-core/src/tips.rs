//! Tip scheduling + history.
//!
//! Port of the schedulable pieces of `src/services/tips/`. The TS side
//! ships 761 LOC total but 686 LOC of that is the `tipRegistry` — deeply
//! coupled to IDE detection, effort-level env overrides, concurrent-
//! session counting, official marketplace plugin state, GrowthBook gates,
//! and referral rewards. Those gates don't exist on the Rust side yet,
//! so a faithful tip-registry port is architectural work.
//!
//! What's here:
//!   - `Tip` struct with id / content / cooldown_sessions / weight
//!   - `TipHistory` — record/query seen tips keyed on session count
//!     (matches TS tipsHistory: Record<tipId, numStartups>)
//!   - `select_tip_with_longest_time_since_shown` — scheduler that picks
//!     the tip the user hasn't seen for the longest
//!   - A small default registry of environment-agnostic tips so callers
//!     have something real to render immediately
//!
//! Callers that want the full TS tip set can register additional tips
//! as they port the underlying gates (IDE detection, plugins, …).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A single tip.
#[derive(Debug, Clone)]
pub struct Tip {
    /// Stable identifier used in history tracking + telemetry.
    pub id: String,
    /// Body shown to the user. Plain text; callers format.
    pub content: String,
    /// Minimum number of sessions between repeat shows.
    pub cooldown_sessions: u32,
    /// Higher weight = more likely to be picked when multiple tips are
    /// eligible. Matches the TS `weight` field semantics.
    pub weight: u32,
}

impl Tip {
    pub fn new(id: &str, content: &str, cooldown_sessions: u32) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            cooldown_sessions,
            weight: 1,
        }
    }
}

// ── History ────────────────────────────────────────────────────────────────

/// Per-tip history: tipId → startup-count when last shown. Mirrors TS
/// `tipsHistory` stored in globalConfig.
#[derive(Debug, Default, Clone)]
pub struct TipHistory {
    shown_at: HashMap<String, u32>,
    num_startups: u32,
}

impl TipHistory {
    pub fn new(num_startups: u32) -> Self {
        Self {
            shown_at: HashMap::new(),
            num_startups,
        }
    }

    pub fn from_map(shown_at: HashMap<String, u32>, num_startups: u32) -> Self {
        Self {
            shown_at,
            num_startups,
        }
    }

    /// Record that `tip_id` was shown during the current startup.
    pub fn record_shown(&mut self, tip_id: &str) {
        self.shown_at.insert(tip_id.to_string(), self.num_startups);
    }

    /// How many sessions ago was this tip last shown? `u32::MAX` if never.
    /// Matches TS `Infinity` return for unseen tips.
    pub fn sessions_since_last_shown(&self, tip_id: &str) -> u32 {
        match self.shown_at.get(tip_id) {
            None => u32::MAX,
            Some(n) => self.num_startups.saturating_sub(*n),
        }
    }

    /// Mark a startup — bumps the counter. Call once per process launch.
    pub fn bump_startup(&mut self) {
        self.num_startups = self.num_startups.wrapping_add(1);
    }
}

// ── Scheduler ──────────────────────────────────────────────────────────────

/// Of the tips eligible this session (cooldown satisfied), pick the one
/// the user hasn't seen for the longest. Matches TS
/// `selectTipWithLongestTimeSinceShown`.
pub fn select_tip_with_longest_time_since_shown<'a>(
    tips: &'a [Tip],
    history: &TipHistory,
) -> Option<&'a Tip> {
    if tips.is_empty() {
        return None;
    }
    tips.iter()
        .max_by_key(|t| history.sessions_since_last_shown(&t.id))
}

/// Filter tips by cooldown: a tip is eligible only if at least
/// `cooldown_sessions` sessions have passed since it was last shown.
pub fn eligible_tips<'a>(tips: &'a [Tip], history: &TipHistory) -> Vec<&'a Tip> {
    tips.iter()
        .filter(|t| history.sessions_since_last_shown(&t.id) >= t.cooldown_sessions)
        .collect()
}

// ── Default registry ───────────────────────────────────────────────────────

/// A small set of environment-agnostic tips. Selected because they apply
/// to every user regardless of IDE / plugins / subscription tier.
pub fn default_tips() -> Vec<Tip> {
    vec![
        Tip::new(
            "slash_help",
            "Type `/help` to see every slash command this build supports.",
            3,
        ),
        Tip::new(
            "slash_doctor",
            "Running into weirdness? `/doctor` reports environment / config issues.",
            5,
        ),
        Tip::new(
            "memory_system",
            "Type `/memory` to browse what Claude has learned about this project.",
            4,
        ),
        Tip::new(
            "keyboard_interrupt",
            "Hit Ctrl+C once to cancel the current turn without exiting the session.",
            6,
        ),
        Tip::new(
            "brief_mode",
            "Ctrl+Shift+B toggles brief mode — Claude replies with just the diff / result.",
            6,
        ),
        Tip::new(
            "background_agent",
            "Ask Claude to launch a background research agent with the Agent tool (run_in_background=true).",
            4,
        ),
        Tip::new(
            "skills",
            "Create reusable instructions via `~/.claude/skills/*.md` — the /skills command lists installed ones.",
            5,
        ),
    ]
}

// ── Process-wide singleton ─────────────────────────────────────────────────

use std::sync::OnceLock;

static GLOBAL_HISTORY: OnceLock<Arc<RwLock<TipHistory>>> = OnceLock::new();

pub fn global_history() -> Arc<RwLock<TipHistory>> {
    GLOBAL_HISTORY
        .get_or_init(|| Arc::new(RwLock::new(TipHistory::new(0))))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_shown_returns_max() {
        let h = TipHistory::new(5);
        assert_eq!(h.sessions_since_last_shown("foo"), u32::MAX);
    }

    #[test]
    fn record_then_query() {
        let mut h = TipHistory::new(10);
        h.record_shown("foo");
        assert_eq!(h.sessions_since_last_shown("foo"), 0);
        h.bump_startup();
        h.bump_startup();
        assert_eq!(h.sessions_since_last_shown("foo"), 2);
    }

    #[test]
    fn scheduler_picks_oldest() {
        let mut h = TipHistory::new(100);
        h.record_shown("recent");
        let mut old = TipHistory::new(10);
        old.record_shown("ancient");
        old.num_startups = 100;

        // Merge into a single history for the test.
        let mut merged = HashMap::new();
        merged.insert("recent".to_string(), 100);
        merged.insert("ancient".to_string(), 10);
        let merged_h = TipHistory::from_map(merged, 100);

        let tips = vec![Tip::new("recent", "…", 0), Tip::new("ancient", "…", 0)];
        let pick = select_tip_with_longest_time_since_shown(&tips, &merged_h);
        assert_eq!(pick.unwrap().id, "ancient");
    }

    #[test]
    fn scheduler_prefers_never_shown() {
        let mut shown = HashMap::new();
        shown.insert("seen".to_string(), 50);
        let h = TipHistory::from_map(shown, 100);

        let tips = vec![Tip::new("seen", "…", 0), Tip::new("fresh", "…", 0)];
        let pick = select_tip_with_longest_time_since_shown(&tips, &h);
        assert_eq!(pick.unwrap().id, "fresh");
    }

    #[test]
    fn cooldown_filters_recent_tips() {
        let mut shown = HashMap::new();
        shown.insert("recent".to_string(), 98);
        let h = TipHistory::from_map(shown, 100);

        let tips = vec![
            Tip::new("recent", "…", 5), // only 2 sessions ago, cooldown 5 → filtered
            Tip::new("eligible", "…", 5),
        ];
        let eligible = eligible_tips(&tips, &h);
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].id, "eligible");
    }

    #[test]
    fn default_registry_non_empty() {
        assert!(!default_tips().is_empty());
        assert!(default_tips().iter().any(|t| t.id == "slash_help"));
    }

    #[test]
    fn empty_tip_list_returns_none() {
        let h = TipHistory::new(0);
        assert!(select_tip_with_longest_time_since_shown(&[], &h).is_none());
    }
}
