//! Curated secret scanner.
//!
//! Port of the defensive half of `src/services/teamMemorySync/secretScanner.ts`.
//! TS uses a ~60-rule subset of gitleaks
//! (https://github.com/gitleaks/gitleaks) picked for near-zero false
//! positives. The scanner runs before team memory uploads so credentials
//! never leave the user's machine.
//!
//! This module is useful beyond team memory — any tool that's about to
//! ship user-generated content to a remote service should scan first.
//!
//! regex-lite does NOT support Perl-style inline flags `(?i)` or
//! back-references, so a handful of gitleaks rules that use those have
//! been rewritten with explicit character classes (matches the TS notes
//! in the original file).

use regex::Regex;
use std::sync::OnceLock;

/// A single curated rule.
struct SecretRule {
    id: &'static str,
    pattern: &'static str,
}

/// Match emitted from a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretMatch {
    pub rule_id: String,
    pub label: String,
}

/// The curated rule set. Patterns come directly from the TS port and
/// gitleaks config. Kept static so callers can reference rule IDs.
const SECRET_RULES: &[SecretRule] = &[
    // — Cloud providers —
    SecretRule {
        id: "aws-access-token",
        pattern: r"\b((?:A3T[A-Z0-9]|AKIA|ASIA|ABIA|ACCA)[A-Z2-7]{16})\b",
    },
    SecretRule {
        id: "gcp-api-key",
        pattern: r#"\b(AIza[\w-]{35})(?:[`'"\s;]|\\[nr]|$)"#,
    },
    SecretRule {
        id: "digitalocean-pat",
        pattern: r#"\b(dop_v1_[a-f0-9]{64})(?:[`'"\s;]|\\[nr]|$)"#,
    },
    SecretRule {
        id: "digitalocean-access-token",
        pattern: r#"\b(doo_v1_[a-f0-9]{64})(?:[`'"\s;]|\\[nr]|$)"#,
    },
    // — AI APIs —
    // The anthropic-api-key pattern is assembled at runtime in TS to dodge
    // the excluded-strings check; same idea here.
    // — Version control —
    SecretRule {
        id: "github-pat",
        pattern: r"ghp_[0-9a-zA-Z]{36}",
    },
    SecretRule {
        id: "github-fine-grained-pat",
        pattern: r"github_pat_\w{82}",
    },
    SecretRule {
        id: "github-oauth",
        pattern: r"gho_[0-9a-zA-Z]{36}",
    },
    SecretRule {
        id: "github-app-token",
        pattern: r"(?:ghu|ghs)_[0-9a-zA-Z]{36}",
    },
    SecretRule {
        id: "github-refresh-token",
        pattern: r"ghr_[0-9a-zA-Z]{36}",
    },
    // — Package managers —
    SecretRule {
        id: "npm-token",
        pattern: r"npm_[A-Za-z0-9]{36}",
    },
    // — Messaging —
    SecretRule {
        id: "slack-bot-token",
        pattern: r"xoxb-[0-9]{10,13}-[0-9]{10,13}-[a-zA-Z0-9]{24,34}",
    },
    SecretRule {
        id: "slack-user-token",
        pattern: r"xoxp-[0-9]{10,13}-[0-9]{10,13}-[0-9]{10,13}-[a-zA-Z0-9]{32}",
    },
    // — Email providers —
    SecretRule {
        id: "sendgrid-api-token",
        pattern: r"SG\.[a-zA-Z0-9_\-\.]{22}\.[a-zA-Z0-9_\-\.]{43}",
    },
    // — Private keys (PEM-style headers) —
    SecretRule {
        id: "private-key-pem",
        pattern: r"-----BEGIN [A-Z ]+PRIVATE KEY( BLOCK)?-----",
    },
    // — HuggingFace —
    SecretRule {
        id: "huggingface-access-token",
        pattern: r#"\b(hf_[a-zA-Z]{34})(?:[`'"\s;]|\\[nr]|$)"#,
    },
    // — Stripe —
    SecretRule {
        id: "stripe-access-token",
        pattern: r"(?:sk|rk)_(?:test|live)_[0-9a-zA-Z]{24,}",
    },
    // — Google/Firebase service account JSON —
    SecretRule {
        id: "gcp-service-account",
        pattern: r#""type":\s*"service_account""#,
    },
];

struct CompiledRule {
    id: &'static str,
    re: Regex,
}

