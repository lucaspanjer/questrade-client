//! Integration tests for `QuestradeClient` using a mock API server.
//!
//! Uses `wiremock` to serve canned JSON responses from the `fixtures/` directory,
//! exercising the full client→HTTP→deserialise path without touching the real
//! Questrade API.

use questrade_client::auth::CachedToken;
use questrade_client::{QuestradeClient, TokenManager};
use time::OffsetDateTime;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Load a fixture file relative to the `tests/fixtures/` directory.
fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
}

/// Create a `QuestradeClient` pointed at the given mock server with a valid
/// cached token so no OAuth refresh is needed.
async fn mock_client(server: &MockServer) -> QuestradeClient {
    let api_server = format!("{}/", server.uri());
    let cached = CachedToken {
        access_token: "test_token".to_string(),
        api_server,
        expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
    };
    let tm = TokenManager::new_with_login_url(
        "unused_refresh".to_string(),
        None,
        server.uri(),
        Some(cached),
    )
    .await
    .unwrap();
    QuestradeClient::new(tm).unwrap()
}

// ---------------------------------------------------------------------------
// Server time
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_server_time_returns_parsed_datetime() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/time"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("time.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let time = client.get_server_time().await.unwrap();
    assert_eq!(time.year(), 2026);
    assert_eq!(time.month(), time::Month::March);
    assert_eq!(time.day(), 3);
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_accounts_returns_all_accounts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("accounts.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let accounts = client.get_accounts().await.unwrap();
    assert_eq!(accounts.len(), 3);
    assert_eq!(accounts[0].account_type, "Margin");
    assert_eq!(accounts[0].number, "12345678");
    assert_eq!(accounts[1].account_type, "TFSA");
    assert!(accounts[1].is_primary);
    assert_eq!(accounts[2].account_type, "RRSP");
}

// ---------------------------------------------------------------------------
// Quotes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_raw_quote_returns_equity_quote() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/markets/quotes/8049"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("quotes.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let quote = client.get_raw_quote(8049).await.unwrap();
    assert_eq!(quote.symbol, "AAPL");
    assert_eq!(quote.symbol_id, 8049);
    assert_eq!(quote.bid_price, Some(182.30));
    assert_eq!(quote.ask_price, Some(182.45));
    assert_eq!(quote.last_trade_price, Some(182.40));
    assert_eq!(quote.volume, Some(52345678));
    assert_eq!(quote.open_price, Some(181.50));
    assert_eq!(quote.high_price, Some(183.10));
    assert_eq!(quote.low_price, Some(180.90));
}

// ---------------------------------------------------------------------------
// Positions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_positions_returns_equity_and_option_positions() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts/12345678/positions"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("positions.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let positions = client.get_positions("12345678").await.unwrap();
    assert_eq!(positions.len(), 2);

    let equity = &positions[0];
    assert_eq!(equity.symbol, "AAPL");
    assert_eq!(equity.open_quantity, 100.0);
    assert_eq!(equity.average_entry_price, 150.00);
    assert_eq!(equity.current_market_value, Some(18240.00));

    let option = &positions[1];
    assert_eq!(option.symbol, "AAPL 21MAR25 180 P");
    assert_eq!(option.open_quantity, -1.0);
    assert_eq!(option.open_pnl, Some(170.00));
}

// ---------------------------------------------------------------------------
// Balances
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_account_balances_returns_multi_currency() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts/12345678/balances"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("balances.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let balances = client.get_account_balances("12345678").await.unwrap();
    assert_eq!(balances.per_currency_balances.len(), 2);
    assert_eq!(balances.per_currency_balances[0].currency, "CAD");
    assert_eq!(balances.per_currency_balances[0].cash, 5000.00);
    assert_eq!(balances.per_currency_balances[1].currency, "USD");
    assert_eq!(balances.per_currency_balances[1].total_equity, 62000.00);
    assert!(balances.per_currency_balances[0].is_real_time);
    assert_eq!(balances.combined_balances.len(), 1);
    assert_eq!(balances.combined_balances[0].total_equity, 117000.00);
    assert_eq!(balances.sod_per_currency_balances.len(), 2);
    assert!(!balances.sod_per_currency_balances[0].is_real_time);
}

