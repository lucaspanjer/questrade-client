//! Proactive rate-limit tracking for the Questrade API.
//!
//! Reads `X-RateLimit-Remaining` and `X-RateLimit-Reset` response headers and
//! blocks outgoing requests when a category's budget is exhausted, resuming
//! automatically when the window resets.
//!
//! Questrade enforces two independent rate-limit buckets:
//!
//! | Category    | Per-second | Per-hour |
//! |-------------|-----------|----------|
//! | Account     | 30        | 30,000   |
//! | Market data | 20        | 15,000   |

use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::{debug, info};

/// Which Questrade rate-limit bucket a request falls into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RateLimitCategory {
    /// `/time`, `/accounts/…` — 30 req/s, 30 000 req/hr.
    Account,
    /// `/symbols/…`, `/markets/…` — 20 req/s, 15 000 req/hr.
    MarketData,
}

impl std::fmt::Display for RateLimitCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Account => write!(f, "account"),
            Self::MarketData => write!(f, "market_data"),
        }
    }
}

/// Per-category rate-limit state from response headers.
#[derive(Debug, Clone, Default)]
struct CategoryState {
    /// Number of requests remaining in the current window (`X-RateLimit-Remaining`).
    remaining: Option<u32>,
    /// Unix epoch seconds when the current window resets (`X-RateLimit-Reset`).
    reset_epoch: Option<u64>,
}

/// Thread-safe tracker for Questrade API rate limits.
///
/// Uses `std::sync::RwLock` because critical sections are trivially short
/// (reading/writing two integers) and never held across `.await` points.
pub(crate) struct RateLimiter {
    account: RwLock<CategoryState>,
    market_data: RwLock<CategoryState>,
}

impl RateLimiter {
    pub(crate) fn new() -> Self {
        Self {
            account: RwLock::new(CategoryState::default()),
            market_data: RwLock::new(CategoryState::default()),
        }
    }

    /// Classify an API path into a rate-limit category.
    pub(crate) fn classify(path: &str) -> RateLimitCategory {
        if path.starts_with("/time") || path.starts_with("/accounts") {
            RateLimitCategory::Account
        } else {
            // /symbols, /markets, and anything else → market data
            RateLimitCategory::MarketData
        }
    }

    fn state_for(&self, category: RateLimitCategory) -> &RwLock<CategoryState> {
        match category {
            RateLimitCategory::Account => &self.account,
            RateLimitCategory::MarketData => &self.market_data,
        }
    }

    /// If the given category's remaining count is 0, return how long to wait
    /// until the window resets. Returns `None` if requests are still available,
    /// the reset time has already passed, or no state has been recorded yet.
    pub(crate) fn wait_duration(&self, category: RateLimitCategory) -> Option<Duration> {
        let state = self
            .state_for(category)
            .read()
            .expect("rate limit lock poisoned");
        if state.remaining != Some(0) {
            return None;
        }
        let reset_epoch = state.reset_epoch?;
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_secs();
        if reset_epoch > now_epoch {
            // Add 100 ms buffer to avoid racing the window boundary.
            Some(Duration::from_secs(reset_epoch - now_epoch) + Duration::from_millis(100))
        } else {
            None
        }
    }

