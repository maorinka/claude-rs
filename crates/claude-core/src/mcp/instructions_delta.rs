//! MCP instructions-delta helpers.
//!
//! Port of the portable half of `src/utils/mcpInstructionsDelta.ts`.
//! TS ships the full diff against a live conversation transcript
//! scanning `<attachment type="mcp_instructions_delta">` blocks; the
//! Rust message + attachment types aren't in the same shape yet, so
//! this module ports:
//!
//!   - the `McpInstructionsDelta` + `ClientSideInstruction` structs
//!   - the `is_mcp_instructions_delta_enabled` gate
//!   - a `compute_delta` function that takes a "previously announced"
//!     Set and the current connected-server list + client-side
//!     instructions, returning what to add / remove
//!
//! The scan-messages-for-attachments path is the caller's job — they
//! already own the transcript shape and know how to extract prior
//! announcements. Giving them `compute_delta` keeps the diff logic
//! centralised.

use std::collections::{BTreeSet, HashMap};

use crate::errors_util::{is_env_definitely_falsy, is_env_truthy};

/// A server-announced or client-side instruction block prepared for
/// the system prompt. Matches TS `McpInstructionsDelta`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpInstructionsDelta {
    /// Server names newly announced this pass.
    pub added_names: Vec<String>,
    /// Rendered `## {name}\n{instructions}` blocks aligned with added_names.
    pub added_blocks: Vec<String>,
    /// Names that were previously announced but no longer connected.
    pub removed_names: Vec<String>,
}

impl McpInstructionsDelta {
    pub fn is_empty(&self) -> bool {
        self.added_names.is_empty() && self.removed_names.is_empty()
    }
}

/// Client-authored instruction block to announce in addition to (or
/// instead of) a server's own `InitializeResult.instructions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSideInstruction {
    pub server_name: String,
    pub block: String,
}

/// A currently-connected MCP server's declared instructions. The
/// upstream MCP types pull in a lot of schema machinery; this is the
/// narrow shape needed for delta computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedServer {
    pub name: String,
    pub instructions: Option<String>,
}

/// Is the delta-based announcement path enabled? Env override wins
/// over any default. Matches TS `isMcpInstructionsDeltaEnabled`.
pub fn is_mcp_instructions_delta_enabled() -> bool {
    if is_env_truthy("CLAUDE_CODE_MCP_INSTR_DELTA") {
        return true;
    }
    if is_env_definitely_falsy("CLAUDE_CODE_MCP_INSTR_DELTA") {
        return false;
    }
    // Default branch: on for ant users, off otherwise. The TS version
    // ALSO consults a GrowthBook flag (tengu_basalt_3kr); Rust hasn't
    // ported GrowthBook so we conservatively stay off for externals.
    crate::user_type::is_ant()
}