// ---------------------------------------------------------------------------
// Symbol search / resolve
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_symbol_finds_exact_match() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/symbols/search"))
        .and(query_param("prefix", "AAPL"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("symbol_search.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let id = client.resolve_symbol("AAPL").await.unwrap();
    assert_eq!(id, 8049);
}

#[tokio::test]
async fn resolve_symbol_case_insensitive() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/symbols/search"))
        .and(query_param("prefix", "AAPL"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("symbol_search.json")))
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let id = client.resolve_symbol("aapl").await.unwrap();
    assert_eq!(id, 8049);
}

// ---------------------------------------------------------------------------
// Symbol detail
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_symbol_returns_full_detail() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/symbols/8049"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("symbol_detail.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let detail = client.get_symbol(8049).await.unwrap();
    assert_eq!(detail.symbol, "AAPL");
    assert_eq!(detail.description, "Apple Inc.");
    assert_eq!(detail.currency, "USD");
    assert!(detail.has_options);
    assert_eq!(detail.eps, Some(6.14));
    assert_eq!(detail.pe, Some(29.74));
    assert_eq!(detail.industry_sector.as_deref(), Some("Technology"));
    assert!(detail.option_type.is_none());
}

// ---------------------------------------------------------------------------
// Option chain structure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_option_chain_structure_returns_expiries_and_strikes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/symbols/8049/options"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("option_chain.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let chain = client.get_option_chain_structure(8049).await.unwrap();
    assert_eq!(chain.option_chain.len(), 2);

    let mar = &chain.option_chain[0];
    assert!(mar.expiry_date.contains("2026-03-21"));
    assert_eq!(mar.option_exercise_type, "American");
    assert_eq!(mar.chain_per_root.len(), 1);
    assert_eq!(mar.chain_per_root[0].option_root, "AAPL");
    assert_eq!(mar.chain_per_root[0].multiplier, Some(100));
    assert_eq!(mar.chain_per_root[0].chain_per_strike_price.len(), 3);

    let strike_180 = &mar.chain_per_root[0].chain_per_strike_price[1];
    assert_eq!(strike_180.strike_price, 180.0);
    assert_eq!(strike_180.call_symbol_id, 90003);
    assert_eq!(strike_180.put_symbol_id, 90004);

    let apr = &chain.option_chain[1];
    assert!(apr.expiry_date.contains("2026-04-17"));
    assert_eq!(apr.chain_per_root[0].chain_per_strike_price.len(), 2);
}

// ---------------------------------------------------------------------------
// Option quotes (POST endpoint)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_option_quotes_by_ids_returns_bid_ask_map() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/markets/quotes/options"))
        .and(header("Authorization", "Bearer test_token"))
        .and(body_json(serde_json::json!({"optionIds": [90003, 90004]})))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("option_quotes.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let map = client
        .get_option_quotes_by_ids(&[90003, 90004])
        .await
        .unwrap();
    assert_eq!(map.len(), 2);

    let (bid, ask) = map[&90003];
    assert_eq!(bid, 5.20);
    assert_eq!(ask, 5.40);

    let (bid, ask) = map[&90004];
    assert_eq!(bid, 3.10);
    assert_eq!(ask, 3.30);
}

#[tokio::test]
async fn get_option_quotes_raw_returns_full_objects() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/markets/quotes/options"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("option_quotes.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let quotes = client.get_option_quotes_raw(&[90003, 90004]).await.unwrap();
    assert_eq!(quotes.len(), 2);

    let call = &quotes[0];
    assert_eq!(call.symbol, "AAPL 21MAR26 180 C");
    assert_eq!(call.symbol_id, 90003);
    assert_eq!(call.delta, Some(0.55));
    assert_eq!(call.theta, Some(-0.08));
    assert_eq!(call.volatility, Some(0.32));
    assert_eq!(call.open_interest, Some(5678));
    assert_eq!(call.option_type.as_deref(), Some("Call"));

    let put = &quotes[1];
    assert_eq!(put.symbol_id, 90004);
    assert_eq!(put.delta, Some(-0.45));
    assert_eq!(put.option_type.as_deref(), Some("Put"));
}

// ---------------------------------------------------------------------------
// Markets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_markets_returns_market_info() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("markets.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let markets = client.get_markets().await.unwrap();
    assert!(markets.len() >= 5);

    let nasdaq = markets.iter().find(|m| m.name == "NASDAQ").unwrap();
    assert!(nasdaq.start_time.is_some());
    assert!(nasdaq.end_time.is_some());

    let tsx = markets.iter().find(|m| m.name == "TSX").unwrap();
    assert!(tsx.start_time.as_ref().unwrap().contains("09:30"));
}

// ---------------------------------------------------------------------------
// Orders
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_orders_returns_order_items() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts/12345678/orders"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("orders.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let start = OffsetDateTime::now_utc() - time::Duration::days(7);
    let end = OffsetDateTime::now_utc();
    let orders = client
        .get_orders(
            "12345678",
            start,
            end,
            questrade_client::api_types::OrderStateFilter::All,
        )
        .await
        .unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].symbol, "AAPL");
    assert_eq!(orders[0].state, "Executed");
    assert_eq!(orders[0].filled_quantity, 100.0);
    assert_eq!(orders[0].avg_exec_price, Some(150.25));
}

