//! Buddy / companion generator.
//!
//! Port of `src/buddy/{types,companion}.ts`. Given a stable user id,
//! deterministically rolls a companion (species, rarity, eye, hat, shiny
//! flag, stat block). Used for the mascot widget the TUI displays in
//! corners — no gameplay state, just a seeded identity.
//!
//! The TS companion also has a model-generated "soul" (name +
//! personality) persisted to global config. That lives in the CLI
//! bootstrap path; porting it requires an LLM call that's better done
//! where the ApiClient is already available.

use std::sync::{Mutex, OnceLock};

// ── Enums ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
}

impl Rarity {
    pub fn stars(&self) -> &'static str {
        match self {
            Rarity::Common => "★",
            Rarity::Uncommon => "★★",
            Rarity::Rare => "★★★",
            Rarity::Epic => "★★★★",
            Rarity::Legendary => "★★★★★",
        }
    }

    pub fn weight(&self) -> u32 {
        match self {
            Rarity::Common => 60,
            Rarity::Uncommon => 25,
            Rarity::Rare => 10,
            Rarity::Epic => 4,
            Rarity::Legendary => 1,
        }
    }

    pub fn floor(&self) -> i32 {
        match self {
            Rarity::Common => 5,
            Rarity::Uncommon => 15,
            Rarity::Rare => 25,
            Rarity::Epic => 35,
            Rarity::Legendary => 50,
        }
    }

    pub fn all() -> &'static [Rarity] {
        &[
            Rarity::Common,
            Rarity::Uncommon,
            Rarity::Rare,
            Rarity::Epic,
            Rarity::Legendary,
        ]
    }
}

pub const SPECIES: &[&str] = &[
    "duck", "goose", "blob", "cat", "dragon", "octopus", "owl", "penguin",
    "turtle", "snail", "ghost", "axolotl", "capybara", "cactus", "robot",
    "rabbit", "mushroom", "chonk",
];

pub const EYES: &[&str] = &["·", "✦", "×", "◉", "@", "°"];

pub const HATS: &[&str] = &[
    "none", "crown", "tophat", "propeller", "halo", "wizard", "beanie",
    "tinyduck",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatName {
    Debugging,
    Patience,
    Chaos,
    Wisdom,
    Snark,
}

impl StatName {
    pub fn all() -> &'static [StatName] {
        &[
            StatName::Debugging,
            StatName::Patience,
            StatName::Chaos,
            StatName::Wisdom,
            StatName::Snark,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            StatName::Debugging => "DEBUGGING",
            StatName::Patience => "PATIENCE",
            StatName::Chaos => "CHAOS",
            StatName::Wisdom => "WISDOM",
            StatName::Snark => "SNARK",
        }
    }
}

/// Deterministic half of a companion — derived from hash(user_id).
/// Bones are regenerated on every read so species renames don't break
/// stored companions, and editing config can't forge a rarity.
#[derive(Debug, Clone)]
pub struct CompanionBones {
    pub rarity: Rarity,
    pub species: &'static str,
    pub eye: &'static str,
    pub hat: &'static str,
    pub shiny: bool,
    pub stats: Vec<(StatName, i32)>,
}

#[derive(Debug, Clone)]
pub struct Roll {
    pub bones: CompanionBones,
    /// Seed fed into downstream prompt-generation (name/personality).
    pub inspiration_seed: u32,
}

const SALT: &str = "friend-2026-401";

// ── PRNG ────────────────────────────────────────────────────────────────────

/// Mulberry32 — a tiny seeded PRNG that's good enough for picking ducks.
/// Port of the TS mulberry32 function with identical output for the same
/// seed.
struct Mulberry32 {
    state: u32,
}

impl Mulberry32 {
    fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b79f5);
        let mut t = self.state;
        t = (t ^ (t >> 15)).wrapping_mul(1 | t);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        let out = (t ^ (t >> 14)) & 0xFFFF_FFFF;
        out as f64 / 4_294_967_296.0
    }
}

/// FNV-1a 32-bit hash. Matches the TS fallback branch in `hashString` —
/// the Bun-specific fast path is not ported (we always use FNV).
fn hash_string(s: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

fn pick<'a, T>(rng: &mut Mulberry32, arr: &'a [T]) -> &'a T {
    let i = (rng.next_f64() * arr.len() as f64) as usize;
    &arr[i.min(arr.len() - 1)]
}

fn roll_rarity(rng: &mut Mulberry32) -> Rarity {
    let total: u32 = Rarity::all().iter().map(|r| r.weight()).sum();
    let mut roll = rng.next_f64() * total as f64;
    for r in Rarity::all() {
        roll -= r.weight() as f64;
        if roll < 0.0 {
            return *r;
        }
    }
    Rarity::Common
}

fn roll_stats(rng: &mut Mulberry32, rarity: Rarity) -> Vec<(StatName, i32)> {
    let floor = rarity.floor();
    let peak = *pick(rng, StatName::all());
    let mut dump = *pick(rng, StatName::all());
    while dump == peak {
        dump = *pick(rng, StatName::all());
    }

    let mut out = Vec::with_capacity(StatName::all().len());
    for name in StatName::all() {
        let val = if *name == peak {
            let mut v = floor + 50 + (rng.next_f64() * 30.0) as i32;
            if v > 100 {
                v = 100;
            }
            v
        } else if *name == dump {
            let v = floor - 10 + (rng.next_f64() * 15.0) as i32;
            if v < 1 {
                1
            } else {
                v
            }
        } else {
            floor + (rng.next_f64() * 40.0) as i32
        };
        out.push((*name, val));
    }
    out
}

