# Session Log: OAuth Login Implementation Alignment

**Date:** 2026-04-02  
**Goal:** Review the login process in `claude-code-leaked` (TypeScript) and ensure `claude-rs` (Rust) implements the exact same logic, assuming a fresh device with only the Rust version installed.

---

## 1. Analysis Phase

### TypeScript Login Flow (claude-code-leaked)
Reviewed the complete OAuth 2.0 + PKCE login flow across these key files:
- `src/cli/handlers/auth.ts` — main `authLogin()` / `installOAuthTokens()` entry points
- `src/services/oauth/index.ts` — `OAuthService` orchestrating the flow
- `src/services/oauth/client.ts` — token exchange, refresh, profile fetch, API key creation
- `src/services/oauth/auth-code-listener.ts` — localhost HTTP callback server
- `src/services/oauth/crypto.ts` — PKCE verifier/challenge/state generation
- `src/utils/auth.ts` — token storage, auth resolution, org validation
- `src/utils/secureStorage/macOsKeychainStorage.ts` — macOS Keychain via `security` CLI
- `src/utils/secureStorage/plainTextStorage.ts` — `~/.claude/.credentials.json` fallback
- `src/constants/oauth.ts` — client ID, URLs, scopes

### Rust Implementation (claude-rs)
Found the existing implementation was ~95% complete:
- `crates/claude-core/src/auth/login.rs` — OAuth flow, token exchange
- `crates/claude-core/src/auth/storage.rs` — Keychain + file storage
- `crates/claude-core/src/auth/pkce.rs` — PKCE crypto
- `crates/claude-core/src/auth/profile.rs` — profile fetching, subscription mapping
- `crates/claude-core/src/auth/resolve.rs` — auth method resolution
- `crates/claude-cli/src/main.rs` — CLI `login`/`logout` subcommands

---

## 2. Gaps Identified

| Gap | Severity | Description |
|-----|----------|-------------|
| Callback server fragility | Medium | Used `try_read`/`try_write` which can fail on `WouldBlock` |
| Missing `ANTHROPIC_AUTH_TOKEN` | Medium | TS checks this env var before `CLAUDE_CODE_OAUTH_TOKEN` |
| Token storage guard | Low | TS only saves Claude.ai tokens to secure storage, not Console |
| Missing `firstTokenDate` fetch | Low | TS fetches org's first token date during login |
| Manual code input for headless | High | No way to complete login on devices without a browser |
| Wrong `redirect_uri` on manual flow | Critical | Token exchange used localhost URI instead of manual redirect URL |

---

## 3. Fixes Applied

### Fix 1: Callback Server Robustness
**Commit:** `810e1e82`

- Replaced bare `try_read` with a loop handling `WouldBlock`
- Added `write_all_to_stream()` helper for reliable HTTP response writes
- Added explicit check for `n == 0` (empty request)

### Fix 2: `ANTHROPIC_AUTH_TOKEN` Environment Variable
**Commit:** `810e1e82`

Added to `resolve.rs` auth resolution chain, matching TS priority order:
1. `ANTHROPIC_API_KEY` -> ApiKey
2. `ANTHROPIC_AUTH_TOKEN` -> OAuthToken (new)
3. `CLAUDE_CODE_OAUTH_TOKEN` -> OAuthToken
4. Stored OAuth tokens from keychain/file

### Fix 3: Token Storage Guard
**Commit:** `810e1e82`

Added conditional in `install_oauth_tokens()` to skip saving tokens to secure storage for:
- Non-Claude.ai auth (Console users)
- Inference-only tokens (no refresh token or expiry)

Matches TS `saveOAuthTokensIfNeeded()`.

### Fix 4: `firstTokenDate` Fetch
**Commit:** `810e1e82`

Added `fetch_and_store_first_token_date()`:
- `GET /api/organization/claude_code_first_token_date`
- Early-returns if already cached in global config
- Validates date string before saving
- Stores result in `claudeCodeFirstTokenDate` field