// ---------------------------------------------------------------------------
// Executions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_executions_returns_fill_details() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts/12345678/executions"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("executions.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let start = OffsetDateTime::now_utc() - time::Duration::days(7);
    let end = OffsetDateTime::now_utc();
    let execs = client.get_executions("12345678", start, end).await.unwrap();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0].symbol, "AAPL");
    assert_eq!(execs[0].quantity, 100.0);
    assert_eq!(execs[0].price, 150.25);
    assert_eq!(execs[0].commission, 4.95);
    assert_eq!(execs[0].venue.as_deref(), Some("LAMP"));
}

// ---------------------------------------------------------------------------
// Activities
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_activities_returns_trade_activity() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts/12345678/activities"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("activities.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let start = OffsetDateTime::now_utc() - time::Duration::days(7);
    let end = OffsetDateTime::now_utc();
    let activities = client.get_activities("12345678", start, end).await.unwrap();
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].symbol, "AAPL");
    assert_eq!(activities[0].action, "Buy");
    assert_eq!(activities[0].quantity, 100.0);
    assert_eq!(activities[0].net_amount, -15029.95);
    assert_eq!(activities[0].activity_type, "Trades");
    assert_eq!(activities[0].currency.as_deref(), Some("USD"));
}

// ---------------------------------------------------------------------------
// Strategy quotes (POST endpoint)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_strategy_quotes_returns_combined_quote() {
    let server = MockServer::start().await;

    let expected_body = serde_json::json!({
        "variants": [{
            "variantId": 1,
            "strategy": "Custom",
            "legs": [
                { "symbolId": 27426, "action": "Buy", "ratio": 1000 },
                { "symbolId": 10550014, "action": "Sell", "ratio": 10 }
            ]
        }]
    });

    Mock::given(method("POST"))
        .and(path("/v1/markets/quotes/strategies"))
        .and(header("Authorization", "Bearer test_token"))
        .and(body_json(&expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("strategy_quotes.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;

    let variants = vec![questrade_client::api_types::StrategyVariantRequest {
        variant_id: 1,
        strategy: "Custom".to_string(),
        legs: vec![
            questrade_client::api_types::StrategyLeg {
                symbol_id: 27426,
                action: "Buy".to_string(),
                ratio: 1000,
            },
            questrade_client::api_types::StrategyLeg {
                symbol_id: 10550014,
                action: "Sell".to_string(),
                ratio: 10,
            },
        ],
    }];

    let quotes = client.get_strategy_quotes(&variants).await.unwrap();
    assert_eq!(quotes.len(), 1);

    let q = &quotes[0];
    assert_eq!(q.variant_id, 1);
    assert_eq!(q.bid_price, Some(27.2));
    assert_eq!(q.ask_price, Some(27.23));
    assert_eq!(q.underlying, "MSFT");
    assert_eq!(q.underlying_id, 9291);
    assert_eq!(q.open_price, Some(27.0));
    assert_eq!(q.volatility, Some(0.30));
    assert_eq!(q.delta, Some(1.0));
    assert_eq!(q.gamma, Some(0.0));
    assert_eq!(q.theta, Some(-0.05));
    assert_eq!(q.vega, Some(0.01));
    assert_eq!(q.rho, Some(0.002));
    assert!(q.is_real_time);
}

// ---------------------------------------------------------------------------
// Auth verification: requests include Bearer token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn requests_include_bearer_auth_header() {
    let server = MockServer::start().await;
    // Only match if auth header is exactly right
    Mock::given(method("GET"))
        .and(path("/v1/accounts"))
        .and(header("Authorization", "Bearer test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("accounts.json")))
        .expect(1)
        .named("auth header check")
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let result = client.get_accounts().await;
    assert!(result.is_ok());
    // wiremock's expect(1) will panic on drop if the matcher didn't fire exactly once
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_error_returns_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/accounts"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string(r#"{"code":1001,"message":"Internal Server Error"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let err = client.get_accounts().await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("500"),
        "error should contain status code: {msg}"
    );
    assert!(
        msg.contains("Internal Server Error"),
        "error should contain body: {msg}"
    );
}

// ---------------------------------------------------------------------------
// get_text returns raw response body
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_text_returns_raw_json_string() {
    let server = MockServer::start().await;
    let raw = fixture("time.json");
    Mock::given(method("GET"))
        .and(path("/v1/time"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&raw))
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let text = client.get_text("/time").await.unwrap();
    // Should contain the same JSON (whitespace may differ due to wiremock)
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["time"], "2026-03-03T16:48:34.140000-05:00");
}

// ---------------------------------------------------------------------------
// Proactive rate limiting via X-RateLimit-* headers
// ---------------------------------------------------------------------------

/// When the server returns X-RateLimit-Remaining: 0 and a future reset time,
/// the next request should be delayed until the reset window. We verify by
/// checking that only one request hits the server before the "exhausted"
/// response, and the second request arrives after the reset.
#[tokio::test]
async fn proactive_rate_limit_blocks_until_reset() {
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    let server = MockServer::start().await;

    // Reset 2 seconds from now.
    let reset_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 2;

    // First request: succeeds but signals remaining=0.
    Mock::given(method("GET"))
        .and(path("/v1/time"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(fixture("time.json"))
                .insert_header("X-RateLimit-Remaining", "0")
                .insert_header("X-RateLimit-Reset", &reset_epoch.to_string()),
        )
        .expect(1)
        .up_to_n_times(1)
        .named("exhausted response")
        .mount(&server)
        .await;

    // Second request: after the reset, remaining is replenished.
    Mock::given(method("GET"))
        .and(path("/v1/time"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(fixture("time.json"))
                .insert_header("X-RateLimit-Remaining", "30")
                .insert_header("X-RateLimit-Reset", &(reset_epoch + 3600).to_string()),
        )
        .expect(1)
        .named("replenished response")
        .mount(&server)
        .await;

    let client = mock_client(&server).await;

    // First call — gets the exhausted response, updates rate limiter state.
    let _time1 = client.get_server_time().await.unwrap();

    // Second call — should block until reset (~2 s).
    let start = Instant::now();
    let _time2 = client.get_server_time().await.unwrap();
    let elapsed = start.elapsed();

    // Should have waited at least 1.5 s (the 2 s reset minus timing slack).
    assert!(
        elapsed.as_millis() >= 1500,
        "expected ≥1.5 s proactive wait, got {:?}",
        elapsed,
    );
}

/// When the server returns 429 WITH rate-limit headers, the client should use
/// the header-based wait instead of the exponential backoff. The retry should
/// succeed without explicit backoff sleep.
#[tokio::test]
async fn rate_limit_429_with_headers_uses_header_based_wait() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let server = MockServer::start().await;

    // Reset 1 second from now — much shorter than the default 1 s backoff,
    // and the test verifies we actually retry and succeed.
    let reset_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 1;

    // First request: 429 with rate-limit headers.
    Mock::given(method("GET"))
        .and(path("/v1/time"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("X-RateLimit-Remaining", "0")
                .insert_header("X-RateLimit-Reset", &reset_epoch.to_string()),
        )
        .expect(1)
        .up_to_n_times(1)
        .named("429 with headers")
        .mount(&server)
        .await;

    // Second request: succeeds.
    Mock::given(method("GET"))
        .and(path("/v1/time"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(fixture("time.json"))
                .insert_header("X-RateLimit-Remaining", "29")
                .insert_header("X-RateLimit-Reset", &(reset_epoch + 3600).to_string()),
        )
        .expect(1)
        .named("success after 429")
        .mount(&server)
        .await;

    let client = mock_client(&server).await;
    let time = client.get_server_time().await.unwrap();
    assert_eq!(time.year(), 2026);
}

/// Rate limits for different categories are tracked independently.
/// Exhausting the account bucket should not block market data requests.
#[tokio::test]
async fn rate_limit_categories_are_independent() {
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    let server = MockServer::start().await;

    let reset_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60; // far future — account category blocked for a long time

    // Account endpoint: exhausts the account bucket.
    Mock::given(method("GET"))
        .and(path("/v1/time"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(fixture("time.json"))
                .insert_header("X-RateLimit-Remaining", "0")
                .insert_header("X-RateLimit-Reset", &reset_epoch.to_string()),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Market data endpoint: unaffected, remaining is healthy.
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(fixture("markets.json"))
                .insert_header("X-RateLimit-Remaining", "19")
                .insert_header("X-RateLimit-Reset", &reset_epoch.to_string()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = mock_client(&server).await;

    // Exhaust the account bucket.
    let _time = client.get_server_time().await.unwrap();

    // Market data should still be fast (no proactive wait).
    let start = Instant::now();
    let markets = client.get_markets().await.unwrap();
    let elapsed = start.elapsed();

    assert!(!markets.is_empty());
    assert!(
        elapsed.as_millis() < 500,
        "market data should not be blocked by account rate limit, took {:?}",
        elapsed,
    );
}
