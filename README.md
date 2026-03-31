# questrade-client

[![Crates.io](https://img.shields.io/crates/v/questrade-client.svg)](https://crates.io/crates/questrade-client)
[![Docs.rs](https://docs.rs/questrade-client/badge.svg)](https://docs.rs/questrade-client)
[![CI](https://github.com/lucaspanjer/questrade-client/actions/workflows/ci.yml/badge.svg)](https://github.com/lucaspanjer/questrade-client/actions/workflows/ci.yml)

Async Rust client for the [Questrade REST API](https://www.questrade.com/api/documentation/getting-started).

Handles OAuth token refresh, typed market-data access (quotes, option chains, candles), and account-data access (positions, balances, activities, orders, executions).

## Features

- **Automatic OAuth token management** with single-use refresh token rotation
- **Token caching** to skip OAuth round-trips on subsequent runs
- **Transparent 401 retry** — forces a token refresh and retries once on Unauthorized
- **Proactive rate limiting** — tracks `X-RateLimit-Remaining` / `X-RateLimit-Reset` headers and blocks requests until the window resets, with independent budgets for account and market data categories
- **Rate-limit retry** with exponential backoff and jitter on 429 responses (fallback when headers are absent)
- **Fully typed** request/response types with serde deserialization
- **`tracing` instrumented** for structured logging at debug/trace levels
- **Raw response logging** mode for API debugging

## Quick start

Add to your `Cargo.toml`:

```toml
[dependencies]
questrade-client = "0.2.1"
tokio = { version = "1", features = ["full"] }
anyhow = "1"
```

```rust
use questrade_client::{TokenManager, QuestradeClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let manager = TokenManager::new(
        "your_refresh_token".to_string(),
        false, // false = live account, true = practice account
        None,  // optional token-refresh callback
        None,  // optional cached token to skip initial refresh
    ).await?;

    let client = QuestradeClient::new(manager)?;
    let accounts = client.get_accounts().await?;
    println!("accounts: {:?}", accounts);
    Ok(())
}
```

## Auth flow

Questrade uses OAuth 2.0 with **single-use refresh tokens**. Every time you exchange a refresh token for an access token, the old refresh token is invalidated and a new one is returned. If you lose the rotated token, you must generate a new one from the [Questrade API Hub](https://www.questrade.com/api).

### Token persistence

Pass an `OnTokenRefresh` callback to `TokenManager::new` to persist the rotated token after every automatic refresh:

```rust
use std::sync::Arc;
use questrade_client::{OnTokenRefresh, TokenManager, CachedToken};

let on_refresh: OnTokenRefresh = Arc::new(|token| {
    // Save token.refresh_token to disk, database, etc.
    // Also save token.access_token + token.api_server for caching
    std::fs::write("/tmp/refresh_token", &token.refresh_token).ok();
});

let manager = TokenManager::new(
    refresh_token,
    false,
    Some(on_refresh),
    None, // or pass a CachedToken to skip the initial refresh
).await?;
```

To skip the OAuth round-trip on subsequent runs, pass a `CachedToken`:

```rust
let cached = CachedToken {
    access_token: "saved_access_token".to_string(),
    api_server: "https://api01.iq.questrade.com/".to_string(),
    expires_at: saved_expiry,
};

let manager = TokenManager::new(
    refresh_token,
    false,
    Some(on_refresh),
    Some(cached),
).await?;
```

See the [`token_manager` example](examples/token_manager.rs) for a complete working implementation.

## API coverage

| Category | Method | Questrade endpoint |
|---|---|---|
| **Auth** | `TokenManager::new` | `GET /oauth2/token` |
| **Server** | `get_server_time` | `GET /v1/time` |
| **Markets** | `get_markets` | `GET /v1/markets` |
| **Symbols** | `resolve_symbol` | `GET /v1/symbols/search` |
| **Symbols** | `get_symbol` | `GET /v1/symbols/:id` |
| **Quotes** | `get_raw_quote` | `GET /v1/markets/quotes/:id` |
| **Options** | `get_option_chain_structure` | `GET /v1/symbols/:id/options` |
| **Options** | `get_option_quotes_by_ids` | `POST /v1/markets/quotes/options` |
| **Options** | `get_option_quotes_raw` | `POST /v1/markets/quotes/options` |
| **Candles** | `get_candles` | `GET /v1/markets/candles/:id` |
| **Accounts** | `get_accounts` | `GET /v1/accounts` |
| **Positions** | `get_positions` | `GET /v1/accounts/:id/positions` |
| **Balances** | `get_account_balances` | `GET /v1/accounts/:id/balances` |
| **Activities** | `get_activities` | `GET /v1/accounts/:id/activities` |
| **Orders** | `get_orders` | `GET /v1/accounts/:id/orders` |
| **Executions** | `get_executions` | `GET /v1/accounts/:id/executions` |
| **Strategies** | `get_strategy_quotes` | `POST /v1/markets/quotes/strategies` |
| **Raw** | `get_text` | Any `GET /v1/*` endpoint |

### Not yet implemented

| Endpoint | Description | Notes |
|---|---|---|
| `POST /v1/accounts/:id/orders` | Place a new order | Requires `trade` scope (partner-only) |
| `POST /v1/accounts/:id/orders/:orderId` | Replace/modify an existing order | Requires `trade` scope (partner-only) |
| `DELETE /v1/accounts/:id/orders/:orderId` | Cancel an order | Requires `trade` scope (partner-only) |
| `POST /v1/accounts/:id/orders/:orderId/impact` | Preview order impact (buying power, commission) | Requires `trade` scope (partner-only) |

The order management endpoints require the `trade` OAuth scope, which Questrade restricts to approved partner developers.

### Partial coverage notes

- **`POST /v1/markets/quotes/options`** supports two modes: by explicit IDs (implemented) and by filters (`underlyingId` + `expiryDate` + optional strike range + option type). Only the ID-based mode is currently exposed.
- **`GET /v1/symbols/search`** is wrapped by `resolve_symbol()` which returns only the first exact-match symbol ID. The raw search response (multiple partial matches) is not directly exposed as a public method.

### Automatic windowing

The `get_activities` and `get_executions` methods automatically split date ranges longer than 30 days into compliant sub-windows (Questrade limits queries to 31-day windows). Results are combined and sorted chronologically.

## Examples

- **[`dump_responses`](examples/dump_responses.rs)** — dump raw API JSON to stdout for debugging
- **[`token_manager`](examples/token_manager.rs)** — persist OAuth tokens across application runs

## License

MIT License. See [LICENSE](LICENSE) for details.