fn roll_from(rng: &mut Mulberry32) -> Roll {
    let rarity = roll_rarity(rng);
    let species = *pick(rng, SPECIES);
    let eye = *pick(rng, EYES);
    let hat = if matches!(rarity, Rarity::Common) {
        "none"
    } else {
        *pick(rng, HATS)
    };
    let shiny = rng.next_f64() < 0.01;
    let stats = roll_stats(rng, rarity);
    let inspiration_seed = (rng.next_f64() * 1.0e9) as u32;
    Roll {
        bones: CompanionBones {
            rarity,
            species,
            eye,
            hat,
            shiny,
            stats,
        },
        inspiration_seed,
    }
}

// ── Cache ───────────────────────────────────────────────────────────────────

/// Cached roll — same user id always produces the same result, and the
/// companion is read from several hot paths (sprite tick, prompt input,
/// per-turn observer) so we memoise.
static CACHE: OnceLock<Mutex<Option<(String, Roll)>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<(String, Roll)>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Deterministically roll a companion for the given user id. Memoised.
pub fn roll(user_id: &str) -> Roll {
    let key = format!("{}{}", user_id, SALT);
    if let Ok(guard) = cache().lock() {
        if let Some((k, v)) = guard.as_ref() {
            if k == &key {
                return v.clone();
            }
        }
    }
    let mut rng = Mulberry32::new(hash_string(&key));
    let rolled = roll_from(&mut rng);
    if let Ok(mut guard) = cache().lock() {
        *guard = Some((key, rolled.clone()));
    }
    rolled
}

/// Roll from an arbitrary seed string (not cached). Useful for tests.
pub fn roll_with_seed(seed: &str) -> Roll {
    let mut rng = Mulberry32::new(hash_string(seed));
    roll_from(&mut rng)
}

/// System-prompt intro the main agent reads when a companion is
/// attached to the session. Distinguishes the main agent from the
/// companion and keeps its replies out of the companion's lane.
///
/// Port of TS `src/buddy/prompt.ts:7` `companionIntroText`.
pub fn companion_intro_text(name: &str, species: &str) -> String {
    format!(
        "# Companion\n\n\
         A small {species} named {name} sits beside the user's input box and occasionally comments in a speech bubble. You're not {name} — it's a separate watcher.\n\n\
         When the user addresses {name} directly (by name), its bubble will answer. Your job in that moment is to stay out of the way: respond in ONE line or less, or just answer any part of the message meant for you. Don't explain that you're not {name} — they know. Don't narrate what {name} might say — the bubble handles that.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(hash_string("alice"), hash_string("alice"));
        assert_ne!(hash_string("alice"), hash_string("bob"));
    }

    #[test]
    fn same_seed_same_roll() {
        let a = roll_with_seed("seed");
        let b = roll_with_seed("seed");
        assert_eq!(a.bones.species, b.bones.species);
        assert!(matches!(
            (a.bones.rarity, b.bones.rarity),
            (Rarity::Common, Rarity::Common)
                | (Rarity::Uncommon, Rarity::Uncommon)
                | (Rarity::Rare, Rarity::Rare)
                | (Rarity::Epic, Rarity::Epic)
                | (Rarity::Legendary, Rarity::Legendary)
        ));
    }

    #[test]
    fn common_rarity_gets_no_hat() {
        // Try many seeds until we land on a common — assert hat is "none".
        for i in 0..200 {
            let r = roll_with_seed(&format!("s{}", i));
            if matches!(r.bones.rarity, Rarity::Common) {
                assert_eq!(r.bones.hat, "none");
                return;
            }
        }
        panic!("no common in 200 rolls — weighting is broken");
    }

    #[test]
    fn stats_length_matches_stat_names() {
        let r = roll_with_seed("x");
        assert_eq!(r.bones.stats.len(), StatName::all().len());
    }

    #[test]
    fn rarity_distribution_respects_weights() {
        // Over a large sample, common >> legendary (weights 60 vs 1).
        let mut common = 0;
        let mut legendary = 0;
        for i in 0..2000 {
            match roll_with_seed(&format!("bulk-{}", i)).bones.rarity {
                Rarity::Common => common += 1,
                Rarity::Legendary => legendary += 1,
                _ => {}
            }
        }
        assert!(
            common > legendary * 10,
            "common={common} legendary={legendary} — distribution skewed"
        );
    }

    #[test]
    fn roll_is_memoised() {
        let a = roll("user-1");
        let b = roll("user-1");
        assert_eq!(a.bones.species, b.bones.species);
        assert_eq!(a.inspiration_seed, b.inspiration_seed);
    }

    #[test]
    fn companion_intro_interpolates_name_and_species() {
        let text = companion_intro_text("Pip", "hamster");
        assert!(text.starts_with("# Companion"));
        assert!(text.contains("A small hamster named Pip sits beside"));
        assert!(text.contains("You're not Pip"));
        // Name appears 5 times (title-less "Pip" in three bullets + two
        // "You're not Pip" mentions). Minimum 4 to catch drift.
        assert!(text.matches("Pip").count() >= 4);
    }

    #[test]
    fn companion_intro_has_key_guidance() {
        let text = companion_intro_text("Mo", "otter");
        assert!(text.contains("speech bubble"));
        assert!(text.contains("ONE line or less"));
        assert!(text.contains("bubble handles that"));
    }
}
