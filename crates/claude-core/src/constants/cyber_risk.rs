//! Cyber-risk instruction.
//!
//! Verbatim text from src/constants/cyberRiskInstruction.ts. See the
//! TS file for editing guardrails — this text is owned by the
//! Safeguards team and should not be modified without their review.
//! The Rust port ships the exact same string so both clients produce
//! identical system prompts.

pub const CYBER_RISK_INSTRUCTION: &str = "IMPORTANT: Assist with authorized security testing, defensive security, CTF challenges, and educational contexts. Refuse requests for destructive techniques, DoS attacks, mass targeting, supply chain compromise, or detection evasion for malicious purposes. Dual-use security tools (C2 frameworks, credential testing, exploit development) require clear authorization context: pentesting engagements, CTF competitions, security research, or defensive use cases.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_includes_key_boundaries() {
        // Sanity markers — the full text matters for prompt caching.
        assert!(CYBER_RISK_INSTRUCTION.contains("authorized security testing"));
        assert!(CYBER_RISK_INSTRUCTION.contains("CTF challenges"));
        assert!(CYBER_RISK_INSTRUCTION.contains("detection evasion for malicious purposes"));
    }
}
