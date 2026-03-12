//! Serde types for Questrade REST API request and response bodies.

use serde::{Deserialize, Deserializer, Serialize};

/// Deserializes a JSON null as 0.0, leaving numeric values unchanged.
/// Questrade occasionally returns null for numeric fields on certain positions.
fn null_as_zero<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.unwrap_or(0.0))
}

// ---------- Server time ----------

/// Response wrapper for `GET /v1/time`.
#[derive(Debug, Deserialize)]
pub struct ServerTimeResponse {
    /// Current server timestamp as an ISO 8601 string.
    pub time: String,
}

// ---------- Symbol search ----------

/// Response wrapper for `GET /v1/symbols/search`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolSearchResponse {
    /// Matching symbol results.
    pub symbols: Vec<SymbolResult>,
}

/// A single result from a symbol search.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolResult {
    /// Ticker symbol (e.g. `"AAPL"`).
    pub symbol: String,
    /// Questrade internal symbol ID.
    pub symbol_id: u64,
    /// Human-readable security name (e.g. `"Apple Inc."`).
    pub description: String,
    /// Security type: `"Stock"`, `"Option"`, `"ETF"`, etc.
    pub security_type: String,
    /// Primary listing exchange (e.g. `"NASDAQ"`).
    pub listing_exchange: String,
}

// ---------- Quotes ----------

/// Response wrapper for `GET /v1/markets/quotes/:id`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteResponse {
    /// Real-time level 1 quotes for the requested symbols.
    pub quotes: Vec<Quote>,
}

/// Real-time level 1 quote for an equity symbol.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    /// Ticker symbol.
    pub symbol: String,
    /// Questrade internal symbol ID.
    pub symbol_id: u64,
    /// Best bid price; `None` if no bid is available.
    pub bid_price: Option<f64>,
    /// Best ask price; `None` if no ask is available.
    pub ask_price: Option<f64>,
    /// Last trade price; `None` outside trading hours or on no prints.
    pub last_trade_price: Option<f64>,
    /// Total session volume in shares.
    pub volume: Option<u64>,
    /// Session open price.
    pub open_price: Option<f64>,
    /// Session high price.
    pub high_price: Option<f64>,
    /// Session low price.
    pub low_price: Option<f64>,
}

// ---------- Option chain structure ----------

/// Response wrapper for `GET /v1/symbols/:id/options`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionChainResponse {
    /// Option expiry groups for the underlying symbol.
    pub option_chain: Vec<OptionExpiry>,
}

/// Option contracts grouped by expiry date.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionExpiry {
    /// Expiry date as an ISO 8601 string (e.g. `"2026-03-21T00:00:00.000000-05:00"`).
    pub expiry_date: String,
    /// Human-readable description for this expiry.
    pub description: String,
    /// Exchange where these options are listed.
    pub listing_exchange: String,
    /// Exercise style: `"American"` or `"European"`.
    pub option_exercise_type: String,
    /// Option chains grouped by root symbol (usually one per expiry).
    pub chain_per_root: Vec<ChainPerRoot>,
}

/// Option contracts for a single root symbol within an expiry.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainPerRoot {
    /// Option root symbol (usually matches the underlying ticker).
    pub option_root: String,
    /// Contract multiplier (typically 100 for equity options).
    pub multiplier: Option<u32>,
    /// Strike-level call/put symbol ID pairs.
    pub chain_per_strike_price: Vec<ChainPerStrike>,
}

/// Call and put symbol IDs at a single strike price.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainPerStrike {
    /// Strike price.
    pub strike_price: f64,
    /// Questrade symbol ID for the call option at this strike.
    pub call_symbol_id: u64,
    /// Questrade symbol ID for the put option at this strike.
    pub put_symbol_id: u64,
}

// ---------- Option quotes ----------

/// Request body for `POST /v1/markets/quotes/options`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionQuoteRequest {
    /// Option symbol IDs to fetch quotes for (max 100 per request).
    pub option_ids: Vec<u64>,
}

/// Response from `POST /v1/markets/quotes/options`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionQuoteResponse {
    /// Quotes for the requested option symbol IDs.
    pub option_quotes: Vec<OptionQuote>,
}

