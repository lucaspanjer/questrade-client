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

/// Pre-existing token state that can be passed to [`TokenManager::new`] to
/// skip the initial token refresh when a valid cached token is available.
#[derive(Debug, Clone)]
pub struct CachedToken {
    /// Bearer access token from a previous session.
    pub access_token: String,
    /// API server URL that was returned alongside this access token.
    pub api_server: String,
    /// When this access token expires.
    pub expires_at: OffsetDateTime,
}

/// Token state read back from a [`TokenStore`] just before a refresh.
#[derive(Debug, Clone)]
pub struct StoredToken {
    /// The most recently persisted refresh token.
    pub refresh_token: String,
    /// The most recently persisted access token, if any. If it is still valid
    /// (and is not the token the server just rejected) it is adopted instead
    /// of spending the refresh token.
    pub cached: Option<CachedToken>,
}

/// Persistent storage for Questrade tokens.
///
/// Questrade refresh tokens are single-use, so every refresh must be persisted
/// via [`save`](Self::save). When several processes or [`TokenManager`]s share
/// one refresh token, implement [`lock`](Self::lock) and [`load`](Self::load)
/// as well: before each refresh the manager takes the lock, loads the latest
/// stored state (adopting a still-valid access token that someone else just
/// fetched, or using their newer refresh token), refreshes, saves, and only
/// then releases the lock. That keeps two holders from spending the same
/// refresh token.
///
/// All methods are blocking. `lock` runs on a blocking thread so it may wait;
/// `load` and `save` should be quick (e.g. a small file read/write). None of
/// them may call back into the [`TokenManager`].
///
/// Any `Fn(&TokenResponse)` closure is a save-only store:
///
/// ```no_run
/// # use std::sync::Arc;
/// # use questrade_client::{TokenManager, TokenResponse};
/// # async fn example() -> questrade_client::Result<()> {
/// let manager = TokenManager::new(
///     "refresh_token".to_string(),
///     false,
///     Some(Arc::new(|token: &TokenResponse| {
///         std::fs::write("/tmp/refresh_token", &token.refresh_token).ok();
///     })),
///     None,
/// )
/// .await?;
/// # Ok(())
/// # }
/// ```
pub trait TokenStore: Send + Sync + 'static {
    /// Acquire exclusive access to the stored tokens. The returned guard is
    /// held from before [`load`](Self::load) until after [`save`](Self::save)
    /// and released by dropping it. The default does no locking.
    fn lock(&self) -> std::io::Result<Box<dyn Send>> {
        Ok(Box::new(()))
    }

    /// Return the latest persisted token state, or `None` to keep using the
    /// manager's in-memory refresh token. The default returns `None`.
    fn load(&self) -> Option<StoredToken> {
        None
    }

    /// Persist a freshly refreshed token. The new `refresh_token` **must** be
    /// saved; the previous one is no longer valid.
    fn save(&self, token: &TokenResponse);
}

impl<F> TokenStore for F
where
    F: Fn(&TokenResponse) + Send + Sync + 'static,
{
    fn save(&self, token: &TokenResponse) {
        self(token)
    }
}

/// Store used when none is given: nothing is persisted.
struct NoStore;

impl TokenStore for NoStore {
    fn save(&self, _token: &TokenResponse) {}
}

/// Manages Questrade OAuth tokens with auto-refresh.
#[derive(Clone)]
pub struct TokenManager {
    inner: Arc<RwLock<TokenState>>,
    login_url: String,
    store: Arc<dyn TokenStore>,
}

struct TokenState {
    access_token: String,
    api_server: String,
    refresh_token: String,
    expires_at: OffsetDateTime,
}

/// Whether an access token can be used: non-empty, unexpired, and not the
/// token the server just rejected.
fn usable(token: &str, expires_at: OffsetDateTime, rejected: Option<&str>) -> bool {
    !token.is_empty() && OffsetDateTime::now_utc() < expires_at && rejected != Some(token)
}

impl TokenManager {
    /// Create a new TokenManager with the given refresh token.
    ///
    /// `store` persists rotated tokens and, optionally, coordinates refreshes
    /// with other holders of the same refresh token (see [`TokenStore`]).
    /// Pass `None` to keep tokens in memory only (e.g. in tests).
    ///
    /// If `cached_token` is provided and still valid, the initial token refresh
    /// is skipped and the cached credentials are used directly.
    pub async fn new(
        refresh_token: String,
        practice: bool,
        store: Option<Arc<dyn TokenStore>>,
        cached_token: Option<CachedToken>,
    ) -> Result<Self> {
        let login_url = if practice {
            "https://practicelogin.questrade.com".to_string()
        } else {
            "https://login.questrade.com".to_string()
        };
        Self::new_with_login_url(refresh_token, store, login_url, cached_token).await
    }