/// Compute the delta between `previously_announced` server names
/// (extracted from the transcript by the caller) and the current
/// `connected` set + client-side additions. Returns `None` when
/// nothing changed.
///
/// Matches TS `getMcpInstructionsDelta` without the transcript-scan
/// step (caller provides the already-deduped announced set).
pub fn compute_delta(
    previously_announced: &BTreeSet<String>,
    connected: &[ConnectedServer],
    client_side_instructions: &[ClientSideInstruction],
) -> Option<McpInstructionsDelta> {
    // Build the rendered block for each server that has any
    // instructions to announce (either server-authored or client-side).
    let connected_names: BTreeSet<String> =
        connected.iter().map(|c| c.name.clone()).collect();

    let mut blocks: HashMap<String, String> = HashMap::new();
    for c in connected {
        if let Some(instructions) = &c.instructions {
            blocks.insert(c.name.clone(), format!("## {}\n{}", c.name, instructions));
        }
    }
    for ci in client_side_instructions {
        if !connected_names.contains(&ci.server_name) {
            continue;
        }
        let new_block = match blocks.get(&ci.server_name) {
            Some(existing) => format!("{}\n\n{}", existing, ci.block),
            None => format!("## {}\n{}", ci.server_name, ci.block),
        };
        blocks.insert(ci.server_name.clone(), new_block);
    }

    let mut added: Vec<(String, String)> = blocks
        .into_iter()
        .filter(|(name, _)| !previously_announced.contains(name))
        .collect();
    added.sort_by(|a, b| a.0.cmp(&b.0));

    // A previously-announced server that's no longer connected is
    // removed. Matches TS: no "announced but lost instructions while
    // still connected" case — InitializeResult is immutable.
    let mut removed: Vec<String> = previously_announced
        .iter()
        .filter(|n| !connected_names.contains(*n))
        .cloned()
        .collect();
    removed.sort();

    if added.is_empty() && removed.is_empty() {
        return None;
    }

    Some(McpInstructionsDelta {
        added_names: added.iter().map(|(n, _)| n.clone()).collect(),
        added_blocks: added.into_iter().map(|(_, b)| b).collect(),
        removed_names: removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn gate_env_truthy_overrides_default() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("USER_TYPE");
        std::env::set_var("CLAUDE_CODE_MCP_INSTR_DELTA", "1");
        assert!(is_mcp_instructions_delta_enabled());
        std::env::remove_var("CLAUDE_CODE_MCP_INSTR_DELTA");
    }

    #[test]
    fn gate_env_falsy_overrides_default() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var("CLAUDE_CODE_MCP_INSTR_DELTA", "0");
        assert!(!is_mcp_instructions_delta_enabled());
        std::env::remove_var("CLAUDE_CODE_MCP_INSTR_DELTA");
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn gate_defaults_on_for_ant() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CLAUDE_CODE_MCP_INSTR_DELTA");
        std::env::set_var("USER_TYPE", "ant");
        assert!(is_mcp_instructions_delta_enabled());
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn gate_defaults_off_for_externals() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CLAUDE_CODE_MCP_INSTR_DELTA");
        std::env::remove_var("USER_TYPE");
        assert!(!is_mcp_instructions_delta_enabled());
    }

    fn server(name: &str, instr: Option<&str>) -> ConnectedServer {
        ConnectedServer {
            name: name.into(),
            instructions: instr.map(|s| s.into()),
        }
    }

    #[test]
    fn adds_new_connected_server_with_instructions() {
        let announced = BTreeSet::new();
        let connected = vec![server("alpha", Some("use this for foo"))];
        let d = compute_delta(&announced, &connected, &[]).unwrap();
        assert_eq!(d.added_names, vec!["alpha".to_string()]);
        assert_eq!(
            d.added_blocks,
            vec!["## alpha\nuse this for foo".to_string()]
        );
        assert!(d.removed_names.is_empty());
    }

    #[test]
    fn skips_servers_without_instructions() {
        let announced = BTreeSet::new();
        let connected = vec![server("alpha", None)];
        assert!(compute_delta(&announced, &connected, &[]).is_none());
    }

    #[test]
    fn reports_disconnected_as_removed() {
        let mut announced = BTreeSet::new();
        announced.insert("alpha".into());
        let connected: Vec<ConnectedServer> = Vec::new();
        let d = compute_delta(&announced, &connected, &[]).unwrap();
        assert!(d.added_names.is_empty());
        assert_eq!(d.removed_names, vec!["alpha".to_string()]);
    }

    #[test]
    fn client_side_appends_to_existing_block() {
        let announced = BTreeSet::new();
        let connected = vec![server("alpha", Some("server side"))];
        let client = vec![ClientSideInstruction {
            server_name: "alpha".into(),
            block: "client side extras".into(),
        }];
        let d = compute_delta(&announced, &connected, &client).unwrap();
        assert_eq!(d.added_names, vec!["alpha".to_string()]);
        assert_eq!(
            d.added_blocks,
            vec!["## alpha\nserver side\n\nclient side extras".to_string()]
        );
    }

    #[test]
    fn client_side_without_server_instructions_still_counts() {
        let announced = BTreeSet::new();
        let connected = vec![server("alpha", None)];
        let client = vec![ClientSideInstruction {
            server_name: "alpha".into(),
            block: "client only".into(),
        }];
        let d = compute_delta(&announced, &connected, &client).unwrap();
        assert_eq!(d.added_names, vec!["alpha".to_string()]);
        assert_eq!(d.added_blocks, vec!["## alpha\nclient only".to_string()]);
    }

    #[test]
    fn client_side_for_unconnected_server_dropped() {
        let announced = BTreeSet::new();
        let connected: Vec<ConnectedServer> = Vec::new();
        let client = vec![ClientSideInstruction {
            server_name: "ghost".into(),
            block: "nope".into(),
        }];
        assert!(compute_delta(&announced, &connected, &client).is_none());
    }

    #[test]
    fn no_changes_returns_none() {
        let mut announced = BTreeSet::new();
        announced.insert("alpha".into());
        let connected = vec![server("alpha", Some("x"))];
        assert!(compute_delta(&announced, &connected, &[]).is_none());
    }

    #[test]
    fn added_names_sorted_alphabetically() {
        let announced = BTreeSet::new();
        let connected = vec![
            server("gamma", Some("g")),
            server("alpha", Some("a")),
            server("beta", Some("b")),
        ];
        let d = compute_delta(&announced, &connected, &[]).unwrap();
        assert_eq!(d.added_names, vec!["alpha", "beta", "gamma"]);
    }
}