/// Real-time quote and Greeks for a single option contract.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionQuote {
    /// Underlying ticker symbol (e.g. `"AAPL"`).
    pub underlying: String,
    /// Questrade symbol ID of the underlying equity.
    pub underlying_id: u64,
    /// Option ticker symbol.
    pub symbol: String,
    /// Questrade symbol ID for this option contract.
    pub symbol_id: u64,
    /// Best bid price; `None` if no bid is available.
    pub bid_price: Option<f64>,
    /// Best ask price; `None` if no ask is available.
    pub ask_price: Option<f64>,
    /// Last trade price.
    pub last_trade_price: Option<f64>,
    /// Total session volume in contracts.
    pub volume: Option<u64>,
    /// Open interest — number of outstanding contracts.
    pub open_interest: Option<u64>,
    /// Implied volatility as a decimal (e.g. `0.30` = 30%).
    pub volatility: Option<f64>,
    /// Delta Greek (rate of change of option price vs. underlying price).
    pub delta: Option<f64>,
    /// Gamma Greek (rate of change of delta vs. underlying price).
    pub gamma: Option<f64>,
    /// Theta Greek — daily time decay of the option price.
    pub theta: Option<f64>,
    /// Vega Greek (sensitivity to implied volatility changes).
    pub vega: Option<f64>,
    /// Rho Greek (sensitivity to interest rate changes).
    pub rho: Option<f64>,
    /// Strike price. May be absent; prefer values derived from the chain structure.
    pub strike_price: Option<f64>,
    /// Expiry date string. May be absent; prefer values derived from the chain structure.
    pub expiry_date: Option<String>,
    /// Option type: `"Call"` or `"Put"`. May be absent.
    pub option_type: Option<String>,
    /// Volume-weighted average price.
    #[serde(rename = "VWAP")]
    pub vwap: Option<f64>,
    /// Whether trading in this option is currently halted.
    pub is_halted: Option<bool>,
    /// Number of contracts at the best bid.
    pub bid_size: Option<u64>,
    /// Number of contracts at the best ask.
    pub ask_size: Option<u64>,
}

// ---------- Strategy quotes ----------

/// A single leg in a multi-leg option strategy variant.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyLeg {
    /// Option symbol ID for this leg.
    pub symbol_id: u64,
    /// Order side: `"Buy"` or `"Sell"`.
    pub action: String,
    /// Ratio of this leg in the strategy (e.g. 1 for a standard spread).
    pub ratio: u32,
}

/// A strategy variant to quote, containing one or more legs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyVariantRequest {
    /// Caller-assigned ID, echoed in the response for matching.
    pub variant_id: u32,
    /// Strategy type (e.g. `"Custom"`).
    pub strategy: String,
    /// Legs comprising this strategy variant.
    pub legs: Vec<StrategyLeg>,
}

/// Request body for `POST /v1/markets/quotes/strategies`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyQuoteRequest {
    /// Strategy variants to fetch combined quotes for.
    pub variants: Vec<StrategyVariantRequest>,
}

/// Response from `POST /v1/markets/quotes/strategies`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyQuotesResponse {
    /// Combined quotes for each requested strategy variant.
    pub strategy_quotes: Vec<StrategyQuote>,
}

/// Combined quote and Greeks for a multi-leg option strategy.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyQuote {
    /// Echoed variant ID from the request.
    pub variant_id: u32,
    /// Best bid price for the strategy; `None` if unavailable.
    pub bid_price: Option<f64>,
    /// Best ask price for the strategy; `None` if unavailable.
    pub ask_price: Option<f64>,
    /// Underlying ticker symbol.
    pub underlying: String,
    /// Questrade internal symbol ID of the underlying.
    pub underlying_id: u64,
    /// Session open price for the strategy.
    pub open_price: Option<f64>,
    /// Implied volatility as a decimal (e.g. `0.30` = 30%).
    pub volatility: Option<f64>,
    /// Delta Greek.
    pub delta: Option<f64>,
    /// Gamma Greek.
    pub gamma: Option<f64>,
    /// Theta Greek — daily time decay.
    pub theta: Option<f64>,
    /// Vega Greek — sensitivity to IV changes.
    pub vega: Option<f64>,
    /// Rho Greek — sensitivity to interest rate changes.
    pub rho: Option<f64>,
    /// Whether the quote data is real-time.
    pub is_real_time: bool,
}

// ---------- Accounts ----------

/// Response wrapper for `GET /v1/accounts`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsResponse {
    /// Brokerage accounts associated with the authenticated user.
    pub accounts: Vec<Account>,
}

/// A Questrade brokerage account.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Account type: `"TFSA"`, `"RRSP"`, `"Margin"`, etc.
    #[serde(rename = "type")]
    pub account_type: String,
    /// Account number string (used as `account_id` in subsequent API calls).
    pub number: String,
    /// Account status: `"Active"`, `"Closed"`, etc.
    pub status: String,
    /// Whether this is the user's primary account.
    #[serde(default)]
    pub is_primary: bool,
}

// ---------- Positions ----------

/// Response wrapper for `GET /v1/accounts/:id/positions`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionsResponse {
    /// Current open positions in the account.
    pub positions: Vec<PositionItem>,
}

