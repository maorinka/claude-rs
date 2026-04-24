//! Display-only constants ported from TS src/constants/.
//!
//! Groups:
//! - `figures`             → unicode glyphs for status / markers / effort
//! - `spinner_verbs`       → loading-message verb list
//! - `turn_completion_verbs` → past-tense verbs for "Worked for 5s" messages
//! - `error_ids`           → obfuscated error-site IDs

// ── figures ───────────────────────────────────────────────────────────────

/// A solid circle that vertically aligns on macOS but not Windows/Linux.
/// Callers can branch on `cfg!(target_os = "macos")` if they care.
pub const BLACK_CIRCLE_MACOS: &str = "⏺";
pub const BLACK_CIRCLE: &str = "●";
pub const BULLET_OPERATOR: &str = "∙";
pub const TEARDROP_ASTERISK: &str = "✻";
pub const UP_ARROW: &str = "↑";
pub const DOWN_ARROW: &str = "↓";
pub const LIGHTNING_BOLT: &str = "↯";

// Effort levels
pub const EFFORT_LOW: &str = "○";
pub const EFFORT_MEDIUM: &str = "◐";
pub const EFFORT_HIGH: &str = "●";
pub const EFFORT_MAX: &str = "◉";

// Media / trigger
pub const PLAY_ICON: &str = "▶";
pub const PAUSE_ICON: &str = "⏸";

// MCP subscription / inbound indicators
pub const REFRESH_ARROW: &str = "↻";
pub const CHANNEL_ARROW: &str = "←";
pub const INJECTED_ARROW: &str = "→";
pub const FORK_GLYPH: &str = "⑂";

// Review status (ultrareview diamond states)
pub const DIAMOND_OPEN: &str = "◇";
pub const DIAMOND_FILLED: &str = "◆";
pub const REFERENCE_MARK: &str = "※";

// Banner glyphs
pub const FLAG_ICON: &str = "⚑";
pub const BLOCKQUOTE_BAR: &str = "▎";
pub const HEAVY_HORIZONTAL: &str = "━";

// Bridge status
pub const BRIDGE_SPINNER_FRAMES: &[&str] = &["·|·", "·/·", "·—·", "·\\·"];
pub const BRIDGE_READY_INDICATOR: &str = "·✔︎·";
pub const BRIDGE_FAILED_INDICATOR: &str = "×";

/// Platform-appropriate solid circle.
pub fn platform_black_circle() -> &'static str {
    if cfg!(target_os = "macos") {
        BLACK_CIRCLE_MACOS
    } else {
        BLACK_CIRCLE
    }
}

// ── turn_completion_verbs ─────────────────────────────────────────────────

/// Past-tense verbs for completed-turn display. Pair naturally with
/// "for [duration]" — e.g. "Worked for 5s".
pub const TURN_COMPLETION_VERBS: &[&str] = &[
    "Baked",
    "Brewed",
    "Churned",
    "Cogitated",
    "Cooked",
    "Crunched",
    "Sautéed",
    "Worked",
];

// ── error_ids ─────────────────────────────────────────────────────────────
//
// Obfuscated numeric IDs surfaced in telemetry to tell which logError()
// call produced an error. Kept in sync with TS; new IDs must be added at
// the next unused slot (TS docs say next ID is 346).

pub const E_TOOL_USE_SUMMARY_GENERATION_FAILED: u32 = 344;

// ── spinner_verbs ─────────────────────────────────────────────────────────

