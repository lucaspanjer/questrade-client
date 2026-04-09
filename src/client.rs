//! [`QuestradeClient`] — async HTTP client for the Questrade REST API.

use std::collections::HashMap;
use std::time::Duration;

use rand::Rng;
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;
use tracing::{debug, trace, warn};

use crate::api_types::*;
use crate::auth::TokenManager;
use crate::error::{QuestradeError, Result};
use crate::rate_limit::RateLimiter;

/// Overall request timeout (connect + read combined).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// TCP connection establishment timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum number of retries on 429 rate-limit responses.
const MAX_RETRIES: u32 = 3;
/// Base delay in milliseconds for exponential backoff (doubles each attempt).
const RETRY_BASE_DELAY_MS: u64 = 1000;

/// Compute exponential backoff delay for a given attempt (0-indexed) with ±20% jitter.
///
/// - attempt 0 → base ~1 s
/// - attempt 1 → base ~2 s
/// - attempt 2 → base ~4 s
fn backoff_delay(attempt: u32) -> Duration {
    let base_ms = RETRY_BASE_DELAY_MS << attempt; // 1000, 2000, 4000 ms
    let jitter_factor = rand::thread_rng().gen_range(0.8f64..=1.2f64);
    let delay_ms = (base_ms as f64 * jitter_factor) as u64;
    Duration::from_millis(delay_ms)
}

/// Determine how long to wait before retrying a 429 response.
///
/// If the response contains a `Retry-After` header with a valid integer number
/// of seconds, that value is used (capped at 60 s to avoid indefinite waits).
/// Otherwise, falls back to [`backoff_delay`] for the given attempt number.
fn retry_after_or_backoff(response: &reqwest::Response, attempt: u32) -> Duration {
    if let Some(val) = response.headers().get(reqwest::header::RETRY_AFTER)
        && let Ok(s) = val.to_str()
        && let Ok(secs) = s.trim().parse::<u64>()
    {
        let capped = secs.min(60);
        return Duration::from_secs(capped);
    }
    backoff_delay(attempt)
}

/// Format datetimes for Questrade query parameters using second precision in UTC.
///
/// Some endpoints reject long fractional-second timestamps with:
/// `{"code":1003,"message":"Argument length exceeds imposed limit"}`.
fn format_query_datetime(dt: OffsetDateTime) -> Result<String> {
    let utc = dt.to_offset(time::UtcOffset::UTC);
    let fmt = time::format_description::parse("[year]-[month]-[day]T[hour]:[minute]:[second]Z")
        .map_err(|e| QuestradeError::DateTime {
            context: "Failed to build datetime format".to_string(),
            source: Box::new(e),
        })?;
    utc.format(&fmt).map_err(|e| QuestradeError::DateTime {
        context: "Failed to format datetime for query parameter".to_string(),
        source: Box::new(e),
    })
}

/// Async HTTP client for the Questrade REST API.
///
/// Wraps a [`TokenManager`] for transparent OAuth token refresh and provides
/// methods for market data (quotes, option chains, candles) and account data
/// (positions, balances, activities).
///
/// Construct via [`QuestradeClient::new`] for defaults, or use
/// [`QuestradeClientBuilder`] to supply a custom [`reqwest::Client`]
/// (e.g. for custom TLS roots or proxy configuration):
///
/// ```no_run
/// # use questrade_client::{QuestradeClientBuilder, TokenManager};
/// # async fn example(tm: TokenManager) -> Result<(), Box<dyn std::error::Error>> {
/// let custom_http = reqwest::Client::builder()
///     .danger_accept_invalid_certs(true)
///     .build()?;
///
/// let client = QuestradeClientBuilder::new()
///     .token_manager(tm)
///     .http_client(custom_http)
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct QuestradeClient {
    http: reqwest::Client,
    token_manager: TokenManager,
    log_raw_responses: bool,
    rate_limiter: RateLimiter,
}

/// Builder for [`QuestradeClient`] that allows injecting a custom
/// [`reqwest::Client`] for TLS, proxy, or timeout configuration.
///
/// # Required
///
/// - [`token_manager`](Self::token_manager) — must be set before calling [`build`](Self::build).
///
/// # Optional
///
/// - [`http_client`](Self::http_client) — if omitted, a default client with
///   30 s request timeout and 10 s connect timeout is created.
///
/// # Example
///
/// ```no_run
/// # use questrade_client::{QuestradeClientBuilder, TokenManager};
/// # async fn example(tm: TokenManager) -> Result<(), Box<dyn std::error::Error>> {
/// let client = QuestradeClientBuilder::new()
///     .token_manager(tm)
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct QuestradeClientBuilder {
    token_manager: Option<TokenManager>,
    http_client: Option<reqwest::Client>,
}

impl QuestradeClientBuilder {
    /// Create a new builder with all fields unset.
    pub fn new() -> Self {
        Self {
            token_manager: None,
            http_client: None,
        }
    }

