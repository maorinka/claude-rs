//! Constants identifying Anthropic's first-party plugin marketplace.
//!
//! Port of TS `utils/plugins/officialMarketplace.ts:1-25`.
//!
//! Used to auto-install the marketplace on startup and to identify
//! it in `known_marketplaces.json`.

/// Display / registry name for the official marketplace. Matches TS
/// `OFFICIAL_MARKETPLACE_NAME` verbatim.
pub const OFFICIAL_MARKETPLACE_NAME: &str = "claude-plugins-official";

/// GitHub coordinates of the first-party marketplace. TS ships this
/// as a `MarketplaceSource` struct
/// (`{ source: "github", repo: "anthropics/claude-plugins-official" }`).
/// The Rust port stores the two pieces as separate constants because
/// the `MarketplaceSource` type itself is not yet ported in this
/// tree — a future plugin-marketplace module should consume both.
pub const OFFICIAL_MARKETPLACE_SOURCE: &str = "github";
pub const OFFICIAL_MARKETPLACE_REPO: &str = "anthropics/claude-plugins-official";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_pin() {
        assert_eq!(OFFICIAL_MARKETPLACE_NAME, "claude-plugins-official");
    }

    #[test]
    fn source_repo_pins() {
        assert_eq!(OFFICIAL_MARKETPLACE_SOURCE, "github");
        assert_eq!(
            OFFICIAL_MARKETPLACE_REPO,
            "anthropics/claude-plugins-official"
        );
    }
}
