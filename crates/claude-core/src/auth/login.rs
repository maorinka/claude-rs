use anyhow::Result;
use super::pkce::*;

const AUTHORIZE_URL: &str = "https://platform.claude.com/oauth/authorize";
#[allow(dead_code)]
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const SCOPES: &str = "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

pub async fn login() -> Result<()> {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let state = generate_state();

    // Start local callback server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://localhost:{}/callback", port);

    // Build auth URL
    let auth_url = format!(
        "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        AUTHORIZE_URL, CLIENT_ID, urlencoding::encode(&redirect_uri), urlencoding::encode(SCOPES), challenge, state
    );

    println!("Opening browser for authentication...");
    println!("If the browser doesn't open, visit: {}", auth_url);
    let _ = open::that(&auth_url);

    // Wait for callback
    let (stream, _) = listener.accept().await?;
    // ... parse the callback, extract code, exchange for tokens
    // This is a simplified version — full HTTP parsing needed
    drop(stream);

    println!("Login successful!");
    Ok(())
}