    /// Set the [`TokenManager`] used for OAuth token management (required).
    pub fn token_manager(mut self, tm: TokenManager) -> Self {
        self.token_manager = Some(tm);
        self
    }

    /// Provide a pre-configured [`reqwest::Client`] for HTTP requests.
    ///
    /// Use this to customise TLS roots, proxy settings, timeouts, or any
    /// other [`reqwest::ClientBuilder`] option. When omitted, a default
    /// client is created with a 30 s overall timeout and a 10 s connect
    /// timeout.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    /// Consume the builder and create a [`QuestradeClient`].
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - [`token_manager`](Self::token_manager) was not set.
    /// - No custom HTTP client was provided and building the default client
    ///   fails (e.g. TLS initialisation error).
    pub fn build(self) -> Result<QuestradeClient> {
        let token_manager = self.token_manager.ok_or_else(|| {
            QuestradeError::EmptyResponse(
                "QuestradeClientBuilder: token_manager is required".to_string(),
            )
        })?;

        let http = match self.http_client {
            Some(client) => client,
            None => reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .connect_timeout(CONNECT_TIMEOUT)
                .build()?,
        };

        Ok(QuestradeClient {
            http,
            token_manager,
            log_raw_responses: false,
            rate_limiter: RateLimiter::new(),
        })
    }
}

impl Default for QuestradeClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl QuestradeClient {
    /// Create a new client backed by the given [`TokenManager`].
    ///
    /// This is a convenience shorthand equivalent to:
    ///
    /// ```no_run
    /// # use questrade_client::{QuestradeClientBuilder, TokenManager};
    /// # fn example(token_manager: TokenManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let client = QuestradeClientBuilder::new()
    ///     .token_manager(token_manager)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be built
    /// (e.g. TLS initialisation fails).
    pub fn new(token_manager: TokenManager) -> Result<Self> {
        QuestradeClientBuilder::new()
            .token_manager(token_manager)
            .build()
    }

    /// Enable or disable raw response body logging at `trace!` level.
    ///
    /// When enabled, `get()` and `post()` read the response body as text,
    /// log it at `trace!` level, then deserialize from the string. When
    /// disabled (the default), responses are deserialized directly from the
    /// stream for zero overhead.
    pub fn with_raw_logging(mut self, enabled: bool) -> Self {
        self.log_raw_responses = enabled;
        self
    }

    /// GET request with auth header.
    ///
    /// Before sending, checks the proactive rate limiter and waits if the
    /// category's budget is exhausted. After each response, updates the
    /// rate-limit state from `X-RateLimit-*` headers. Retries once on 401
    /// Unauthorized after forcing a token refresh. Retries up to `MAX_RETRIES`
    /// times on 429 responses — the proactive wait handles the delay when
    /// rate-limit headers are present, otherwise falls back to `Retry-After`
    /// or exponential backoff.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let category = RateLimiter::classify(path);
        let mut auth_retried = false;
        loop {
            let (token, api_server) = self.token_manager.get_token().await?;
            let url = format!("{}v1{}", api_server, path);
            debug!(method = "GET", endpoint = %url, "HTTP request");

            let resp = {
                let mut attempt = 0u32;
                loop {
                    if let Some(wait) = self.rate_limiter.wait_duration(category) {
                        debug!(category = %category, wait = ?wait, "sleeping until rate-limit window resets");
                        tokio::time::sleep(wait).await;
                    }

                    let resp = self.http.get(&url).bearer_auth(&token).send().await?;
                    self.rate_limiter
                        .update_from_headers(category, resp.headers());

                    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        if attempt < MAX_RETRIES {
                            // If the rate limiter learned a wait duration from headers,
                            // the pre-check at the top of this loop handles the delay.
                            // Otherwise, fall back to Retry-After / exponential backoff.
                            if self.rate_limiter.wait_duration(category).is_none() {
                                let delay = retry_after_or_backoff(&resp, attempt);
                                warn!(attempt = attempt + 1, delay = ?delay, reason = "429", "rate limited: 429 response, no rate-limit headers, backing off");
                                tokio::time::sleep(delay).await;
                            }
                            attempt += 1;
                            continue;
                        }
                        return Err(QuestradeError::RateLimited {
                            retries: MAX_RETRIES,
                        });
                    }

                    break resp;
                }
            };

            if resp.status() == reqwest::StatusCode::UNAUTHORIZED && !auth_retried {
                warn!("received 401 Unauthorized, forcing token refresh and retrying");
                self.token_manager.force_refresh().await?;
                auth_retried = true;
                continue;
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(QuestradeError::Api { status, body });
            }

