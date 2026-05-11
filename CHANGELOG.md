# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-05-11

### Added

- Proactive rate limiting via `X-RateLimit-Remaining` and `X-RateLimit-Reset` response headers
- Independent tracking of account and market data rate-limit budgets
- Automatic request blocking when a category's budget is exhausted, resuming when the window resets
- Integration tests for proactive rate limiting, 429 recovery with headers, and category independence
- Batch `get_raw_quotes` and `get_symbols` methods that fetch multiple IDs in a single request
- `tracing` `error!` logs for `reqwest` send failures and non-success HTTP responses (with response body), plus `debug!` logs for every response status across `get_json` / `post_json` / `get_text`

### Changed

- 429 retry now defers to header-based wait when rate-limit headers are present, falling back to exponential backoff only when headers are missing
- Rate-limit log messages now include a `reason` field and show `reset_at` as an RFC 3339 timestamp with `wait_secs`, instead of a raw epoch; redundant 429+header warning removed and proactive sleep demoted to `debug!`

### Fixed

- Batch quotes and symbols endpoints now use `?ids=` query parameters; Questrade's path-param form (`/markets/quotes/1,2,3`, `/symbols/1,2,3`) returns HTTP 400
- `/symbols` is now classified under the account rate-limit bucket to match Questrade's empirical behaviour (only `/markets/*` uses the market-data bucket, despite the docs)
- Candles endpoint now uses the short datetime format used by other endpoints, avoiding "Argument length exceeds imposed limit" errors
- Rate-limit `remaining` is logged as an integer instead of `Some(N)`

### Documentation

- README documents API variances from the published Questrade docs: batch endpoints require query params, `/symbols` uses the account rate-limit bucket, headers track only hourly quota, and per-second limits are not surfaced

## [0.1.2] - 2026-03-11

### Added

- Strategy quotes (`POST /v1/markets/quotes/strategies`)

## [0.1.1] - 2026-03-08

### Changed

- Bump MSRV to 1.88 for let chains stabilization

### Fixed

- Resolve rustdoc warnings for intra-doc links


## [0.1.0] - 2026-03-07

### Added

- `TokenManager` with automatic OAuth token refresh and single-use token rotation
- `CachedToken` support to skip initial OAuth round-trip
- `OnTokenRefresh` callback for token persistence
- `QuestradeClient` with transparent 401 retry and 429 rate-limit backoff
- Server time (`GET /v1/time`)
- Symbol search and resolution (`GET /v1/symbols/search`)
- Symbol detail (`GET /v1/symbols/:id`)
- Equity quotes (`GET /v1/markets/quotes/:id`)
- Option chain structure (`GET /v1/symbols/:id/options`)
- Option quotes with batching (`POST /v1/markets/quotes/options`)
- Historical candles (`GET /v1/markets/candles/:id`)
- Account listing (`GET /v1/accounts`)
- Position retrieval (`GET /v1/accounts/:id/positions`)
- Account balances (`GET /v1/accounts/:id/balances`)
- Account activities with automatic 30-day windowing (`GET /v1/accounts/:id/activities`)
- Order retrieval with state filtering (`GET /v1/accounts/:id/orders`)
- Execution retrieval with automatic 30-day windowing (`GET /v1/accounts/:id/executions`)
- Market info (`GET /v1/markets`)
- Raw response access via `get_text` for debugging
- Raw response logging mode via `with_raw_logging`
- Fully typed serde request/response types for all endpoints
- `tracing` instrumentation throughout
- Integration tests with wiremock and JSON fixtures
- `dump_responses` and `token_manager` examples
