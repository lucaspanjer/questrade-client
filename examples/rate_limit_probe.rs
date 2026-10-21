//! Probe Questrade's live rate-limit behaviour and print what the headers say.
//!
//! Checks two claims from the README's "API variances" section:
//!
//! 1. `/symbols` calls count against the account bucket, not market data.
//! 2. `X-RateLimit-*` headers track only the hourly quota, not the per-second one.
//!
//! All calls are read-only GETs. A default run makes about 60 requests
//! (well under 1% of either hourly budget) and may deliberately trigger a few
//! 429s during the burst phase.
//!
//! Usage, with a still-valid access token (no refresh, nothing rotates):
//!     QUESTRADE_ACCESS_TOKEN=xxx QUESTRADE_API_SERVER=https://api01.iq.questrade.com/ \
//!         cargo run --example rate_limit_probe
//!
//! Or with a refresh token:
//!     QUESTRADE_REFRESH_TOKEN=xxx cargo run --example rate_limit_probe
//!
//! The refresh token is single-use: the rotated token is printed to stderr,
//! and must be saved before the next run.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use reqwest::header::HeaderMap;
use tokio::task::JoinSet;

use questrade_client::{OnTokenRefresh, TokenManager};

/// Probe Questrade rate-limit headers and per-second behaviour.
#[derive(Parser)]
#[command(name = "rate_limit_probe")]
struct Args {
    /// Questrade OAuth refresh token. Falls back to QUESTRADE_REFRESH_TOKEN env var.
    #[arg(
        long,
        env = "QUESTRADE_REFRESH_TOKEN",
        required_unless_present = "access_token"
    )]
    refresh_token: Option<String>,

    /// Existing access token; skips the OAuth refresh so no token is rotated.
    #[arg(long, env = "QUESTRADE_ACCESS_TOKEN", requires = "api_server")]
    access_token: Option<String>,

    /// API server returned alongside the access token.
    #[arg(long, env = "QUESTRADE_API_SERVER")]
    api_server: Option<String>,

    /// Use practice (sandbox) account instead of live.
    #[arg(long, default_value_t = false)]
    practice: bool,

    /// Ticker used for market-data and symbol calls.
    #[arg(long, default_value = "AAPL")]
    ticker: String,

    /// Number of concurrent requests fired in each burst.
    #[arg(long, default_value_t = 40)]
    burst: usize,

    /// Skip the burst phase (bucket mapping only).
    #[arg(long, default_value_t = false)]
    no_burst: bool,
}

/// One observed response.
#[derive(Debug, Clone)]
struct Sample {
    label: String,
    status: u16,
    remaining: Option<u64>,
    reset: Option<i64>,
    server_now: Option<i64>,
    retry_after: Option<String>,
    elapsed_ms: u128,
}

impl Sample {
    /// Seconds from the server's `Date` header until `X-RateLimit-Reset`.
    fn reset_in(&self) -> Option<i64> {
        Some(self.reset? - self.server_now?)
    }
}

fn header_u64(h: &HeaderMap, name: &str) -> Option<u64> {
    h.get(name)?.to_str().ok()?.trim().parse().ok()
}

fn server_epoch(h: &HeaderMap) -> Option<i64> {
    let s = h.get(reqwest::header::DATE)?.to_str().ok()?;
    let dt = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc2822).ok()?;
    Some(dt.unix_timestamp())
}

async fn call(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    label: &str,
    path: &str,
    start: Instant,
) -> Result<(Sample, HeaderMap, String)> {
    let url = format!("{base}v1{path}");
    let resp = http
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("GET {path}"))?;
    let h = resp.headers().clone();
    let sample = Sample {
        label: label.to_string(),
        status: resp.status().as_u16(),
        remaining: header_u64(&h, "X-RateLimit-Remaining"),
        reset: header_u64(&h, "X-RateLimit-Reset").map(|v| v as i64),
        server_now: server_epoch(&h),
        retry_after: h
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        elapsed_ms: start.elapsed().as_millis(),
    };
    let body = resp.text().await.unwrap_or_default();
    Ok((sample, h, body))
}