            if self.log_raw_responses {
                let text = resp.text().await?;
                trace!(method = "GET", endpoint = %url, body = %text, "raw response");
                return Ok(serde_json::from_str(&text)?);
            } else {
                return Ok(resp.json().await?);
            }
        }
    }

    /// POST request with auth header and JSON body.
    ///
    /// Same rate-limit, auth-retry, and 429-retry behaviour as `get()`.
    async fn post<T: serde::de::DeserializeOwned, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let category = RateLimiter::classify(path);
        let mut auth_retried = false;
        loop {
            let (token, api_server) = self.token_manager.get_token().await?;
            let url = format!("{}v1{}", api_server, path);
            debug!(method = "POST", endpoint = %url, "HTTP request");

            let resp = {
                let mut attempt = 0u32;
                loop {
                    if let Some(wait) = self.rate_limiter.wait_duration(category) {
                        debug!(category = %category, wait = ?wait, "sleeping until rate-limit window resets");
                        tokio::time::sleep(wait).await;
                    }

                    let resp = self
                        .http
                        .post(&url)
                        .bearer_auth(&token)
                        .json(body)
                        .send()
                        .await?;
                    self.rate_limiter
                        .update_from_headers(category, resp.headers());

                    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        if attempt < MAX_RETRIES {
                            if self.rate_limiter.wait_duration(category).is_none() {
                                let delay = retry_after_or_backoff(&resp, attempt);
                                warn!(attempt = attempt + 1, delay = ?delay, reason = "429", "rate limited: 429 response (POST), no rate-limit headers, backing off");
                                tokio::time::sleep(delay).await;
                            }
                            attempt += 1;
                            continue;
                        }
                        return Err(QuestradeError::RateLimited {
                            retries: MAX_RETRIES,
                        });
                    }

                    break resp;
                }
            };

            if resp.status() == reqwest::StatusCode::UNAUTHORIZED && !auth_retried {
                warn!("received 401 Unauthorized, forcing token refresh and retrying");
                self.token_manager.force_refresh().await?;
                auth_retried = true;
                continue;
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                return Err(QuestradeError::Api {
                    status,
                    body: body_text,
                });
            }

            if self.log_raw_responses {
                let text = resp.text().await?;
                trace!(method = "POST", endpoint = %url, body = %text, "raw response");
                return Ok(serde_json::from_str(&text)?);
            } else {
                return Ok(resp.json().await?);
            }
        }
    }

    /// GET request that returns the raw response body as a string.
    ///
    /// Same rate-limit, auth-retry, and 429-retry behaviour as `get()`
    /// but returns the response body as-is without deserializing. Useful for
    /// inspecting raw API responses during development.
    pub async fn get_text(&self, path: &str) -> Result<String> {
        let category = RateLimiter::classify(path);
        let mut auth_retried = false;
        loop {
            let (token, api_server) = self.token_manager.get_token().await?;
            let url = format!("{}v1{}", api_server, path);
            debug!(method = "GET", endpoint = %url, "HTTP request (text)");

            let resp = {
                let mut attempt = 0u32;
                loop {
                    if let Some(wait) = self.rate_limiter.wait_duration(category) {
                        debug!(category = %category, wait = ?wait, "sleeping until rate-limit window resets");
                        tokio::time::sleep(wait).await;
                    }

                    let resp = self.http.get(&url).bearer_auth(&token).send().await?;
                    self.rate_limiter
                        .update_from_headers(category, resp.headers());

                    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        if attempt < MAX_RETRIES {
                            if self.rate_limiter.wait_duration(category).is_none() {
                                let delay = retry_after_or_backoff(&resp, attempt);
                                warn!(attempt = attempt + 1, delay = ?delay, reason = "429", "rate limited: 429 response, no rate-limit headers, backing off");
                                tokio::time::sleep(delay).await;
                            }
                            attempt += 1;
                            continue;
                        }
                        return Err(QuestradeError::RateLimited {
                            retries: MAX_RETRIES,
                        });
                    }

                    break resp;
                }
            };

            if resp.status() == reqwest::StatusCode::UNAUTHORIZED && !auth_retried {
                warn!("received 401 Unauthorized, forcing token refresh and retrying");
                self.token_manager.force_refresh().await?;
                auth_retried = true;
                continue;
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(QuestradeError::Api { status, body });
            }

            return Ok(resp.text().await?);
        }
    }

    /// Parse a Questrade datetime string to `OffsetDateTime`.
    ///
    /// Questrade returns datetimes like `"2014-10-24T20:06:40.131000-04:00"`.
    pub fn parse_datetime(s: &str) -> Result<OffsetDateTime> {
        OffsetDateTime::parse(s, &Iso8601::DEFAULT).map_err(|e| QuestradeError::DateTime {
            context: format!("Failed to parse datetime: {}", s),
            source: Box::new(e),
        })
    }

    /// Parse a Questrade datetime to just a `time::Date` (for option expiry).
    pub fn parse_date(s: &str) -> Result<time::Date> {
        let dt = Self::parse_datetime(s)?;
        Ok(dt.date())
    }

    /// Resolve a ticker string to a Questrade symbol ID.
    pub async fn resolve_symbol(&self, ticker: &str) -> Result<u64> {
        let key = ticker.to_uppercase();
        let resp: SymbolSearchResponse =
            self.get(&format!("/symbols/search?prefix={}", key)).await?;
        let symbol = resp
            .symbols
            .into_iter()
            .find(|s| s.symbol.to_uppercase() == key)
            .ok_or_else(|| QuestradeError::SymbolNotFound(ticker.to_string()))?;
        Ok(symbol.symbol_id)
    }

    /// Fetch a raw equity quote by symbol ID.
    pub async fn get_raw_quote(&self, symbol_id: u64) -> Result<Quote> {
        let resp: QuoteResponse = self.get(&format!("/markets/quotes/{}", symbol_id)).await?;
        resp.quotes
            .into_iter()
            .next()
            .ok_or_else(|| QuestradeError::EmptyResponse("No quote returned".to_string()))
    }

    /// Fetch raw equity quotes for multiple symbol IDs in a single API call.
    ///
    /// Uses `GET /v1/markets/quotes?ids=...` with comma-separated IDs.
    /// Returns quotes in arbitrary order; callers should match on `symbol_id`.
    pub async fn get_raw_quotes(&self, symbol_ids: &[u64]) -> Result<Vec<Quote>> {
        if symbol_ids.is_empty() {
            return Ok(vec![]);
        }
        let ids = symbol_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let resp: QuoteResponse = self.get(&format!("/markets/quotes?ids={}", ids)).await?;
        Ok(resp.quotes)
    }

    /// Fetch the option chain structure (expiries + strikes + symbol IDs) for a symbol.
    pub async fn get_option_chain_structure(&self, symbol_id: u64) -> Result<OptionChainResponse> {
        self.get(&format!("/symbols/{}/options", symbol_id)).await
    }

    /// Fetch current quotes for a set of option symbol IDs.
    /// Returns a map of symbol_id -> (bid, ask).
    pub async fn get_option_quotes_by_ids(
        &self,
        symbol_ids: &[u64],
    ) -> Result<HashMap<u64, (f64, f64)>> {
        let mut result = HashMap::new();
        for chunk in symbol_ids.chunks(100) {
            let req = OptionQuoteRequest {
                option_ids: chunk.to_vec(),
            };
            let resp: OptionQuoteResponse = self.post("/markets/quotes/options", &req).await?;
            for oq in resp.option_quotes {
                result.insert(
                    oq.symbol_id,
                    (oq.bid_price.unwrap_or(0.0), oq.ask_price.unwrap_or(0.0)),
                );
            }
        }
        Ok(result)
    }

    /// Fetch full option quote objects for a set of option symbol IDs (in batches).
    pub async fn get_option_quotes_raw(&self, ids: &[u64]) -> Result<Vec<OptionQuote>> {
        let mut result = Vec::new();
        for chunk in ids.chunks(100) {
            let req = OptionQuoteRequest {
                option_ids: chunk.to_vec(),
            };
            let resp: OptionQuoteResponse = self.post("/markets/quotes/options", &req).await?;
            result.extend(resp.option_quotes);
        }
        Ok(result)
    }

    /// Fetch combined quotes for multi-leg option strategy variants.
    ///
    /// Posts the given variants to `POST /v1/markets/quotes/strategies` and
    /// returns the strategy quotes. Each variant's `variant_id` is echoed in
    /// the response for caller-side matching.
    pub async fn get_strategy_quotes(
        &self,
        variants: &[StrategyVariantRequest],
    ) -> Result<Vec<StrategyQuote>> {
        let req = StrategyQuoteRequest {
            variants: variants.to_vec(),
        };
        let resp: StrategyQuotesResponse = self.post("/markets/quotes/strategies", &req).await?;
        Ok(resp.strategy_quotes)
    }

    /// Fetch historical candles for a symbol.
    pub async fn get_candles(
        &self,
        symbol_id: u64,
        start: OffsetDateTime,
        end: OffsetDateTime,
        interval: &str,
    ) -> Result<Vec<Candle>> {
        let start_str = format_query_datetime(start)?;
        let end_str = format_query_datetime(end)?;
        let resp: CandleResponse = self
            .get(&format!(
                "/markets/candles/{}?startTime={}&endTime={}&interval={}",
                symbol_id, start_str, end_str, interval
            ))
            .await?;
        Ok(resp.candles)
    }

    /// Fetch the current server time from Questrade.
    ///
    /// Uses `GET /v1/time`. Not cached — real-time by definition.
    pub async fn get_server_time(&self) -> Result<OffsetDateTime> {
        let resp: ServerTimeResponse = self.get("/time").await?;
        Self::parse_datetime(&resp.time)
    }

    /// Fetch all accounts for the authenticated user.
    pub async fn get_accounts(&self) -> Result<Vec<Account>> {
        let resp: AccountsResponse = self.get("/accounts").await?;
        Ok(resp.accounts)
    }

    /// Fetch positions for a specific account.
    pub async fn get_positions(&self, account_id: &str) -> Result<Vec<PositionItem>> {
        let resp: PositionsResponse = self
            .get(&format!("/accounts/{}/positions", account_id))
            .await?;
        Ok(resp.positions)
    }

    /// Fetch current and start-of-day balances for a specific account.
    pub async fn get_account_balances(&self, account_id: &str) -> Result<AccountBalances> {
        self.get(&format!("/accounts/{}/balances", account_id))
            .await
    }

    /// Fetch metadata for all markets (trading hours, open/closed status).
    pub async fn get_markets(&self) -> Result<Vec<crate::api_types::MarketInfo>> {
        let resp: crate::api_types::MarketsResponse = self.get("/markets").await?;
        Ok(resp.markets)
    }

    /// Fetch full symbol details by numeric ID via `GET /v1/symbols/:id`.
    pub async fn get_symbol(&self, symbol_id: u64) -> Result<SymbolDetail> {
        let resp: SymbolDetailResponse = self.get(&format!("/symbols/{}", symbol_id)).await?;
        resp.symbols.into_iter().next().ok_or_else(|| {
            QuestradeError::EmptyResponse(format!("No symbol returned for id {}", symbol_id))
        })
    }

    /// Fetch full symbol details for multiple IDs in a single API call.
    ///
    /// Uses `GET /v1/symbols?ids=...` with comma-separated IDs.
    /// Returns details in arbitrary order; callers should match on `symbol_id`.
    pub async fn get_symbols(&self, symbol_ids: &[u64]) -> Result<Vec<SymbolDetail>> {
        if symbol_ids.is_empty() {
            return Ok(vec![]);
        }
        let ids = symbol_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let resp: SymbolDetailResponse = self.get(&format!("/symbols?ids={}", ids)).await?;
        Ok(resp.symbols)
    }

    /// Fetch account activities (executions, dividends, etc.) for a date range.
    ///
    /// Questrade limits queries to 31-day windows per request; we use 30-day
    /// windows to stay safely within the boundary. This method transparently
    /// splits any range longer than 30 days into compliant sub-windows and
    /// combines the results, sorted by `trade_date` ascending.
    pub async fn get_activities(
        &self,
        account_id: &str,
        start: OffsetDateTime,
        end: OffsetDateTime,
    ) -> Result<Vec<ActivityItem>> {
        let windows = activity_windows(start, end);
        let mut all = Vec::new();
        for (w_start, w_end) in windows {
            let start_str = format_query_datetime(w_start)?;
            let end_str = format_query_datetime(w_end)?;
            let resp: ActivitiesResponse = self
                .get(&format!(
                    "/accounts/{}/activities?startTime={}&endTime={}",
                    account_id, start_str, end_str,
                ))
                .await?;
            all.extend(resp.activities);
        }
        all.sort_by(|a, b| a.trade_date.cmp(&b.trade_date));
        Ok(all)
    }

    /// Fetch orders for a specific account within a date range.
    ///
    /// Use `state_filter` to limit results to open, closed, or all orders.
    /// Unlike activities, there is no documented date-range window limit for
    /// this endpoint.
    pub async fn get_orders(
        &self,
        account_id: &str,
        start: OffsetDateTime,
        end: OffsetDateTime,
        state_filter: OrderStateFilter,
    ) -> Result<Vec<OrderItem>> {
        let start_str = format_query_datetime(start)?;
        let end_str = format_query_datetime(end)?;
        let resp: OrdersResponse = self
            .get(&format!(
                "/accounts/{}/orders?startTime={}&endTime={}&stateFilter={}",
                account_id, start_str, end_str, state_filter,
            ))
            .await?;
        Ok(resp.orders)
    }

    /// Fetch trade executions (fill-level detail) for a date range.
    ///
    /// Uses 30-day windowing, same as [`get_activities`](Self::get_activities).
    /// Results are sorted by `timestamp` ascending.
    pub async fn get_executions(
        &self,
        account_id: &str,
        start: OffsetDateTime,
        end: OffsetDateTime,
    ) -> Result<Vec<Execution>> {
        let windows = activity_windows(start, end);
        let mut all = Vec::new();
        for (w_start, w_end) in windows {
            let start_str = format_query_datetime(w_start)?;
            let end_str = format_query_datetime(w_end)?;
            let resp: ExecutionsResponse = self
                .get(&format!(
                    "/accounts/{}/executions?startTime={}&endTime={}",
                    account_id, start_str, end_str,
                ))
                .await?;
            all.extend(resp.executions);
        }
        all.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(all)
    }
}

