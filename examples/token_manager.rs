//! Shows how to persist Questrade OAuth tokens across application runs.
//!
//! Questrade refresh tokens are **single-use** — each OAuth refresh rotates
//! the token, invalidating the old one. This example demonstrates:
//!
//! 1. Saving the rotated refresh token to a file (so the next run can auth)
//! 2. Caching the access token (so the next run skips the OAuth round-trip)
//! 3. Loading the cached token on startup
//!
//! Run:
//!
//!     cargo run -p questrade-client --example token_manager -- --refresh-token <TOKEN>
//!
//! Then run again *without* --refresh-token to see it load from the persisted files:
//!
//!     cargo run -p questrade-client --example token_manager

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use time::OffsetDateTime;

use questrade_client::{CachedToken, OnTokenRefresh, QuestradeClient, TokenManager};

/// Directory where we store token files for this example.
fn token_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("questrade-example");
    std::fs::create_dir_all(&dir).expect("create token dir");
    dir
}

/// Save the rotated refresh token to a file.
fn save_refresh_token(token: &str) {
    let path = token_dir().join("refresh_token");
    std::fs::write(&path, token).expect("write refresh token");
    eprintln!("Saved refresh token to {}", path.display());
}

/// Load a previously-saved refresh token.
fn load_refresh_token() -> Option<String> {
    let path = token_dir().join("refresh_token");
    std::fs::read_to_string(path).ok().filter(|s| !s.is_empty())
}

/// Save the access token + api_server so the next run can skip the OAuth refresh.
fn save_access_token(access_token: &str, api_server: &str, expires_at: OffsetDateTime) {
    let json = serde_json::json!({
        "access_token": access_token,
        "api_server": api_server,
        "expires_at": expires_at.format(&time::format_description::well_known::Rfc3339).unwrap(),
    });
    let path = token_dir().join("access_token.json");
    std::fs::write(&path, json.to_string()).expect("write access token cache");
    eprintln!("Cached access token to {}", path.display());
}

/// Load a previously-cached access token, if it exists and hasn't expired.
fn load_cached_token() -> Option<CachedToken> {
    let path = token_dir().join("access_token.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let expires_at = time::OffsetDateTime::parse(
        v["expires_at"].as_str()?,
        &time::format_description::well_known::Rfc3339,
    )
    .ok()?;

    Some(CachedToken {
        access_token: v["access_token"].as_str()?.to_string(),
        api_server: v["api_server"].as_str()?.to_string(),
        expires_at,
    })
}

#[derive(Parser)]
struct Args {
    /// Questrade OAuth refresh token (omit to load from persisted file).
    #[arg(long, env = "QUESTRADE_REFRESH_TOKEN")]
    refresh_token: Option<String>,

    /// Use practice (sandbox) account.
    #[arg(long, default_value_t = false)]
    practice: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("questrade_client=debug")
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    // Resolve the refresh token: CLI arg > persisted file.
    let refresh_token = args
        .refresh_token
        .or_else(load_refresh_token)
        .context("No refresh token. Pass --refresh-token or run once to persist one.")?;

    // On each token refresh, persist both the rotated refresh token and
    // the new access token so the next CLI invocation can reuse them.
    let on_refresh: OnTokenRefresh = Arc::new(|token| {
        save_refresh_token(&token.refresh_token);
        let expires_at =
            OffsetDateTime::now_utc() + time::Duration::seconds(token.expires_in as i64 - 30);
        save_access_token(&token.access_token, &token.api_server, expires_at);
    });

    // If we have a cached access token, pass it to skip the initial OAuth call.
    let cached = load_cached_token();
    if cached.is_some() {
        eprintln!("Found cached access token — will skip OAuth refresh if still valid.");
    }

    let tm = TokenManager::new(refresh_token, args.practice, Some(on_refresh), cached).await?;
    let client = QuestradeClient::new(tm)?;

    // Make a couple API calls to prove it works.
    let time = client.get_server_time().await?;
    println!("Server time: {time}");

    let accounts = client.get_accounts().await?;
    for acct in &accounts {
        println!(
            "Account: {} (type={}, status={})",
            acct.number, acct.account_type, acct.status
        );
    }

    Ok(())
}