fn print_sample(s: &Sample) {
    println!(
        "{:>6}ms  {:<28} {:>3}  remaining={:<7} reset_in={:<6} retry_after={}",
        s.elapsed_ms,
        s.label,
        s.status,
        s.remaining.map_or("-".into(), |v| v.to_string()),
        s.reset_in().map_or("-".into(), |v| format!("{v}s")),
        s.retry_after.as_deref().unwrap_or("-"),
    );
}

/// Fire `n` concurrent GETs at `path` and return the samples in completion order.
async fn burst(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    label: &str,
    path: &str,
    n: usize,
) -> Vec<Sample> {
    let start = Instant::now();
    let mut set = JoinSet::new();
    for i in 0..n {
        let (http, base, token, path) = (
            http.clone(),
            base.to_string(),
            token.to_string(),
            path.to_string(),
        );
        let label = format!("{label}#{i}");
        set.spawn(async move { call(&http, &base, &token, &label, &path, start).await });
    }
    let mut out = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok((s, _, _))) => out.push(s),
            Ok(Err(e)) => eprintln!("request failed: {e:#}"),
            Err(e) => eprintln!("task failed: {e}"),
        }
    }
    out.sort_by_key(|s| s.elapsed_ms);
    out
}

fn summarize_burst(name: &str, samples: &[Sample], per_second_limit: u64) {
    let ok = samples.iter().filter(|s| s.status == 200).count();
    let limited: Vec<&Sample> = samples.iter().filter(|s| s.status == 429).collect();
    let window_ms = samples.last().map_or(0, |s| s.elapsed_ms);
    let max_reset_in = samples.iter().filter_map(Sample::reset_in).max();
    let min_remaining = samples.iter().filter_map(|s| s.remaining).min();
    println!(
        "\n{name} burst: {} sent in ~{window_ms} ms, {ok} x 200, {} x 429",
        samples.len(),
        limited.len()
    );
    println!(
        "  lowest remaining seen: {:?}, furthest reset: {:?}s",
        min_remaining, max_reset_in
    );
    if let Some(first) = limited.first() {
        println!(
            "  first 429 headers: remaining={:?} reset_in={:?}s retry_after={:?}",
            first.remaining,
            first.reset_in(),
            first.retry_after
        );
    }
    let looks_per_second = min_remaining.is_some_and(|r| r <= per_second_limit)
        && max_reset_in.is_some_and(|r| r <= 2);
    println!(
        "  verdict: headers look like {} counter",
        if looks_per_second {
            "a PER-SECOND"
        } else {
            "an HOURLY (or longer) window"
        }
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let (token, base) = match (args.access_token, args.api_server) {
        (Some(token), Some(mut base)) => {
            if !base.ends_with('/') {
                base.push('/');
            }
            (token, base)
        }
        _ => {
            let on_refresh: OnTokenRefresh = Arc::new(|token| {
                eprintln!("--- Token rotated. Save this before running again: ---");
                eprintln!("export QUESTRADE_REFRESH_TOKEN={}", token.refresh_token);
            });
            let refresh_token = args.refresh_token.context("no refresh token")?;
            let tm = TokenManager::new(refresh_token, args.practice, Some(on_refresh), None)
                .await
                .context("Failed to create token manager")?;
            tm.get_token().await?
        }
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let start = Instant::now();

    // Resolve a symbol id to use for the rest of the probe.
    let (s, headers, body) = call(
        &http,
        &base,
        &token,
        "symbols/search",
        &format!("/symbols/search?prefix={}", args.ticker),
        start,
    )
    .await?;
    let v: serde_json::Value = serde_json::from_str(&body).context("parse symbol search")?;
    let symbol_id = v["symbols"][0]["symbolId"]
        .as_u64()
        .context("no symbolId in search result")?;

    println!("== Response headers on first call (looking for any other limit headers) ==");
    for (k, val) in &headers {
        println!("  {k}: {}", val.to_str().unwrap_or("<binary>"));
    }

    // Phase 1: bucket mapping. Interleave calls so each one's effect on the
    // account and market counters is visible: a call belongs to the bucket
    // whose remaining value it continues.
    println!(
        "\n== Phase 1: bucket mapping (symbol {} = {symbol_id}) ==",
        args.ticker
    );
    print_sample(&s);
    let sequence: Vec<(&str, String)> = vec![
        ("time [account]", "/time".into()),
        ("markets", "/markets".into()),
        ("symbols/:id", format!("/symbols/{symbol_id}")),
        ("time [account]", "/time".into()),
        (
            "markets/quotes/:id [market]",
            format!("/markets/quotes/{symbol_id}"),
        ),
        ("symbols?ids=", format!("/symbols?ids={symbol_id}")),
        ("time [account]", "/time".into()),
        (
            "markets/quotes/:id [market]",
            format!("/markets/quotes/{symbol_id}"),
        ),
        (
            "symbols/:id/options",
            format!("/symbols/{symbol_id}/options"),
        ),
        ("time [account]", "/time".into()),
        (
            "markets/quotes/:id [market]",
            format!("/markets/quotes/{symbol_id}"),
        ),
        (
            "symbols/search",
            format!("/symbols/search?prefix={}", args.ticker),
        ),
        ("time [account]", "/time".into()),
        (
            "markets/quotes/:id [market]",
            format!("/markets/quotes/{symbol_id}"),
        ),
        ("accounts [account]", "/accounts".into()),
    ];
    let mut phase1 = vec![s];
    for (label, path) in &sequence {
        let (s, _, _) = call(&http, &base, &token, label, path, start).await?;
        print_sample(&s);
        phase1.push(s);
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    // Classify each non-reference call by which reference counter it sits next to.
    println!("\n== Phase 1 verdict ==");
    let mut last_account: Option<u64> = None;
    let mut last_market: Option<u64> = None;
    for s in &phase1 {
        let Some(r) = s.remaining else {
            println!("  {:<28} no X-RateLimit-Remaining header", s.label);
            continue;
        };
        if s.label.ends_with("[account]") {
            last_account = Some(r);
            continue;
        }
        if s.label.ends_with("[market]") {
            last_market = Some(r);
            continue;
        }
        let dist = |prev: Option<u64>| prev.map(|p| p.abs_diff(r));
        let bucket = match (dist(last_account), dist(last_market)) {
            (Some(a), Some(m)) if a < m => "ACCOUNT",
            (Some(_), Some(_)) => "MARKET DATA",
            (Some(_), None) => "ACCOUNT (no market reference yet)",
            (None, Some(_)) => "MARKET DATA (no account reference yet)",
            (None, None) => "unknown (no reference yet)",
        };
        println!(
            "  {:<28} remaining={r:<7} account_ref={:<7} market_ref={:<7} -> {bucket}",
            s.label,
            last_account.map_or("-".into(), |v| v.to_string()),
            last_market.map_or("-".into(), |v| v.to_string()),
        );
    }

    if args.no_burst {
        return Ok(());
    }

    // Phase 2: per-second behaviour. If headers tracked the per-second limit,
    // remaining would drop to <= 20/30 and reset would be ~1 s away.
    println!(
        "\n== Phase 2: burst of {} concurrent requests per bucket ==",
        args.burst
    );
    let market = burst(
        &http,
        &base,
        &token,
        "quotes",
        &format!("/markets/quotes/{symbol_id}"),
        args.burst,
    )
    .await;
    market.iter().for_each(print_sample);
    summarize_burst("Market data (/markets/quotes)", &market, 20);

    tokio::time::sleep(Duration::from_secs(3)).await;

    let account = burst(&http, &base, &token, "time", "/time", args.burst).await;
    account.iter().for_each(print_sample);
    summarize_burst("Account (/time)", &account, 30);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_date_header() {
        let mut h = HeaderMap::new();
        h.insert(
            reqwest::header::DATE,
            "Tue, 20 Oct 2026 21:00:00 GMT".parse().unwrap(),
        );
        assert_eq!(server_epoch(&h), Some(1_792_530_000));
    }
}