/// A single open position in a Questrade account.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionItem {
    /// Ticker symbol.
    pub symbol: String,
    /// Questrade internal symbol ID.
    pub symbol_id: u64,
    /// Number of shares or contracts currently held. Questrade may return
    /// `null` for this field; it is deserialized as `0.0` in that case.
    #[serde(deserialize_with = "null_as_zero")]
    pub open_quantity: f64,
    /// Current market value of the position.
    pub current_market_value: Option<f64>,
    /// Current market price per share or contract.
    pub current_price: Option<f64>,
    /// Average cost basis per share or contract. Deserializes `null` as `0.0`.
    #[serde(deserialize_with = "null_as_zero")]
    pub average_entry_price: f64,
    /// Realized P&L on closed portions of the position.
    pub closed_pnl: Option<f64>,
    /// Unrealized P&L on the remaining open position.
    pub open_pnl: Option<f64>,
    /// Total cost basis for the position. Deserializes `null` as `0.0`.
    #[serde(deserialize_with = "null_as_zero")]
    pub total_cost: f64,
}

// ---------- Account activities ----------

/// Response wrapper for `GET /v1/accounts/:id/activities`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitiesResponse {
    /// Activity items for the requested date range.
    pub activities: Vec<ActivityItem>,
}

/// A single account activity (execution, dividend, deposit, etc.).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    /// ISO datetime of the trade (e.g. `"2024-10-23T00:00:00.000000-04:00"`).
    pub trade_date: String,
    /// Date the transaction was recorded (may differ from `trade_date`).
    #[serde(default)]
    pub transaction_date: Option<String>,
    /// T+1/T+2 settlement date.
    #[serde(default)]
    pub settlement_date: Option<String>,
    /// Human-readable description (e.g. `"SELL 1 AAPL Jan 17 '25 $200 Put"`).
    #[serde(default)]
    pub description: Option<String>,
    /// Action type: `"Buy"`, `"Sell"`, `"SellShort"`, `"BuyToCover"`, etc.
    pub action: String,
    /// Ticker symbol for this activity.
    pub symbol: String,
    /// Questrade internal symbol ID.
    pub symbol_id: u64,
    /// Number of shares or contracts involved in the activity.
    pub quantity: f64,
    /// Execution price per share or contract.
    pub price: f64,
    /// Gross amount before commission. Zero if not provided.
    #[serde(default)]
    pub gross_amount: f64,
    /// Commission charged (typically negative). Zero if not applicable.
    #[serde(default)]
    pub commission: f64,
    /// Net cash impact on the account (gross amount + commission).
    pub net_amount: f64,
    /// Settlement currency (e.g. `"CAD"`, `"USD"`). `None` if not provided.
    #[serde(default)]
    pub currency: Option<String>,
    /// Activity category: `"Trades"`, `"Dividends"`, `"Deposits"`, etc.
    #[serde(rename = "type")]
    pub activity_type: String,
}

// ---------- Balances ----------

/// Current and start-of-day balances for a Questrade account.
///
/// Returned by `GET /v1/accounts/:id/balances`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountBalances {
    /// Real-time balances broken down by currency.
    pub per_currency_balances: Vec<PerCurrencyBalance>,
    /// Real-time balances combined across currencies (expressed in CAD).
    pub combined_balances: Vec<CombinedBalance>,
    /// Start-of-day balances broken down by currency.
    pub sod_per_currency_balances: Vec<PerCurrencyBalance>,
    /// Start-of-day balances combined across currencies (expressed in CAD).
    pub sod_combined_balances: Vec<CombinedBalance>,
}

/// Balance snapshot for a single currency.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerCurrencyBalance {
    /// Currency code (`"CAD"` or `"USD"`).
    pub currency: String,
    /// Cash balance in this currency.
    pub cash: f64,
    /// Total market value of securities denominated in this currency.
    pub market_value: f64,
    /// Total account equity in this currency (cash + market value).
    pub total_equity: f64,
    /// Available buying power.
    pub buying_power: f64,
    /// Maintenance excess (equity above the margin maintenance requirement).
    pub maintenance_excess: f64,
    /// Whether the values are real-time (`true`) or delayed (`false`).
    pub is_real_time: bool,
}

/// Combined balance across all currencies, expressed in a single currency.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombinedBalance {
    /// Currency in which combined values are expressed (typically `"CAD"`).
    pub currency: String,
    /// Combined cash balance.
    pub cash: f64,
    /// Combined market value of all securities.
    pub market_value: f64,
    /// Combined total equity (cash + market value).
    pub total_equity: f64,
    /// Combined available buying power.
    pub buying_power: f64,
    /// Combined maintenance excess.
    pub maintenance_excess: f64,
    /// Whether the values are real-time (`true`) or delayed (`false`).
    pub is_real_time: bool,
}

// ---------- Markets ----------

/// Response wrapper for `GET /v1/markets`.
#[derive(Debug, Deserialize)]
pub struct MarketsResponse {
    /// Metadata for each market / exchange.
    pub markets: Vec<MarketInfo>,
}

