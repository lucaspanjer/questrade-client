//! Async Rust client for the [Questrade REST API](https://www.questrade.com/api/documentation/getting-started).
//!
//! Handles OAuth token refresh, typed market-data access (quotes, option
//! chains, candles), and account-data access (positions, balances, activities).
//!
//! ## Quick start
//!
//! ```no_run
//! use questrade_client::{TokenManager, QuestradeClient};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let manager = TokenManager::new(
//!     "your_refresh_token".to_string(),
//!     false, // false = live account, true = practice account
//!     None,  // optional token store for persisting rotated tokens
//!     None,  // optional cached token to skip initial refresh
//! ).await?;
//!
//! let client = QuestradeClient::new(manager)?;
//! let accounts = client.get_accounts().await?;
//! println!("accounts: {:?}", accounts);
//! # Ok(())
//! # }
//! ```
//!
//! ## Token persistence
//!
//! Questrade refresh tokens are **single-use**. Pass a [`TokenStore`] to
//! [`TokenManager::new`] to persist the rotated token after every automatic
//! refresh so your next session can authenticate successfully. A store that
//! also implements `lock` and `load` lets several processes share one
//! refresh token without invalidating each other's.

#![deny(missing_docs)]

pub mod api_types;
pub mod auth;
pub mod client;
pub mod error;
pub(crate) mod rate_limit;

pub use auth::{CachedToken, StoredToken, TokenManager, TokenResponse, TokenStore};
pub use client::{QuestradeClient, QuestradeClientBuilder};
pub use error::{QuestradeError, Result};
