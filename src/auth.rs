//! OAuth token management for the Questrade API.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::error::{QuestradeError, Result};

/// Token response from the Questrade OAuth endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    /// Short-lived Bearer access token used to authenticate API requests.
    pub access_token: String,
    /// Token type; always `"Bearer"`.
    pub token_type: String,
    /// Token lifetime in seconds (typically 1800).
    pub expires_in: u64,
    /// Single-use refresh token. **Must be persisted** after every refresh —
    /// using an old token will result in an authentication failure.
    pub refresh_token: String,
    /// Base URL for API requests (e.g. `"https://api01.iq.questrade.com/"`).
    /// May change between refreshes; always use the most recently received value.
    pub api_server: String,
}

/// Callback invoked whenever a token refresh completes successfully.
/// Receives the full `TokenResponse`; the caller is responsible for persisting
/// the new `refresh_token` (Questrade refresh tokens are single-use).
pub type OnTokenRefresh = Arc<dyn Fn(TokenResponse) + Send + Sync>;

/// Pre-existing token state that can be passed to [`TokenManager::new`] to
/// skip the initial token refresh when a valid cached token is available.
pub struct CachedToken {
    /// Bearer access token from a previous session.
    pub access_token: String,
    /// API server URL that was returned alongside this access token.
    pub api_server: String,
    /// When this access token expires.
    pub expires_at: OffsetDateTime,
}

/// Manages Questrade OAuth tokens with auto-refresh.
#[derive(Clone)]
pub struct TokenManager {
    inner: Arc<RwLock<TokenState>>,
    login_url: String,
    on_token_refresh: OnTokenRefresh,
}

struct TokenState {
    access_token: String,
    api_server: String,
    refresh_token: String,
    expires_at: OffsetDateTime,
}

impl TokenManager {
    /// Create a new TokenManager with the given refresh token.
    ///
    /// `on_token_refresh` is called whenever the token is refreshed; pass `None`
    /// for a no-op (e.g. in tests that don't need persistence).
    ///
    /// If `cached_token` is provided and still valid, the initial token refresh
    /// is skipped and the cached credentials are used directly.
    pub async fn new(
        refresh_token: String,
        practice: bool,
        on_token_refresh: Option<OnTokenRefresh>,
        cached_token: Option<CachedToken>,
    ) -> Result<Self> {
        let login_url = if practice {
            "https://practicelogin.questrade.com".to_string()
        } else {
            "https://login.questrade.com".to_string()
        };
        Self::new_with_login_url(refresh_token, on_token_refresh, login_url, cached_token).await
    }

    /// Like [`new`] but accepts an explicit login URL.
    /// Used internally and in tests (e.g. to point at a wiremock server).
    pub async fn new_with_login_url(
        refresh_token: String,
        on_token_refresh: Option<OnTokenRefresh>,
        login_url: String,
        cached_token: Option<CachedToken>,
    ) -> Result<Self> {
        let cb: OnTokenRefresh = on_token_refresh.unwrap_or_else(|| Arc::new(|_| {}));

        // Use cached token if provided and still valid, otherwise start expired.
        let (access_token, api_server, expires_at) =
            if let Some(ct) = cached_token.filter(|ct| OffsetDateTime::now_utc() < ct.expires_at) {
                info!("reusing cached Questrade access token");
                (ct.access_token, ct.api_server, ct.expires_at)
            } else {
                (String::new(), String::new(), OffsetDateTime::UNIX_EPOCH)
            };

        let manager = Self {
            inner: Arc::new(RwLock::new(TokenState {
                access_token,
                api_server,
                refresh_token,
                expires_at,
            })),
            login_url,
            on_token_refresh: cb,
        };

        // Only refresh if we don't have a valid token.
        if manager.inner.read().await.access_token.is_empty() {
            manager.refresh().await?;
        }

        Ok(manager)
    }

    /// Get a valid access token and API server URL, refreshing if needed.
    pub async fn get_token(&self) -> Result<(String, String)> {
        {
            let state = self.inner.read().await;
            if OffsetDateTime::now_utc() < state.expires_at {
                return Ok((state.access_token.clone(), state.api_server.clone()));
            }
        }
        // Token expired, refresh
        self.refresh().await
    }

    /// Force a token refresh even if the current token has not expired.
    ///
    /// Used when the server returns 401 Unauthorized, indicating the access
    /// token was revoked server-side before its stated expiry.
    pub async fn force_refresh(&self) -> Result<(String, String)> {
        {
            let mut state = self.inner.write().await;
            state.expires_at = OffsetDateTime::UNIX_EPOCH;
            state.access_token.clear();
        }
        self.refresh().await
    }