/// Metadata for a single market / exchange returned by `GET /v1/markets`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketInfo {
    /// Market or exchange name (e.g. `"NYSE"`, `"TSX"`).
    pub name: String,
    /// Trading currency (e.g. `"USD"`, `"CAD"`).
    #[serde(default)]
    pub currency: Option<String>,
    /// Regular session open time (ISO 8601).
    #[serde(default)]
    pub start_time: Option<String>,
    /// Regular session close time (ISO 8601).
    #[serde(default)]
    pub end_time: Option<String>,
    /// Extended (pre-market) session open time (ISO 8601).
    #[serde(default)]
    pub extended_start_time: Option<String>,
    /// Extended (after-hours) session close time (ISO 8601).
    #[serde(default)]
    pub extended_end_time: Option<String>,
    /// Current open/closed status snapshot; `None` if not available.
    #[serde(default)]
    pub snapshot: Option<MarketSnapshot>,
}

/// Real-time open/closed status for a market.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSnapshot {
    /// Whether the market is currently open for regular trading.
    pub is_open: bool,
    /// Quote delay in minutes (`0` = real-time).
    #[serde(default)]
    pub delay: u32,
}

// ---------- Symbol detail ----------

/// Response wrapper for `GET /v1/symbols/:id`.
#[derive(Debug, Deserialize)]
pub struct SymbolDetailResponse {
    /// Full details for the requested symbol (typically one entry).
    pub symbols: Vec<SymbolDetail>,
}

/// Full symbol details returned by `GET /v1/symbols/:id`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolDetail {
    /// Ticker symbol (e.g. `"AAPL"`).
    pub symbol: String,
    /// Questrade internal symbol ID.
    pub symbol_id: u64,
    /// Human-readable company or security name.
    pub description: String,
    /// Security type: `"Stock"`, `"Option"`, `"ETF"`, etc.
    pub security_type: String,
    /// Primary listing exchange.
    pub listing_exchange: String,
    /// Trading currency (e.g. `"USD"`, `"CAD"`).
    pub currency: String,
    /// Whether the security is tradable through Questrade.
    pub is_tradable: bool,
    /// Whether real-time quotes are available.
    pub is_quotable: bool,
    /// Whether listed options exist for this security.
    pub has_options: bool,
    /// Previous trading day's closing price.
    pub prev_day_close_price: Option<f64>,
    /// 52-week high price.
    pub high_price52: Option<f64>,
    /// 52-week low price.
    pub low_price52: Option<f64>,
    /// 3-month average daily volume in shares.
    pub average_vol3_months: Option<u64>,
    /// 20-day average daily volume in shares.
    pub average_vol20_days: Option<u64>,
    /// Total shares outstanding.
    pub outstanding_shares: Option<u64>,
    /// Trailing twelve-month earnings per share.
    pub eps: Option<f64>,
    /// Price-to-earnings ratio.
    pub pe: Option<f64>,
    /// Annual dividend per share.
    pub dividend: Option<f64>,
    /// Annual dividend yield as a percentage (e.g. `0.53` = 0.53%).
    #[serde(rename = "yield")]
    pub dividend_yield: Option<f64>,
    /// Most recent ex-dividend date (ISO 8601).
    pub ex_date: Option<String>,
    /// Most recent dividend payment date (ISO 8601).
    pub dividend_date: Option<String>,
    /// Market capitalisation.
    pub market_cap: Option<f64>,
    /// GICS sector name (e.g. `"Technology"`).
    pub industry_sector: Option<String>,
    /// GICS industry group name.
    pub industry_group: Option<String>,
    /// GICS sub-industry name.
    pub industry_sub_group: Option<String>,
    /// For option symbols: `"Call"` or `"Put"`.
    pub option_type: Option<String>,
    /// For option symbols: expiry date (ISO 8601).
    pub option_expiry: Option<String>,
    /// For option symbols: strike price.
    pub option_strike_price: Option<f64>,
    /// For option symbols: exercise style — `"American"` or `"European"`.
    pub option_exercise_type: Option<String>,
}

// ---------- Orders ----------

/// Filter for order state when querying `GET /v1/accounts/:id/orders`.
#[derive(Debug, Clone, Copy, Serialize)]
pub enum OrderStateFilter {
    /// Return all orders regardless of state.
    All,
    /// Return only open (working) orders.
    Open,
    /// Return only closed (filled, canceled, expired) orders.
    Closed,
}

impl std::fmt::Display for OrderStateFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => write!(f, "All"),
            Self::Open => write!(f, "Open"),
            Self::Closed => write!(f, "Closed"),
        }
    }
}

/// Response wrapper for `GET /v1/accounts/:id/orders`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrdersResponse {
    /// Orders matching the query filters.
    pub orders: Vec<OrderItem>,
}