    /// Update the tracked state for `category` from HTTP response headers.
    ///
    /// Silently ignores missing or unparseable headers — the caller can still
    /// fall back to exponential backoff if a 429 arrives without headers.
    pub(crate) fn update_from_headers(
        &self,
        category: RateLimitCategory,
        headers: &reqwest::header::HeaderMap,
    ) {
        let remaining = headers
            .get("X-RateLimit-Remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u32>().ok());

        let reset_epoch = headers
            .get("X-RateLimit-Reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());

        if remaining.is_none() && reset_epoch.is_none() {
            return;
        }

        let mut state = self
            .state_for(category)
            .write()
            .expect("rate limit lock poisoned");

        if let Some(r) = remaining {
            state.remaining = Some(r);
        }
        if let Some(e) = reset_epoch {
            state.reset_epoch = Some(e);
        }

        if state.remaining == Some(0) {
            info!(
                category = %category,
                reset_epoch = state.reset_epoch,
                "rate limit exhausted, will block requests until reset",
            );
        } else {
            debug!(
                category = %category,
                remaining = ?state.remaining,
                "rate limit state updated",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_account_endpoints() {
        assert_eq!(RateLimiter::classify("/time"), RateLimitCategory::Account);
        assert_eq!(
            RateLimiter::classify("/accounts/123/positions"),
            RateLimitCategory::Account,
        );
        assert_eq!(
            RateLimiter::classify("/accounts/123/balances"),
            RateLimitCategory::Account,
        );
        assert_eq!(
            RateLimiter::classify("/accounts/123/orders"),
            RateLimitCategory::Account,
        );
        assert_eq!(
            RateLimiter::classify("/accounts/123/executions"),
            RateLimitCategory::Account,
        );
        assert_eq!(
            RateLimiter::classify("/accounts/123/activities"),
            RateLimitCategory::Account,
        );
    }

    #[test]
    fn classify_market_data_endpoints() {
        assert_eq!(
            RateLimiter::classify("/symbols/search?prefix=AAPL"),
            RateLimitCategory::MarketData,
        );
        assert_eq!(
            RateLimiter::classify("/symbols/12345"),
            RateLimitCategory::MarketData,
        );
        assert_eq!(
            RateLimiter::classify("/symbols/12345/options"),
            RateLimitCategory::MarketData,
        );
        assert_eq!(
            RateLimiter::classify("/markets/quotes/12345"),
            RateLimitCategory::MarketData,
        );
        assert_eq!(
            RateLimiter::classify("/markets/quotes/options"),
            RateLimitCategory::MarketData,
        );
        assert_eq!(
            RateLimiter::classify("/markets/candles/12345"),
            RateLimitCategory::MarketData,
        );
        assert_eq!(
            RateLimiter::classify("/markets"),
            RateLimitCategory::MarketData,
        );
    }

    #[test]
    fn wait_duration_none_when_no_state() {
        let rl = RateLimiter::new();
        assert!(rl.wait_duration(RateLimitCategory::Account).is_none());
        assert!(rl.wait_duration(RateLimitCategory::MarketData).is_none());
    }

    #[test]
    fn wait_duration_none_when_remaining_positive() {
        let rl = RateLimiter::new();
        {
            let mut state = rl.account.write().unwrap();
            state.remaining = Some(10);
            state.reset_epoch = Some(u64::MAX);
        }
        assert!(rl.wait_duration(RateLimitCategory::Account).is_none());
    }

    #[test]
    fn wait_duration_some_when_exhausted_and_reset_in_future() {
        let rl = RateLimiter::new();
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 5;
        {
            let mut state = rl.account.write().unwrap();
            state.remaining = Some(0);
            state.reset_epoch = Some(future);
        }
        let wait = rl.wait_duration(RateLimitCategory::Account).unwrap();
        // Should be ~5 s + 100 ms buffer
        assert!(wait.as_secs() >= 4 && wait.as_secs() <= 6);
    }

    #[test]
    fn wait_duration_none_when_exhausted_but_reset_in_past() {
        let rl = RateLimiter::new();
        let past = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 10;
        {
            let mut state = rl.account.write().unwrap();
            state.remaining = Some(0);
            state.reset_epoch = Some(past);
        }
        assert!(rl.wait_duration(RateLimitCategory::Account).is_none());
    }

    #[test]
    fn wait_duration_none_when_exhausted_but_no_reset_epoch() {
        let rl = RateLimiter::new();
        {
            let mut state = rl.market_data.write().unwrap();
            state.remaining = Some(0);
            state.reset_epoch = None;
        }
        assert!(rl.wait_duration(RateLimitCategory::MarketData).is_none());
    }

    #[test]
    fn update_from_headers_parses_both_headers() {
        let rl = RateLimiter::new();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("X-RateLimit-Remaining", "42".parse().unwrap());
        headers.insert("X-RateLimit-Reset", "1700000000".parse().unwrap());

        rl.update_from_headers(RateLimitCategory::Account, &headers);

        let state = rl.account.read().unwrap();
        assert_eq!(state.remaining, Some(42));
        assert_eq!(state.reset_epoch, Some(1_700_000_000));
    }

    #[test]
    fn update_from_headers_ignores_missing_headers() {
        let rl = RateLimiter::new();
        let headers = reqwest::header::HeaderMap::new();

        rl.update_from_headers(RateLimitCategory::Account, &headers);

        let state = rl.account.read().unwrap();
        assert!(state.remaining.is_none());
        assert!(state.reset_epoch.is_none());
    }

    #[test]
    fn update_from_headers_ignores_malformed_values() {
        let rl = RateLimiter::new();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("X-RateLimit-Remaining", "not-a-number".parse().unwrap());
        headers.insert("X-RateLimit-Reset", "also-bad".parse().unwrap());

        rl.update_from_headers(RateLimitCategory::Account, &headers);

        let state = rl.account.read().unwrap();
        assert!(state.remaining.is_none());
        assert!(state.reset_epoch.is_none());
    }

    #[test]
    fn categories_are_independent() {
        let rl = RateLimiter::new();
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 10;

        // Exhaust account category
        {
            let mut state = rl.account.write().unwrap();
            state.remaining = Some(0);
            state.reset_epoch = Some(future);
        }

        // Account should block, market data should not
        assert!(rl.wait_duration(RateLimitCategory::Account).is_some());
        assert!(rl.wait_duration(RateLimitCategory::MarketData).is_none());
    }
}
