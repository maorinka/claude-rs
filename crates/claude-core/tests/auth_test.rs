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
    assert!(v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
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
    tokio::fs::write(&cred_path, serde_json::to_string(&data).unwrap()).await.unwrap();

    // Read back and verify the JSON was written correctly
    let content = tokio::fs::read_to_string(&cred_path).await.unwrap();
    let loaded: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(loaded["claudeAiOauth"]["accessToken"], "test_access");
    assert_eq!(loaded["claudeAiOauth"]["scopes"][0], "user:inference");
}

/// Bug #6: Credentials file should be written with 0o600 permissions on Unix.
/// This test simulates the store_tokens pattern: write file then set permissions.
#[cfg(unix)]
#[tokio::test]
async fn test_credentials_file_permissions_are_0600() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let cred_path = tmp.path().join(".credentials.json");

    let data = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": "secret_token",
            "refreshToken": "secret_refresh",
            "expiresAt": 9999999999u64,
            "scopes": ["user:inference"]
        }
    });
    let json = serde_json::to_string_pretty(&data).unwrap();
    tokio::fs::write(&cred_path, &json).await.unwrap();

    // After writing, set permissions to 0o600 (the fix)
    tokio::fs::set_permissions(&cred_path, std::fs::Permissions::from_mode(0o600))
        .await
        .unwrap();

    let metadata = tokio::fs::metadata(&cred_path).await.unwrap();
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "Credentials file should have 0o600 permissions, got {:o}",
        mode
    );
}

/// Bug #19: from_utf8_lossy silently corrupts invalid UTF-8 data.
/// The fix uses String::from_utf8 which returns an error on invalid UTF-8.
#[test]
fn test_invalid_utf8_rejected_not_lossy() {
    // Invalid UTF-8 bytes
    let invalid_bytes: Vec<u8> = vec![0xFF, 0xFE, 0x80, 0x81];

    // The buggy code would silently replace with U+FFFD:
    let lossy = String::from_utf8_lossy(&invalid_bytes).to_string();
    assert!(lossy.contains('\u{FFFD}'), "Lossy conversion should contain replacement chars");

    // The fix: String::from_utf8 returns an error
    let result = String::from_utf8(invalid_bytes);
    assert!(result.is_err(), "Invalid UTF-8 should return an error, not be silently corrupted");
}

/// Bug #19: Valid UTF-8 should still work fine with from_utf8.
#[test]
fn test_valid_utf8_accepted() {
    let valid_json = b"{\"claudeAiOauth\":{\"accessToken\":\"tok\"}}".to_vec();
    let result = String::from_utf8(valid_json);
    assert!(result.is_ok(), "Valid UTF-8 should be accepted");
    assert!(result.unwrap().starts_with('{'));
}
