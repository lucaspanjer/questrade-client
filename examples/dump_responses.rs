//! Dump raw Questrade API responses to stdout.
//!
//! Usage:
//!     cargo run -p questrade-client --example dump_responses -- --refresh-token <TOKEN> --endpoint markets
//!
//! Or with env var:
//!     QUESTRADE_REFRESH_TOKEN=xxx cargo run -p questrade-client --example dump_responses -- --endpoint markets
//!
//! Pipe to jq for pretty printing:
//!     cargo run -p questrade-client --example dump_responses -- --refresh-token <TOKEN> --endpoint markets | jq .

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;

use questrade_client::{OnTokenRefresh, QuestradeClient, TokenManager};

/// Dump raw Questrade API responses to stdout for debugging.
#[derive(Parser)]
#[command(name = "dump_responses")]
struct Args {
    /// Questrade OAuth refresh token. Falls back to QUESTRADE_REFRESH_TOKEN env var.
    #[arg(long, env = "QUESTRADE_REFRESH_TOKEN")]
    refresh_token: String,

    /// Use practice (sandbox) account instead of live.
    #[arg(long, default_value_t = false)]
    practice: bool,

    /// API endpoint to call.
    #[arg(long, value_parser = ["time", "markets", "accounts"])]
    endpoint: String,

    /// Account ID (required for account-specific endpoints like positions).
    #[arg(long)]
    account_id: Option<String>,

    /// Symbol ID (required for symbol-specific endpoints).
    #[arg(long)]
    symbol_id: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Minimal tracing so auth issues are visible on stderr.
    tracing_subscriber::fmt()
        .with_env_filter("questrade_client=debug")
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    let on_refresh: OnTokenRefresh = Arc::new(|token| {
        eprintln!("--- Token rotated. Run this to update: ---");
        eprintln!("export QUESTRADE_REFRESH_TOKEN={}", token.refresh_token);
    });

    let tm = TokenManager::new(args.refresh_token, args.practice, Some(on_refresh), None)
        .await
        .context("Failed to create token manager")?;

    let client = QuestradeClient::new(tm)?;

    let path = match args.endpoint.as_str() {
        "time" => "/time".to_string(),
        "markets" => "/markets".to_string(),
        "accounts" => "/accounts".to_string(),
        other => bail!("Unknown endpoint: {other}"),
    };

    let text = client.get_text(&path).await?;
    println!("{text}");

    Ok(())
}