/// A single order from the Questrade orders endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderItem {
    /// Internal order identifier.
    pub id: u64,
    /// Ticker symbol (e.g. `"AAPL"`).
    pub symbol: String,
    /// Questrade internal symbol ID.
    pub symbol_id: u64,
    /// Total quantity of the order.
    #[serde(default)]
    pub total_quantity: f64,
    /// Quantity still open (unfilled).
    #[serde(default)]
    pub open_quantity: f64,
    /// Quantity that has been filled.
    #[serde(default)]
    pub filled_quantity: f64,
    /// Quantity that was canceled.
    #[serde(default)]
    pub canceled_quantity: f64,
    /// Order side: `"Buy"`, `"Sell"`, `"BuyToOpen"`, `"SellToClose"`, etc.
    pub side: String,
    /// Order type: `"Market"`, `"Limit"`, `"Stop"`, `"StopLimit"`, etc.
    pub order_type: String,
    /// Limit price, if applicable.
    #[serde(default)]
    pub limit_price: Option<f64>,
    /// Stop price, if applicable.
    #[serde(default)]
    pub stop_price: Option<f64>,
    /// Average execution price across all fills.
    #[serde(default)]
    pub avg_exec_price: Option<f64>,
    /// Price of the last execution.
    #[serde(default)]
    pub last_exec_price: Option<f64>,
    /// Commission charged for the order.
    #[serde(default)]
    pub commission_charged: f64,
    /// Current order state (e.g. `"Executed"`, `"Canceled"`, `"Pending"`, etc.).
    pub state: String,
    /// Time in force: `"Day"`, `"GoodTillCanceled"`, `"GoodTillDate"`, etc.
    pub time_in_force: String,
    /// ISO 8601 datetime when the order was created.
    pub creation_time: String,
    /// ISO 8601 datetime of the last update to the order.
    pub update_time: String,
    /// Questrade staff annotations.
    #[serde(default)]
    pub notes: Option<String>,
    /// Whether the order is all-or-none.
    #[serde(default)]
    pub is_all_or_none: bool,
    /// Whether the order is anonymous.
    #[serde(default)]
    pub is_anonymous: bool,
    /// Order group ID for bracket orders.
    #[serde(default)]
    pub order_group_id: Option<u64>,
    /// Chain ID linking related orders.
    #[serde(default)]
    pub chain_id: Option<u64>,
}

// ---------- Executions ----------

/// Response wrapper for `GET /v1/accounts/:id/executions`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionsResponse {
    /// Execution records for the requested date range.
    pub executions: Vec<Execution>,
}

/// A single trade execution (fill-level detail) from Questrade.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    /// Ticker symbol (e.g. `"AAPL"`).
    pub symbol: String,
    /// Questrade internal symbol ID.
    pub symbol_id: u64,
    /// Number of shares or contracts filled.
    pub quantity: f64,
    /// Client side of the order: `"Buy"`, `"Sell"`, etc.
    pub side: String,
    /// Execution price per share or contract.
    pub price: f64,
    /// Internal execution identifier.
    pub id: u64,
    /// Internal order identifier.
    pub order_id: u64,
    /// Internal order chain identifier.
    pub order_chain_id: u64,
    /// Identifier of the execution at the originating exchange.
    #[serde(default)]
    pub exchange_exec_id: Option<String>,
    /// Execution timestamp (ISO 8601).
    pub timestamp: String,
    /// Manual notes from Trade Desk staff (empty string if none).
    #[serde(default)]
    pub notes: Option<String>,
    /// Trading venue where the execution originated (e.g. `"LAMP"`).
    #[serde(default)]
    pub venue: Option<String>,
    /// Total cost: price × quantity.
    #[serde(default, deserialize_with = "null_as_zero")]
    pub total_cost: f64,
    /// Trade Desk order placement commission.
    #[serde(default, deserialize_with = "null_as_zero")]
    pub order_placement_commission: f64,
    /// Questrade commission.
    #[serde(default, deserialize_with = "null_as_zero")]
    pub commission: f64,
    /// Venue liquidity execution fee.
    #[serde(default, deserialize_with = "null_as_zero")]
    pub execution_fee: f64,
    /// SEC fee on US security sales.
    #[serde(default, deserialize_with = "null_as_zero")]
    pub sec_fee: f64,
    /// TSX/Canadian execution fee.
    #[serde(default, deserialize_with = "null_as_zero")]
    pub canadian_execution_fee: f64,
    /// Parent order identifier (0 if none).
    #[serde(default)]
    pub parent_id: u64,
}

// ---------- Candles ----------

/// Response wrapper for `GET /v1/markets/candles/:id`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandleResponse {
    /// OHLCV price bars in chronological order.
    pub candles: Vec<Candle>,
}

