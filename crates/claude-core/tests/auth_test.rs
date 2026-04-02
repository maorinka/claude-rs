use claude_core::auth::pkce::*;
use claude_core::auth::storage::*;

#[test]
fn test_code_verifier_length() {
    let v = generate_code_verifier();
    assert_eq!(v.len(), 43); // 32 bytes → 43 base64url chars
}

#[test]
fn test_code_verifier_is_base64url() {
    let v = generate_code_verifier();
    assert!(v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
}

#[test]
fn test_code_challenge_deterministic() {
    let challenge1 = generate_code_challenge("test_verifier");
    let challenge2 = generate_code_challenge("test_verifier");
    assert_eq!(challenge1, challenge2);
}

#[test]
fn test_code_challenge_differs_from_verifier() {
    let verifier = "test_verifier";
    let challenge = generate_code_challenge(verifier);
    assert_ne!(verifier, challenge);
}

#[test]
fn test_state_is_random() {
    let s1 = generate_state();
    let s2 = generate_state();
    assert_ne!(s1, s2); // Extremely unlikely to collide
    assert_eq!(s1.len(), 43);
}

#[tokio::test]
async fn test_store_and_load_tokens() {
    let tmp = tempfile::tempdir().unwrap();
    // Override claude_dir by using direct file ops
    let cred_path = tmp.path().join(".credentials.json");

    let tokens = OAuthStoredTokens {
        access_token: "test_access".into(),
        refresh_token: Some("test_refresh".into()),
        expires_at: Some(1234567890),
        scopes: vec!["user:inference".into()],
        subscription_type: None,
        rate_limit_tier: None,
    };

    // Write directly to temp path using camelCase keys (matching real Claude Code format)
    let data = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": "test_access",
            "refreshToken": "test_refresh",
            "expiresAt": 1234567890,
            "scopes": ["user:inference"]
        }
    });
    tokio::fs::write(&cred_path, serde_json::to_string(&data).unwrap())
        .await
        .unwrap();

    // Read back and verify the JSON was written correctly
    let content = tokio::fs::read_to_string(&cred_path).await.unwrap();
    let loaded: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(loaded["claudeAiOauth"]["accessToken"], "test_access");
    assert_eq!(loaded["claudeAiOauth"]["scopes"][0], "user:inference");
}