    /// Like [`Self::new`] but accepts an explicit login URL.
    /// Used internally and in tests (e.g. to point at a wiremock server).
    pub async fn new_with_login_url(
        refresh_token: String,
        store: Option<Arc<dyn TokenStore>>,
        login_url: String,
        cached_token: Option<CachedToken>,
    ) -> Result<Self> {
        let store: Arc<dyn TokenStore> = store.unwrap_or_else(|| Arc::new(NoStore));

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
            store,
        };

        // Only refresh if we don't have a valid token.
        if manager.inner.read().await.access_token.is_empty() {
            manager.refresh(None).await?;
        }

        Ok(manager)
    }

    /// Get a valid access token and API server URL, refreshing if needed.
    pub async fn get_token(&self) -> Result<(String, String)> {
        {
            let state = self.inner.read().await;
            if usable(&state.access_token, state.expires_at, None) {
                return Ok((state.access_token.clone(), state.api_server.clone()));
            }
        }
        // Token expired, refresh
        self.refresh(None).await
    }

    /// Force a token refresh even if the current token has not expired.
    ///
    /// Used when the server returns 401 Unauthorized, indicating the access
    /// token was revoked server-side before its stated expiry.
    pub async fn force_refresh(&self) -> Result<(String, String)> {
        let rejected = self.inner.read().await.access_token.clone();
        self.refresh(Some(rejected)).await
    }

    /// Replace an access token the server rejected with 401.
    ///
    /// Unlike [`Self::force_refresh`], this is a no-op if the current token
    /// has already moved past `rejected`, so concurrent requests that all hit
    /// 401 with the same token trigger a single refresh.
    pub(crate) async fn refresh_rejected(&self, rejected: &str) -> Result<(String, String)> {
        self.refresh(Some(rejected.to_string())).await
    }

    /// Run the refresh on its own task so that dropping the caller's future
    /// (a timeout, a disconnected client) cannot abandon it after Questrade
    /// has rotated the refresh token but before the new one is stored.
    async fn refresh(&self, rejected: Option<String>) -> Result<(String, String)> {
        let this = self.clone();
        let task = tokio::spawn(async move { this.refresh_inner(rejected.as_deref()).await });
        match task.await {
            Ok(result) => result,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(e) => Err(QuestradeError::TokenStore(std::io::Error::other(e))),
        }
    }

    async fn refresh_inner(&self, rejected: Option<&str>) -> Result<(String, String)> {
        let mut state = self.inner.write().await;

        // Double-check after acquiring write lock: another task may have
        // refreshed while we waited.
        if usable(&state.access_token, state.expires_at, rejected) {
            return Ok((state.access_token.clone(), state.api_server.clone()));
        }

        // Take the store's lock and read the latest persisted state, so we
        // never spend a refresh token another process has already used.
        let store = self.store.clone();
        let (guard, stored) = tokio::task::spawn_blocking(move || {
            let guard = store.lock()?;
            Ok::<_, std::io::Error>((guard, store.load()))
        })
        .await
        .map_err(|e| QuestradeError::TokenStore(std::io::Error::other(e)))?
        .map_err(QuestradeError::TokenStore)?;

        if let Some(stored) = stored {
            if let Some(ct) = stored.cached
                && usable(&ct.access_token, ct.expires_at, rejected)
                && ct.access_token != state.access_token
            {
                info!("adopting Questrade access token refreshed elsewhere");
                state.access_token = ct.access_token;
                state.api_server = ct.api_server;
                state.expires_at = ct.expires_at;
                state.refresh_token = stored.refresh_token;
                return Ok((state.access_token.clone(), state.api_server.clone()));
            }
            if !stored.refresh_token.is_empty() {
                state.refresh_token = stored.refresh_token;
            }
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
        drop(state);

        // Persist before releasing the store lock so the next holder sees it.
        self.store.save(&token_resp);
        drop(guard);

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    async fn closure_store_saves_new_token_on_refresh() {
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
        let store: Arc<dyn TokenStore> = Arc::new(move |t: &TokenResponse| {
            seen_clone.lock().unwrap().push(t.refresh_token.clone());
        });

        TokenManager::new_with_login_url("seed_token".to_string(), Some(store), server.uri(), None)
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

    fn valid_cached(token: &str) -> CachedToken {
        CachedToken {
            access_token: token.to_string(),
            api_server: "https://api01.iq.questrade.com/".to_string(),
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(25),
        }
    }

    /// In-memory store that mimics a shared file: `load` returns whatever was
    /// last saved (or seeded), and the lock guard flags whether it is held.
    #[derive(Default)]
    struct SharedStore {
        stored: Mutex<Option<StoredToken>>,
        locked: Arc<AtomicBool>,
        locks: AtomicUsize,
        saved_while_locked: AtomicBool,
    }

    struct Guard(Arc<AtomicBool>);

    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    impl TokenStore for SharedStore {
        fn lock(&self) -> std::io::Result<Box<dyn Send>> {
            self.locks.fetch_add(1, Ordering::SeqCst);
            self.locked.store(true, Ordering::SeqCst);
            Ok(Box::new(Guard(self.locked.clone())))
        }

        fn load(&self) -> Option<StoredToken> {
            self.stored.lock().unwrap().clone()
        }

        fn save(&self, token: &TokenResponse) {
            self.saved_while_locked
                .store(self.locked.load(Ordering::SeqCst), Ordering::SeqCst);
            *self.stored.lock().unwrap() = Some(StoredToken {
                refresh_token: token.refresh_token.clone(),
                cached: Some(valid_cached(&token.access_token)),
            });
        }
    }

    #[tokio::test]
    async fn refresh_uses_refresh_token_from_store() {
        // Another process already rotated the token; the in-memory one is stale.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .and(query_param("refresh_token", "newer_from_store"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_body("rotated")))
            .expect(1)
            .mount(&server)
            .await;

        let store = Arc::new(SharedStore::default());
        *store.stored.lock().unwrap() = Some(StoredToken {
            refresh_token: "newer_from_store".to_string(),
            cached: None,
        });

        TokenManager::new_with_login_url(
            "stale_in_memory".to_string(),
            Some(store.clone()),
            server.uri(),
            None,
        )
        .await
        .unwrap();

        let stored = store.load().unwrap();
        assert_eq!(stored.refresh_token, "rotated");
        assert!(store.saved_while_locked.load(Ordering::SeqCst));
        assert!(!store.locked.load(Ordering::SeqCst), "lock released");
    }

    #[tokio::test]
    async fn valid_token_in_store_is_adopted_without_refresh() {
        let store = Arc::new(SharedStore::default());
        *store.stored.lock().unwrap() = Some(StoredToken {
            refresh_token: "rt_from_other_process".to_string(),
            cached: Some(valid_cached("fresh_from_other_process")),
        });

        let manager = TokenManager::new_with_login_url(
            "rt".to_string(),
            Some(store),
            "http://127.0.0.1:1".to_string(), // unreachable — proves no refresh happens
            Some(valid_cached("revoked")),
        )
        .await
        .unwrap();

        let (token, _) = manager.force_refresh().await.unwrap();
        assert_eq!(token, "fresh_from_other_process");
    }

    #[tokio::test]
    async fn rejected_token_in_store_is_not_adopted() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .and(query_param("refresh_token", "rt_stored"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_body("rotated")))
            .expect(1)
            .mount(&server)
            .await;

        let store = Arc::new(SharedStore::default());
        *store.stored.lock().unwrap() = Some(StoredToken {
            refresh_token: "rt_stored".to_string(),
            cached: Some(valid_cached("revoked")),
        });

        let manager = TokenManager::new_with_login_url(
            "rt".to_string(),
            Some(store),
            server.uri(),
            Some(valid_cached("revoked")),
        )
        .await
        .unwrap();

        let (token, _) = manager.refresh_rejected("revoked").await.unwrap();
        assert_eq!(token, "acc_123");
    }

    #[tokio::test]
    async fn concurrent_rejections_of_same_token_refresh_once() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_token_body("rotated")))
            .expect(1)
            .mount(&server)
            .await;

        let manager = TokenManager::new_with_login_url(
            "rt".to_string(),
            None,
            server.uri(),
            Some(valid_cached("revoked")),
        )
        .await
        .unwrap();

        let handles: Vec<_> = (0..5)
            .map(|_| {
                let m = manager.clone();
                tokio::spawn(async move { m.refresh_rejected("revoked").await })
            })
            .collect();
        for h in handles {
            assert_eq!(h.await.unwrap().unwrap().0, "acc_123");
        }
    }

    #[tokio::test]
    async fn refresh_completes_even_if_caller_is_dropped() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(mock_token_body("rotated"))
                    .set_delay(std::time::Duration::from_millis(200)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let store = Arc::new(SharedStore::default());
        let manager = TokenManager::new_with_login_url(
            "rt".to_string(),
            Some(store.clone()),
            server.uri(),
            Some(valid_cached("revoked")),
        )
        .await
        .unwrap();

        // Caller gives up before the auth server answers.
        let timed_out = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            manager.force_refresh(),
        )
        .await;
        assert!(timed_out.is_err());

        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert_eq!(store.load().unwrap().refresh_token, "rotated");
        let (token, _) = manager.get_token().await.unwrap();
        assert_eq!(token, "acc_123");
    }
}