/// A single OHLCV price bar.
#[derive(Debug, Deserialize)]
pub struct Candle {
    /// Bar open time (ISO 8601).
    pub start: String,
    /// Bar close time (ISO 8601).
    pub end: String,
    /// Open price.
    pub open: f64,
    /// High price.
    pub high: f64,
    /// Low price.
    pub low: f64,
    /// Closing price.
    pub close: f64,
    /// Volume traded during the bar.
    pub volume: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markets_response_deserializes_from_questrade_json() {
        // Representative response from GET /v1/markets (real Questrade format).
        let json = r#"{
            "markets": [
                {
                    "name": "NYSE",
                    "tradingVenues": ["NYSE"],
                    "defaultTradingVenue": "NYSE",
                    "primaryOrderRoutes": ["NYSE"],
                    "secondaryOrderRoutes": [],
                    "level1Feeds": ["NYSE"],
                    "level2Feeds": [],
                    "extendedStartTime": "2026-02-21T08:00:00.000000-05:00",
                    "startTime": "2026-02-21T09:30:00.000000-05:00",
                    "endTime": "2026-02-21T16:00:00.000000-05:00",
                    "extendedEndTime": "2026-02-21T20:00:00.000000-05:00",
                    "currency": "USD",
                    "snapshot": { "isOpen": true, "delay": 0 }
                },
                {
                    "name": "TSX",
                    "tradingVenues": ["TSX"],
                    "defaultTradingVenue": "TSX",
                    "primaryOrderRoutes": ["TSX"],
                    "secondaryOrderRoutes": [],
                    "level1Feeds": ["TSX"],
                    "level2Feeds": [],
                    "extendedStartTime": "2026-02-21T08:00:00.000000-05:00",
                    "startTime": "2026-02-21T09:30:00.000000-05:00",
                    "endTime": "2026-02-21T16:00:00.000000-05:00",
                    "extendedEndTime": "2026-02-21T17:00:00.000000-05:00",
                    "currency": "CAD",
                    "snapshot": null
                }
            ]
        }"#;