/// Loading-message verbs. Callers typically pick one at turn start and
/// animate it with trailing dots.
pub const SPINNER_VERBS: &[&str] = &[
    "Accomplishing",
    "Actioning",
    "Actualizing",
    "Architecting",
    "Baking",
    "Beaming",
    "Beboppin'",
    "Befuddling",
    "Billowing",
    "Blanching",
    "Bloviating",
    "Boogieing",
    "Boondoggling",
    "Booping",
    "Bootstrapping",
    "Brewing",
    "Bunning",
    "Burrowing",
    "Calculating",
    "Canoodling",
    "Caramelizing",
    "Cascading",
    "Catapulting",
    "Cerebrating",
    "Channeling",
    "Channelling",
    "Choreographing",
    "Churning",
    "Clauding",
    "Coalescing",
    "Cogitating",
    "Combobulating",
    "Composing",
    "Computing",
    "Concocting",
    "Considering",
    "Contemplating",
    "Cooking",
    "Crafting",
    "Creating",
    "Crunching",
    "Crystallizing",
    "Cultivating",
    "Deciphering",
    "Deliberating",
    "Determining",
    "Dilly-dallying",
    "Discombobulating",
    "Doing",
    "Doodling",
    "Drizzling",
    "Ebbing",
    "Effecting",
    "Elucidating",
    "Embellishing",
    "Enchanting",
    "Envisioning",
    "Evaporating",
    "Fermenting",
    "Fiddle-faddling",
    "Finagling",
    "Flambéing",
    "Flibbertigibbeting",
    "Flowing",
    "Flummoxing",
    "Fluttering",
    "Forging",
    "Forming",
    "Frolicking",
    "Frosting",
    "Gallivanting",
    "Galloping",
    "Garnishing",
    "Generating",
    "Gesticulating",
    "Germinating",
    "Gitifying",
    "Grooving",
    "Gusting",
    "Harmonizing",
    "Hashing",
    "Hatching",
    "Herding",
    "Honking",
    "Hullaballooing",
    "Hyperspacing",
    "Ideating",
    "Imagining",
    "Improvising",
    "Incubating",
    "Inferring",
    "Infusing",
    "Ionizing",
    "Jitterbugging",
    "Julienning",
    "Kneading",
    "Leavening",
    "Levitating",
    "Lollygagging",
    "Manifesting",
    "Marinating",
    "Meandering",
    "Metamorphosing",
    "Misting",
    "Moonwalking",
    "Moseying",
    "Mulling",
    "Mustering",
    "Musing",
    "Nebulizing",
    "Nesting",
    "Newspapering",
    "Noodling",
    "Nucleating",
    "Orbiting",
    "Orchestrating",
    "Osmosing",
    "Perambulating",
    "Percolating",
    "Perusing",
    "Philosophising",
    "Photosynthesizing",
    "Pollinating",
    "Pondering",
    "Pontificating",
    "Pouncing",
    "Precipitating",
    "Prestidigitating",
    "Processing",
    "Proofing",
    "Propagating",
    "Puttering",
    "Puzzling",
    "Quantumizing",
    "Razzle-dazzling",
    "Razzmatazzing",
    "Recombobulating",
    "Reticulating",
    "Roosting",
    "Ruminating",
    "Sautéing",
    "Scampering",
    "Schlepping",
    "Scurrying",
    "Seasoning",
    "Shenaniganing",
    "Shimmying",
    "Simmering",
    "Skedaddling",
    "Sketching",
    "Slithering",
    "Smooshing",
    "Sock-hopping",
    "Spelunking",
    "Spinning",
    "Sprouting",
    "Stewing",
    "Sublimating",
    "Swirling",
    "Swooping",
    "Symbioting",
    "Synthesizing",
    "Tempering",
    "Thinking",
    "Thundering",
    "Tinkering",
    "Tomfoolering",
    "Topsy-turvying",
    "Transfiguring",
    "Transmuting",
    "Twisting",
    "Undulating",
    "Unfurling",
    "Unravelling",
    "Vibing",
    "Waddling",
    "Wandering",
    "Warping",
    "Whatchamacalliting",
    "Whirlpooling",
    "Whirring",
    "Whisking",
    "Wibbling",
    "Working",
    "Wrangling",
    "Zesting",
    "Zigzagging",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_verbs_count_roughly_matches_ts() {
        // TS ships 189 at the time of port — allow drift but catch accidental wipeouts.
        assert!(SPINNER_VERBS.len() > 150);
    }

    #[test]
    fn turn_completion_verbs_are_past_tense() {
        assert!(TURN_COMPLETION_VERBS.contains(&"Worked"));
        assert!(TURN_COMPLETION_VERBS.contains(&"Cogitated"));
    }

    #[test]
    fn error_ids_stable() {
        assert_eq!(E_TOOL_USE_SUMMARY_GENERATION_FAILED, 344);
    }

    #[test]
    fn figures_have_expected_glyphs() {
        assert_eq!(UP_ARROW, "↑");
        assert_eq!(EFFORT_HIGH, "●");
        assert_eq!(BRIDGE_SPINNER_FRAMES.len(), 4);
    }
}
