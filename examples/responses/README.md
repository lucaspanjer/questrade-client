# Questrade API — Real Response Examples

These JSON files are anonymized real responses captured from the Questrade REST API using the `dump_responses` example tool:

```bash
cargo run -p questrade-client --example dump_responses -- --refresh-token <TOKEN> --endpoint markets
```

## Captured Endpoints

| File | Endpoint | Captured |
|------|----------|----------|
| `time.json` | `GET /v1/time` | 2026-03-03 |
| `markets.json` | `GET /v1/markets` | 2026-03-03 |
| `accounts.json` | `GET /v1/accounts` | 2026-03-03 |

## Discrepancies: Public Docs vs Actual API

### `GET /v1/markets`

The [public docs](https://www.questrade.com/api/documentation/rest-operations/market-calls/markets) list these fields per market:

- `name`, `tradingVenues`, `defaultTradingVenue`, `primaryOrderRoutes`, `secondaryOrderRoutes`, `level1Feeds`, `level2Feeds`, `extendedStartTime`, `startTime`, `endTime`, `extendedEndTime`, `currency`, `snapQuotesLimit`

**Missing from actual response:**
- `currency` — not present in any market object
