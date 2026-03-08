//! Error types for the questrade-client crate.

use reqwest::StatusCode;

/// Errors returned by the Questrade API client.
#[derive(Debug, thiserror::Error)]
pub enum QuestradeError {
    /// An HTTP/network error from the underlying `reqwest` client.
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The Questrade API returned a non-success status code.
    #[error("Questrade API error ({status}): {body}")]
    Api {
        /// HTTP status code returned by the API.
        status: StatusCode,
        /// Response body (may contain a JSON error object or plain text).
        body: String,
    },

    /// Rate-limited (HTTP 429) after exhausting all retry attempts.
    #[error("Questrade API rate limit exceeded after {retries} retries")]
    RateLimited {
        /// Number of retry attempts made before giving up.
        retries: u32,
    },

    /// OAuth token refresh failed with a non-success status.
    #[error("Token refresh failed ({status}): {body}")]
    TokenRefresh {
        /// HTTP status code from the auth server.
        status: StatusCode,
        /// Response body from the auth server.
        body: String,
    },

    /// Failed to deserialize a JSON response body.
    #[error("Failed to parse response: {0}")]
    Deserialization(#[from] serde_json::Error),

    /// A datetime string could not be formatted or parsed.
    #[error("{context}: {source}")]
    DateTime {
        /// What we were trying to do (e.g. "Failed to parse datetime: …").
        context: String,
        /// The underlying `time` format or parse error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A symbol lookup returned no matching result.
    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),

    /// A response was expected to contain at least one item but was empty.
    #[error("{0}")]
    EmptyResponse(String),
}

/// Convenience type alias for `Result<T, QuestradeError>`.
pub type Result<T> = std::result::Result<T, QuestradeError>;