### Fix 5: Manual Code Input for Headless Login
**Commit:** `509e90ac`

Added `await_authorization_code()` function that races:
- TCP listener (automatic browser callback)
- Stdin reader (manual `code#state` or full URL paste)

Added `parse_manual_callback_input()` supporting:
- Full callback URL: `https://...?code=XXX&state=YYY`
- Short format: `code#state`

### Fix 6: Correct `redirect_uri` for Manual Flow
**Commit:** `da6c5877`

When manual input is used (no TCP stream), the token exchange now sends `MANUAL_REDIRECT_URL` (`https://platform.claude.com/oauth/code/callback`) instead of `http://localhost:PORT/callback`. The `redirect_uri` must match what was in the authorize URL.

---

## 4. Testing on Jetson (aarch64 Linux)

### Setup
- Device: NVIDIA Jetson (`maor-desktop`) running Linux 4.9.253-tegra aarch64
- Connected via Tailscale VPN (`100.95.228.92`, DERP relay ~190ms)
- SSH access: `maor@100.95.228.92` with password auth
- Rust installed via rustup (stable-aarch64-unknown-linux-gnu, rustc 1.94.1)

### Build
- `cargo build --release` succeeded (~2m30s on Jetson)

### Auth Tests
- All 29 auth unit tests passed on Jetson:
  - 17 login tests (URL building, callback parsing, scope matching, constants)
  - 4 profile tests (deserialization, subscription mapping)
  - 5 resolve tests (token expiry with 5-minute buffer)
  - 3 storage tests (serialization, keychain naming, TS format compat)

### End-to-End Login
1. `claude-rs logout` — cleared credentials successfully
2. `claude-rs login` — generated OAuth URL with PKCE
3. Auth URL opened on Mac, authenticated in browser
4. Pasted `code#state` via stdin pipe to Jetson process
5. Token exchange succeeded, profile fetched, tokens stored
6. `Login successful!` printed

### Model Verification
| Model | Result |
|-------|--------|
| `haiku` (claude-haiku-4-5) | Working — returned response |
| `sonnet` (claude-sonnet-4-6) | Auth OK — hit 429 rate limit from session usage |
| `opus` (claude-opus-4-6) | Auth OK — hit 429 rate limit from session usage |

All models authenticated successfully (received 429 not 401/403), confirming the OAuth tokens are valid.

---

## 5. Architecture Summary

```
Fresh Device Login Flow (Rust):

  claude-rs login
       |
       v
  Check CLAUDE_CODE_OAUTH_REFRESH_TOKEN env var?
       |-- yes --> refresh_for_login() --> install_oauth_tokens()
       |-- no  --> Browser OAuth flow:
       |
       v
  Generate PKCE (verifier, challenge, state)
  Start TCP listener on random port
  Build auth URLs (automatic + manual)
  Open browser / print manual URL
       |
       v
  Race: TCP callback vs stdin manual input
       |
       v
  Exchange code for tokens (POST /v1/oauth/token)
  Fetch profile info (GET /api/oauth/profile)
  Redirect browser to success page (if automatic)
       |
       v
  install_oauth_tokens():
    1. Clear old auth state
    2. Store account info in ~/.claude.json
    3. Save OAuth tokens to Keychain + ~/.claude/.credentials.json
    4. Fetch and store user roles
    5. Fetch firstTokenDate (Claude.ai users)
    6. Create API key (Console users)
    7. Mark onboarding complete
       |
       v
  "Login successful!"
```

---

## 6. Files Modified

- `crates/claude-core/src/auth/login.rs` — callback server, manual input, firstTokenDate, redirect_uri fix
- `crates/claude-core/src/auth/resolve.rs` — ANTHROPIC_AUTH_TOKEN env var

## 7. Commits

| Hash | Description |
|------|-------------|
| `810e1e82` | fix: harden OAuth login to match TS implementation |
| `509e90ac` | feat: add manual code input for headless login + full working tree sync |
| `da6c5877` | fix: use MANUAL_REDIRECT_URL for token exchange on manual code input |