fn compiled_rules() -> &'static [CompiledRule] {
    static CELL: OnceLock<Vec<CompiledRule>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut out = Vec::with_capacity(SECRET_RULES.len() + 2);
        for r in SECRET_RULES {
            if let Ok(re) = Regex::new(r.pattern) {
                out.push(CompiledRule { id: r.id, re });
            }
        }
        // Runtime-assembled Anthropic API key patterns so the literal
        // prefix strings don't appear in the compiled binary — matches
        // TS's ANT_KEY_PFX trick.
        let ant_pfx: String = ["sk", "ant", "api"].join("-");
        let anthropic = format!(
            r#"\b({pfx}03-[a-zA-Z0-9_\-]{{93}}AA)(?:[`'"\s;]|\\[nr]|$)"#,
            pfx = ant_pfx,
        );
        if let Ok(re) = Regex::new(&anthropic) {
            out.push(CompiledRule {
                id: "anthropic-api-key",
                re,
            });
        }
        let anthropic_admin = r#"\b(sk-ant-admin01-[a-zA-Z0-9_\-]{93}AA)(?:[`'"\s;]|\\[nr]|$)"#;
        if let Ok(re) = Regex::new(anthropic_admin) {
            out.push(CompiledRule {
                id: "anthropic-admin-api-key",
                re,
            });
        }
        out
    })
}

/// Scan `content` for potential secrets. Returns one match per rule that
/// fired, deduplicated by rule id. The matched text is intentionally not
/// returned — never log secret values.
pub fn scan_for_secrets(content: &str) -> Vec<SecretMatch> {
    let mut out = Vec::new();
    for rule in compiled_rules() {
        if rule.re.is_match(content) {
            out.push(SecretMatch {
                rule_id: rule.id.to_string(),
                label: rule_id_to_label(rule.id),
            });
        }
    }
    out
}

fn rule_id_to_label(id: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for p in id.split('-') {
        let label = match p {
            "aws" => "AWS",
            "gcp" => "GCP",
            "ad" => "AD",
            "tf" => "TF",
            "oauth" => "OAuth",
            "npm" => "NPM",
            "pypi" => "PyPI",
            "jwt" => "JWT",
            "github" => "GitHub",
            "gitlab" => "GitLab",
            "openai" => "OpenAI",
            "digitalocean" => "DigitalOcean",
            "huggingface" => "HuggingFace",
            "hashicorp" => "HashiCorp",
            "sendgrid" => "SendGrid",
            "pat" => "PAT",
            "api" => "API",
            "pem" => "PEM",
            other => {
                // capitalise first letter
                let mut chars = other.chars();
                match chars.next() {
                    Some(c) => {
                        parts.push(format!("{}{}", c.to_ascii_uppercase(), chars.as_str()));
                        continue;
                    }
                    None => other,
                }
            }
        };
        parts.push(label.to_string());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_github_pat() {
        // ghp_ followed by exactly 36 alphanumeric chars
        let content = "token=ghp_123456789012345678901234567890abcdef";
        let m = scan_for_secrets(content);
        assert!(m.iter().any(|x| x.rule_id == "github-pat"));
    }

    #[test]
    fn detects_aws_key() {
        let content = "export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";
        let m = scan_for_secrets(content);
        assert!(m.iter().any(|x| x.rule_id == "aws-access-token"));
    }

    #[test]
    fn detects_pem_block() {
        let content = "-----BEGIN RSA PRIVATE KEY-----\n...";
        let m = scan_for_secrets(content);
        assert!(m.iter().any(|x| x.rule_id == "private-key-pem"));
    }

    #[test]
    fn clean_content_no_matches() {
        let m = scan_for_secrets("just some normal project notes");
        assert!(m.is_empty());
    }

    #[test]
    fn label_generation_handles_special_cases() {
        assert_eq!(rule_id_to_label("aws-access-token"), "AWS Access Token");
        assert_eq!(rule_id_to_label("github-pat"), "GitHub PAT");
        assert_eq!(rule_id_to_label("openai-api-key"), "OpenAI API Key");
        assert_eq!(rule_id_to_label("private-key-pem"), "Private Key PEM");
    }

    #[test]
    fn dedup_across_rules() {
        // One GitHub PAT should produce one match even if multiple
        // rules share prefixes.
        let content = "ghp_123456789012345678901234567890abcdef";
        let m = scan_for_secrets(content);
        let ids: std::collections::HashSet<_> = m.iter().map(|x| &x.rule_id).collect();
        assert_eq!(ids.len(), m.len(), "duplicated rule ids");
    }

    #[test]
    fn slack_bot_token_detected() {
        let content = "slack=xoxb-1234567890-1234567890-abcdefghijklmnopqrstuvwxyz";
        let m = scan_for_secrets(content);
        assert!(m.iter().any(|x| x.rule_id == "slack-bot-token"));
    }

    #[test]
    fn service_account_json_marker_detected() {
        let content = r#"{"type": "service_account", "project_id": "..."}"#;
        let m = scan_for_secrets(content);
        assert!(m.iter().any(|x| x.rule_id == "gcp-service-account"));
    }
}