        let resp: MarketsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.markets.len(), 2);

        let nyse = &resp.markets[0];
        assert_eq!(nyse.name, "NYSE");
        assert_eq!(nyse.currency.as_deref(), Some("USD"));
        assert_eq!(
            nyse.start_time.as_deref(),
            Some("2026-02-21T09:30:00.000000-05:00")
        );
        assert_eq!(
            nyse.end_time.as_deref(),
            Some("2026-02-21T16:00:00.000000-05:00")
        );
        let snap = nyse.snapshot.as_ref().unwrap();
        assert!(snap.is_open);
        assert_eq!(snap.delay, 0);

        // TSX has snapshot: null — should deserialise to None.
        let tsx = &resp.markets[1];
        assert_eq!(tsx.name, "TSX");
        assert!(tsx.snapshot.is_none());
    }

    #[test]
    fn account_balances_deserializes_from_questrade_json() {
        let json = r#"{
            "perCurrencyBalances": [
                {
                    "currency": "CAD",
                    "cash": 10000.0,
                    "marketValue": 50000.0,
                    "totalEquity": 60000.0,
                    "buyingPower": 60000.0,
                    "maintenanceExcess": 60000.0,
                    "isRealTime": false
                }
            ],
            "combinedBalances": [
                {
                    "currency": "CAD",
                    "cash": 10000.0,
                    "marketValue": 50000.0,
                    "totalEquity": 60000.0,
                    "buyingPower": 60000.0,
                    "maintenanceExcess": 60000.0,
                    "isRealTime": false
                }
            ],
            "sodPerCurrencyBalances": [
                {
                    "currency": "CAD",
                    "cash": 9000.0,
                    "marketValue": 49000.0,
                    "totalEquity": 58000.0,
                    "buyingPower": 58000.0,
                    "maintenanceExcess": 58000.0,
                    "isRealTime": false
                }
            ],
            "sodCombinedBalances": [
                {
                    "currency": "CAD",
                    "cash": 9000.0,
                    "marketValue": 49000.0,
                    "totalEquity": 58000.0,
                    "buyingPower": 58000.0,
                    "maintenanceExcess": 58000.0,
                    "isRealTime": false
                }
            ]
        }"#;

        let balances: AccountBalances = serde_json::from_str(json).unwrap();
        assert_eq!(balances.per_currency_balances.len(), 1);
        let cad = &balances.per_currency_balances[0];
        assert_eq!(cad.currency, "CAD");
        assert_eq!(cad.cash, 10000.0);
        assert_eq!(cad.market_value, 50000.0);
        assert_eq!(cad.total_equity, 60000.0);
        assert!(!cad.is_real_time);
        assert_eq!(balances.combined_balances.len(), 1);
        assert_eq!(balances.sod_per_currency_balances.len(), 1);
        assert_eq!(balances.sod_combined_balances.len(), 1);
    }

    #[test]
    fn symbol_detail_deserializes_from_questrade_json() {
        let json = r#"{
            "symbols": [
                {
                    "symbol": "AAPL",
                    "symbolId": 8049,
                    "description": "Apple Inc.",
                    "securityType": "Stock",
                    "listingExchange": "NASDAQ",
                    "currency": "USD",
                    "isTradable": true,
                    "isQuotable": true,
                    "hasOptions": true,
                    "prevDayClosePrice": 182.50,
                    "highPrice52": 199.62,
                    "lowPrice52": 124.17,
                    "averageVol3Months": 52000000,
                    "averageVol20Days": 50000000,
                    "outstandingShares": 15700000000,
                    "eps": 6.14,
                    "pe": 29.74,
                    "dividend": 0.96,
                    "yield": 0.53,
                    "exDate": "2023-11-10T00:00:00.000000-05:00",
                    "dividendDate": "2023-11-16T00:00:00.000000-05:00",
                    "marketCap": 2866625000000.0,
                    "industrySector": "Technology",
                    "industryGroup": "Technology Hardware, Storage & Peripherals",
                    "industrySubGroup": "Other",
                    "optionType": null,
                    "optionExpiry": null,
                    "optionStrikePrice": null,
                    "optionExerciseType": null
                }
            ]
        }"#;

        let resp: SymbolDetailResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.symbols.len(), 1);
        let s = &resp.symbols[0];
        assert_eq!(s.symbol, "AAPL");
        assert_eq!(s.symbol_id, 8049);
        assert_eq!(s.description, "Apple Inc.");
        assert_eq!(s.security_type, "Stock");
        assert_eq!(s.listing_exchange, "NASDAQ");
        assert_eq!(s.currency, "USD");
        assert!(s.is_tradable);
        assert!(s.is_quotable);
        assert!(s.has_options);
        assert_eq!(s.prev_day_close_price, Some(182.50));
        assert_eq!(s.high_price52, Some(199.62));
        assert_eq!(s.low_price52, Some(124.17));
        assert_eq!(s.eps, Some(6.14));
        assert_eq!(s.dividend_yield, Some(0.53));
        assert_eq!(s.industry_sector.as_deref(), Some("Technology"));
        assert!(s.option_type.is_none());
        assert!(s.option_expiry.is_none());
    }

    #[test]
    fn orders_response_deserializes_from_questrade_json() {
        let json = r#"{
            "orders": [
                {
                    "id": 173577870,
                    "symbol": "AAPL",
                    "symbolId": 8049,
                    "totalQuantity": 100,
                    "openQuantity": 0,
                    "filledQuantity": 100,
                    "canceledQuantity": 0,
                    "side": "Buy",
                    "orderType": "Limit",
                    "limitPrice": 150.50,
                    "stopPrice": null,
                    "avgExecPrice": 150.25,
                    "lastExecPrice": 150.25,
                    "commissionCharged": 4.95,
                    "state": "Executed",
                    "timeInForce": "Day",
                    "creationTime": "2026-02-20T10:30:00.000000-05:00",
                    "updateTime": "2026-02-20T10:31:15.000000-05:00",
                    "notes": null,
                    "isAllOrNone": false,
                    "isAnonymous": false,
                    "orderGroupId": 0,
                    "chainId": 173577870
                },
                {
                    "id": 173600001,
                    "symbol": "MSFT",
                    "symbolId": 9291,
                    "totalQuantity": 50,
                    "openQuantity": 50,
                    "filledQuantity": 0,
                    "canceledQuantity": 0,
                    "side": "Buy",
                    "orderType": "Limit",
                    "limitPrice": 400.00,
                    "stopPrice": null,
                    "avgExecPrice": null,
                    "lastExecPrice": null,
                    "commissionCharged": 0,
                    "state": "Pending",
                    "timeInForce": "GoodTillCanceled",
                    "creationTime": "2026-02-21T09:45:00.000000-05:00",
                    "updateTime": "2026-02-21T09:45:00.000000-05:00",
                    "notes": "Staff note here",
                    "isAllOrNone": true,
                    "isAnonymous": false
                }
            ]
        }"#;

        let resp: OrdersResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.orders.len(), 2);

        // Fully filled order
        let o1 = &resp.orders[0];
        assert_eq!(o1.id, 173577870);
        assert_eq!(o1.symbol, "AAPL");
        assert_eq!(o1.symbol_id, 8049);
        assert_eq!(o1.total_quantity, 100.0);
        assert_eq!(o1.open_quantity, 0.0);
        assert_eq!(o1.filled_quantity, 100.0);
        assert_eq!(o1.canceled_quantity, 0.0);
        assert_eq!(o1.side, "Buy");
        assert_eq!(o1.order_type, "Limit");
        assert_eq!(o1.limit_price, Some(150.50));
        assert!(o1.stop_price.is_none());
        assert_eq!(o1.avg_exec_price, Some(150.25));
        assert_eq!(o1.last_exec_price, Some(150.25));
        assert_eq!(o1.commission_charged, 4.95);
        assert_eq!(o1.state, "Executed");
        assert_eq!(o1.time_in_force, "Day");
        assert!(o1.notes.is_none());
        assert!(!o1.is_all_or_none);
        assert_eq!(o1.chain_id, Some(173577870));

        // Pending order with optional fields missing
        let o2 = &resp.orders[1];
        assert_eq!(o2.id, 173600001);
        assert_eq!(o2.symbol, "MSFT");
        assert_eq!(o2.state, "Pending");
        assert_eq!(o2.time_in_force, "GoodTillCanceled");
        assert!(o2.avg_exec_price.is_none());
        assert!(o2.last_exec_price.is_none());
        assert_eq!(o2.commission_charged, 0.0);
        assert_eq!(o2.notes.as_deref(), Some("Staff note here"));
        assert!(o2.is_all_or_none);
        // Missing orderGroupId/chainId should default to None
        assert!(o2.order_group_id.is_none());
        assert!(o2.chain_id.is_none());
    }

    #[test]
    fn execution_deserializes_from_questrade_json() {
        let json = r#"{
            "executions": [
                {
                    "symbol": "AAPL",
                    "symbolId": 8049,
                    "quantity": 10,
                    "side": "Buy",
                    "price": 536.87,
                    "id": 53817310,
                    "orderId": 177106005,
                    "orderChainId": 17710600,
                    "exchangeExecId": "XS1771060050147",
                    "timestamp": "2014-03-31T13:38:29.000000-04:00",
                    "notes": "",
                    "venue": "LAMP",
                    "totalCost": 5368.7,
                    "orderPlacementCommission": 0,
                    "commission": 4.95,
                    "executionFee": 0,
                    "secFee": 0,
                    "canadianExecutionFee": 0,
                    "parentId": 0
                }
            ]
        }"#;

        let resp: ExecutionsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.executions.len(), 1);

        let e = &resp.executions[0];
        assert_eq!(e.symbol, "AAPL");
        assert_eq!(e.symbol_id, 8049);
        assert_eq!(e.quantity, 10.0);
        assert_eq!(e.side, "Buy");
        assert_eq!(e.price, 536.87);
        assert_eq!(e.id, 53817310);
        assert_eq!(e.order_id, 177106005);
        assert_eq!(e.order_chain_id, 17710600);
        assert_eq!(e.exchange_exec_id.as_deref(), Some("XS1771060050147"));
        assert_eq!(e.timestamp, "2014-03-31T13:38:29.000000-04:00");
        assert_eq!(e.venue.as_deref(), Some("LAMP"));
        assert_eq!(e.total_cost, 5368.7);
        assert_eq!(e.commission, 4.95);
        assert_eq!(e.execution_fee, 0.0);
        assert_eq!(e.sec_fee, 0.0);
        assert_eq!(e.parent_id, 0);
    }

    #[test]
    fn strategy_quotes_response_deserializes_from_questrade_json() {
        let json = r#"{
            "strategyQuotes": [
                {
                    "variantId": 1,
                    "bidPrice": 27.2,
                    "askPrice": 27.23,
                    "underlying": "MSFT",
                    "underlyingId": 9291,
                    "openPrice": 27.0,
                    "volatility": 0.30,
                    "delta": 1.0,
                    "gamma": 0.0,
                    "theta": -0.05,
                    "vega": 0.01,
                    "rho": 0.002,
                    "isRealTime": true
                }
            ]
        }"#;

        let resp: StrategyQuotesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.strategy_quotes.len(), 1);

        let q = &resp.strategy_quotes[0];
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

    #[test]
    fn strategy_quote_request_serializes_to_questrade_json() {
        let req = StrategyQuoteRequest {
            variants: vec![StrategyVariantRequest {
                variant_id: 1,
                strategy: "Custom".to_string(),
                legs: vec![
                    StrategyLeg {
                        symbol_id: 27426,
                        action: "Buy".to_string(),
                        ratio: 1000,
                    },
                    StrategyLeg {
                        symbol_id: 10550014,
                        action: "Sell".to_string(),
                        ratio: 10,
                    },
                ],
            }],
        };

        let json = serde_json::to_value(&req).unwrap();
        let variants = json["variants"].as_array().unwrap();
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0]["variantId"], 1);
        assert_eq!(variants[0]["strategy"], "Custom");
        let legs = variants[0]["legs"].as_array().unwrap();
        assert_eq!(legs.len(), 2);
        assert_eq!(legs[0]["symbolId"], 27426);
        assert_eq!(legs[0]["action"], "Buy");
        assert_eq!(legs[0]["ratio"], 1000);
        assert_eq!(legs[1]["symbolId"], 10550014);
        assert_eq!(legs[1]["action"], "Sell");
        assert_eq!(legs[1]["ratio"], 10);
    }
}