/// Split a date range into ≤30-day windows for Questrade's activities endpoint.
///
/// Questrade documents a "maximum 31 days" range, but live testing (Feb 2026)
/// shows the actual limit is **31 calendar days in Eastern Time**, measured from
/// midnight ET. For example, at 16:52 ET the API rejects a start time only
/// 30 d 17 h earlier (past midnight ET 31 days ago) while accepting 30 d 16 h 30 m.
///
/// Using 30-day windows keeps us a full calendar day inside the limit regardless
/// of the caller's timezone or time of day, with no observable cost (one extra
/// API call per year of history).
///
/// Returns windows as `(start, end)` pairs in chronological order.
/// Returns an empty `Vec` if `start >= end`.
fn activity_windows(
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> Vec<(OffsetDateTime, OffsetDateTime)> {
    const MAX_WINDOW: time::Duration = time::Duration::days(30);
    let mut windows = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let window_end = (cursor + MAX_WINDOW).min(end);
        windows.push((cursor, window_end));
        cursor = window_end;
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{CachedToken, TokenManager};
    use time::OffsetDateTime;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn server_time_response_deserializes() {
        let json = r#"{"time":"2026-02-21T14:32:00.000000-05:00"}"#;
        let resp: ServerTimeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.time, "2026-02-21T14:32:00.000000-05:00");
    }

    #[test]
    fn parse_server_time_returns_correct_fields() {
        let json = r#"{"time":"2026-02-21T14:32:00.000000-05:00"}"#;
        let resp: ServerTimeResponse = serde_json::from_str(json).unwrap();
        let dt = QuestradeClient::parse_datetime(&resp.time).unwrap();
        assert_eq!(dt.year(), 2026);
        assert_eq!(dt.month(), time::Month::February);
        assert_eq!(dt.day(), 21);
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 32);
        assert_eq!(dt.second(), 0);
        assert_eq!(dt.offset().whole_hours(), -5);
    }

    #[test]
    fn format_query_datetime_uses_utc_second_precision() {
        let dt = OffsetDateTime::parse("2026-02-24T03:58:12.123456789-05:00", &Iso8601::DEFAULT)
            .unwrap();
        let s = format_query_datetime(dt).unwrap();
        assert_eq!(s, "2026-02-24T08:58:12Z");
        assert!(!s.contains('.'));
    }

    #[test]
    fn backoff_delay_within_jitter_bounds() {
        for attempt in 0..MAX_RETRIES {
            for _ in 0..20 {
                let delay = backoff_delay(attempt);
                let base_ms = RETRY_BASE_DELAY_MS << attempt;
                let min_ms = (base_ms as f64 * 0.8) as u64;
                let max_ms = (base_ms as f64 * 1.2) as u64;
                let actual_ms = delay.as_millis() as u64;
                assert!(
                    actual_ms >= min_ms && actual_ms <= max_ms,
                    "attempt {attempt}: delay {actual_ms}ms not in [{min_ms}, {max_ms}]"
                );
            }
        }
    }

    #[test]
    fn backoff_delay_doubles_each_attempt() {
        for attempt in 1..MAX_RETRIES {
            let prev_base = RETRY_BASE_DELAY_MS << (attempt - 1);
            let curr_base = RETRY_BASE_DELAY_MS << attempt;
            assert_eq!(
                curr_base,
                prev_base * 2,
                "base delay should double from attempt {} to {}",
                attempt - 1,
                attempt
            );
        }
    }

    #[test]
    fn max_retries_constant() {
        assert_eq!(MAX_RETRIES, 3, "expected 3 retries");
    }

    // --- activity_windows ---

    fn dt(s: &str) -> OffsetDateTime {
        OffsetDateTime::parse(s, &Iso8601::DEFAULT).unwrap()
    }

    #[test]
    fn activity_windows_empty_range_returns_empty() {
        let start = dt("2026-01-01T00:00:00Z");
        assert!(activity_windows(start, start).is_empty());
        // end before start also empty
        assert!(activity_windows(start, start - time::Duration::days(1)).is_empty());
    }

    #[test]
    fn activity_windows_single_window_when_range_within_31_days() {
        let start = dt("2026-01-01T00:00:00Z");
        let end = start + time::Duration::days(30);
        let windows = activity_windows(start, end);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0], (start, end));
    }

    #[test]
    fn activity_windows_exactly_30_days_is_single_window() {
        let start = dt("2026-01-01T00:00:00Z");
        let end = start + time::Duration::days(30);
        let windows = activity_windows(start, end);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0], (start, end));
    }

    #[test]
    fn activity_windows_31_days_splits_into_two() {
        let start = dt("2026-01-01T00:00:00Z");
        let end = start + time::Duration::days(31);
        let windows = activity_windows(start, end);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0], (start, start + time::Duration::days(30)));
        assert_eq!(windows[1], (start + time::Duration::days(30), end));
    }

    #[test]
    fn activity_windows_365_days_all_within_limit_and_contiguous() {
        let start = dt("2026-01-01T00:00:00Z");
        let end = start + time::Duration::days(365);
        let windows = activity_windows(start, end);
        // 365 / 30 = 12 full + 5 remaining = 13
        assert_eq!(windows.len(), 13);
        assert_eq!(windows[0].0, start);
        assert_eq!(windows.last().unwrap().1, end);
        for (ws, we) in &windows {
            assert!(
                (*we - *ws).whole_days() <= 30,
                "window exceeds 30 days: {} days",
                (*we - *ws).whole_days()
            );
        }
        // Contiguous: each window starts where the previous ended
        for i in 1..windows.len() {
            assert_eq!(
                windows[i].0,
                windows[i - 1].1,
                "gap between window {i} and {}",
                i - 1
            );
        }
    }

    // --- 401 retry tests ---

    #[tokio::test]
    async fn get_retries_on_401_after_force_refresh() {
        let server = MockServer::start().await;
        let api_server = format!("{}/", server.uri());

        // First API call with stale token → 401.
        Mock::given(method("GET"))
            .and(path("/v1/time"))
            .and(header("Authorization", "Bearer stale_token"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .named("stale request")
            .mount(&server)
            .await;

        // OAuth refresh → new token (api_server stays the same mock).
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "fresh_token",
                "token_type": "Bearer",
                "expires_in": 1800,
                "refresh_token": "new_rt",
                "api_server": api_server,
            })))
            .expect(1)
            .named("oauth refresh")
            .mount(&server)
            .await;

        // Retry with fresh token → success.
        Mock::given(method("GET"))
            .and(path("/v1/time"))
            .and(header("Authorization", "Bearer fresh_token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"time": "2026-03-02T12:00:00.000000-05:00"})),
            )
            .expect(1)
            .named("fresh request")
            .mount(&server)
            .await;

        // Build client with stale cached token.
        let cached = CachedToken {
            access_token: "stale_token".to_string(),
            api_server: api_server.clone(),
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };
        let tm = TokenManager::new_with_login_url(
            "old_rt".to_string(),
            None,
            server.uri(),
            Some(cached),
        )
        .await
        .unwrap();

        let client = QuestradeClient::new(tm).unwrap();
        let time = client.get_server_time().await.unwrap();
        assert_eq!(time.year(), 2026);
    }

    #[tokio::test]
    async fn get_does_not_retry_401_more_than_once() {
        let server = MockServer::start().await;
        let api_server = format!("{}/", server.uri());

        // API always returns 401.
        Mock::given(method("GET"))
            .and(path("/v1/time"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .expect(2) // initial + one retry = 2
            .mount(&server)
            .await;

        // OAuth refresh succeeds (but the new token is still rejected).
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "still_bad",
                "token_type": "Bearer",
                "expires_in": 1800,
                "refresh_token": "new_rt",
                "api_server": api_server,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let cached = CachedToken {
            access_token: "stale_token".to_string(),
            api_server,
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };
        let tm = TokenManager::new_with_login_url(
            "old_rt".to_string(),
            None,
            server.uri(),
            Some(cached),
        )
        .await
        .unwrap();

        let client = QuestradeClient::new(tm).unwrap();
        let result = client.get_server_time().await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("401"),
            "error should mention 401"
        );
    }

    #[tokio::test]
    async fn post_retries_on_401_after_force_refresh() {
        let server = MockServer::start().await;
        let api_server = format!("{}/", server.uri());

        // First POST with stale token → 401.
        Mock::given(method("POST"))
            .and(path("/v1/markets/quotes/options"))
            .and(header("Authorization", "Bearer stale_token"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .named("stale post")
            .mount(&server)
            .await;

        // OAuth refresh → new token.
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "fresh_token",
                "token_type": "Bearer",
                "expires_in": 1800,
                "refresh_token": "new_rt",
                "api_server": api_server,
            })))
            .expect(1)
            .named("oauth refresh")
            .mount(&server)
            .await;

        // Retry POST with fresh token → success.
        Mock::given(method("POST"))
            .and(path("/v1/markets/quotes/options"))
            .and(header("Authorization", "Bearer fresh_token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"optionQuotes": []})),
            )
            .expect(1)
            .named("fresh post")
            .mount(&server)
            .await;

        let cached = CachedToken {
            access_token: "stale_token".to_string(),
            api_server,
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };
        let tm = TokenManager::new_with_login_url(
            "old_rt".to_string(),
            None,
            server.uri(),
            Some(cached),
        )
        .await
        .unwrap();

        let client = QuestradeClient::new(tm).unwrap();
        let quotes = client.get_option_quotes_raw(&[12345]).await.unwrap();
        assert!(quotes.is_empty());
    }

    #[tokio::test]
    async fn get_with_raw_logging_deserializes_correctly() {
        let server = MockServer::start().await;
        let api_server = format!("{}/", server.uri());

        Mock::given(method("GET"))
            .and(path("/v1/time"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"time": "2026-03-02T12:00:00.000000-05:00"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let cached = CachedToken {
            access_token: "token".to_string(),
            api_server,
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };
        let tm =
            TokenManager::new_with_login_url("rt".to_string(), None, server.uri(), Some(cached))
                .await
                .unwrap();

        let client = QuestradeClient::new(tm).unwrap().with_raw_logging(true);
        let time = client.get_server_time().await.unwrap();
        assert_eq!(time.year(), 2026);
    }

    #[tokio::test]
    async fn get_text_returns_raw_body() {
        let server = MockServer::start().await;
        let api_server = format!("{}/", server.uri());

        let expected_json = r#"{"time":"2026-03-02T12:00:00.000000-05:00"}"#;
        Mock::given(method("GET"))
            .and(path("/v1/time"))
            .respond_with(ResponseTemplate::new(200).set_body_string(expected_json))
            .expect(1)
            .mount(&server)
            .await;

        let cached = CachedToken {
            access_token: "token".to_string(),
            api_server,
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };
        let tm =
            TokenManager::new_with_login_url("rt".to_string(), None, server.uri(), Some(cached))
                .await
                .unwrap();

        let client = QuestradeClient::new(tm).unwrap();
        let text = client.get_text("/time").await.unwrap();
        assert_eq!(text, expected_json);
    }

    // --- 429 retry tests ---

    #[tokio::test]
    async fn get_retries_on_429_then_succeeds() {
        let server = MockServer::start().await;
        let api_server = format!("{}/", server.uri());

        Mock::given(method("GET"))
            .and(path("/v1/time"))
            .respond_with(ResponseTemplate::new(429))
            .expect(2)
            .up_to_n_times(2)
            .named("rate limited")
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/v1/time"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"time": "2026-03-02T12:00:00.000000-05:00"})),
            )
            .expect(1)
            .named("success after rate limit")
            .mount(&server)
            .await;

        let cached = CachedToken {
            access_token: "token".to_string(),
            api_server,
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };
        let tm =
            TokenManager::new_with_login_url("rt".to_string(), None, server.uri(), Some(cached))
                .await
                .unwrap();

        let client = QuestradeClient::new(tm).unwrap();
        let time = client.get_server_time().await.unwrap();
        assert_eq!(time.year(), 2026);
    }

    #[tokio::test]
    async fn post_retries_on_429_then_succeeds() {
        let server = MockServer::start().await;
        let api_server = format!("{}/", server.uri());

        Mock::given(method("POST"))
            .and(path("/v1/markets/quotes/options"))
            .respond_with(ResponseTemplate::new(429))
            .expect(1)
            .up_to_n_times(1)
            .named("rate limited post")
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1/markets/quotes/options"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"optionQuotes": []})),
            )
            .expect(1)
            .named("success post after rate limit")
            .mount(&server)
            .await;

        let cached = CachedToken {
            access_token: "token".to_string(),
            api_server,
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };
        let tm =
            TokenManager::new_with_login_url("rt".to_string(), None, server.uri(), Some(cached))
                .await
                .unwrap();

        let client = QuestradeClient::new(tm).unwrap();
        let quotes = client.get_option_quotes_raw(&[12345]).await.unwrap();
        assert!(quotes.is_empty());
    }

    #[tokio::test]
    async fn get_fails_after_max_429_retries() {
        let server = MockServer::start().await;
        let api_server = format!("{}/", server.uri());

        Mock::given(method("GET"))
            .and(path("/v1/time"))
            .respond_with(ResponseTemplate::new(429))
            .expect((MAX_RETRIES + 1) as u64)
            .mount(&server)
            .await;

        let cached = CachedToken {
            access_token: "token".to_string(),
            api_server,
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };
        let tm =
            TokenManager::new_with_login_url("rt".to_string(), None, server.uri(), Some(cached))
                .await
                .unwrap();

        let client = QuestradeClient::new(tm).unwrap();
        let result = client.get_server_time().await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("rate limit"),
            "error should mention rate limit"
        );
    }

    #[test]
    fn retry_after_header_is_respected() {
        let resp = http::Response::builder()
            .status(429)
            .header("Retry-After", "5")
            .body("")
            .unwrap();
        let resp = reqwest::Response::from(resp);
        let delay = retry_after_or_backoff(&resp, 0);
        assert_eq!(delay, Duration::from_secs(5));
    }

    #[test]
    fn retry_after_header_capped_at_60s() {
        let resp = http::Response::builder()
            .status(429)
            .header("Retry-After", "300")
            .body("")
            .unwrap();
        let resp = reqwest::Response::from(resp);
        let delay = retry_after_or_backoff(&resp, 0);
        assert_eq!(delay, Duration::from_secs(60));
    }

    #[test]
    fn retry_after_missing_falls_back_to_backoff() {
        let resp = http::Response::builder().status(429).body("").unwrap();
        let resp = reqwest::Response::from(resp);
        let delay = retry_after_or_backoff(&resp, 0);
        let ms = delay.as_millis() as u64;
        assert!(ms >= 800 && ms <= 1200, "expected ~1000ms, got {}ms", ms);
    }
}