    async fn refresh(&self) -> Result<(String, String)> {
        let mut state = self.inner.write().await;

        // Double-check after acquiring write lock
        if OffsetDateTime::now_utc() < state.expires_at && !state.access_token.is_empty() {
            return Ok((state.access_token.clone(), state.api_server.clone()));
        }

        info!("refreshing Questrade access token");

        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        let url = format!("{}/oauth2/token", self.login_url);

        let resp = client
            .get(&url)
            .query(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", state.refresh_token.as_str()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(QuestradeError::TokenRefresh { status, body });
        }

        let token_resp: TokenResponse = resp.json().await?;

        debug!(api_server = %token_resp.api_server, "new API server");

        let expires_at =
            OffsetDateTime::now_utc() + time::Duration::seconds(token_resp.expires_in as i64 - 30); // 30s buffer

        state.access_token = token_resp.access_token.clone();
        state.api_server = token_resp.api_server.clone();
        state.refresh_token = token_resp.refresh_token.clone();
        state.expires_at = expires_at;

        let result = (state.access_token.clone(), state.api_server.clone());
        drop(state); // release lock before invoking callback to prevent deadlock

        // Notify caller — token persistence is the caller's responsibility.
        (self.on_token_refresh)(token_resp);

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mock_token_body(refresh: &str) -> serde_json::Value {
        serde_json::json!({
            "access_token": "acc_123",
            "token_type": "Bearer",
            "expires_in": 1800,
            "refresh_token": refresh,
            "api_server": "https://api01.iq.questrade.com/"
        })
    }

    #[tokio::test]
    async fn callback_invoked_with_new_token_on_refresh() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .and(query_param("grant_type", "refresh_token"))
            .and(query_param("refresh_token", "seed_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_body("rotated")))
            .mount(&server)
            .await;

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let seen_clone = seen.clone();
        let cb: OnTokenRefresh = Arc::new(move |t: TokenResponse| {
            seen_clone.lock().unwrap().push(t.refresh_token.clone());
        });

        TokenManager::new_with_login_url("seed_token".to_string(), Some(cb), server.uri(), None)
            .await
            .unwrap();

        assert_eq!(*seen.lock().unwrap(), vec!["rotated"]);
    }

    #[tokio::test]
    async fn token_with_reserved_url_characters_is_encoded() {
        // Tokens containing '+', '=', '&' must be percent-encoded so they are
        // not misinterpreted as query-string delimiters.
        let tricky_token = "abc+def==&ghi";
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .and(query_param("grant_type", "refresh_token"))
            .and(query_param("refresh_token", tricky_token))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_body("rotated")))
            .mount(&server)
            .await;

        let result =
            TokenManager::new_with_login_url(tricky_token.to_string(), None, server.uri(), None)
                .await;
        assert!(result.is_ok(), "token with reserved chars should succeed");
    }

    #[tokio::test]
    async fn no_callback_constructs_successfully() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_body("tok")))
            .mount(&server)
            .await;

        let result =
            TokenManager::new_with_login_url("any".to_string(), None, server.uri(), None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cached_token_skips_initial_refresh() {
        // No mock server needed — if a refresh were attempted it would fail
        // because there's nothing to connect to.
        let cached = CachedToken {
            access_token: "cached_acc".to_string(),
            api_server: "https://api05.iq.questrade.com/".to_string(),
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };

        let manager = TokenManager::new_with_login_url(
            "unused_refresh".to_string(),
            None,
            "http://127.0.0.1:1".to_string(), // unreachable — proves no refresh happens
            Some(cached),
        )
        .await
        .unwrap();

        let (token, server) = manager.get_token().await.unwrap();
        assert_eq!(token, "cached_acc");
        assert_eq!(server, "https://api05.iq.questrade.com/");
    }

    #[tokio::test]
    async fn expired_cached_token_triggers_refresh() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_body("fresh")))
            .expect(1)
            .mount(&server)
            .await;

        let expired = CachedToken {
            access_token: "stale".to_string(),
            api_server: "https://old.example.com/".to_string(),
            expires_at: OffsetDateTime::now_utc() - time::Duration::seconds(1),
        };

        let manager =
            TokenManager::new_with_login_url("rt".to_string(), None, server.uri(), Some(expired))
                .await
                .unwrap();

        let (token, _) = manager.get_token().await.unwrap();
        assert_eq!(token, "acc_123");
    }

    #[tokio::test]
    async fn force_refresh_bypasses_valid_cached_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_body("refreshed")))
            .expect(1) // exactly one refresh call expected
            .mount(&server)
            .await;

        // Start with a valid cached token — normally no refresh would happen.
        let cached = CachedToken {
            access_token: "old_acc".to_string(),
            api_server: "https://api01.iq.questrade.com/".to_string(),
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        };

        let manager =
            TokenManager::new_with_login_url("rt".to_string(), None, server.uri(), Some(cached))
                .await
                .unwrap();

        // Confirm cached token is being used.
        let (token, _) = manager.get_token().await.unwrap();
        assert_eq!(token, "old_acc");

        // Force refresh should bypass the valid cache and hit the OAuth endpoint.
        let (token, _) = manager.force_refresh().await.unwrap();
        assert_eq!(token, "acc_123"); // from mock_token_body
    }
}
