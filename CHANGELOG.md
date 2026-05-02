# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.3] - 2026-05-02

### Added
- Weather-market intelligence: extractor for location (`display`, `country`, ICAO) from question + Wunderground/NOAA description prose; lookup table covering all 51 ICAO stations currently referenced by active Polymarket temperature markets (lat/lon/elevation/IANA tz); generator script `scripts/gen_airports.py` to refresh the table from gamma + the OurAirports public-domain CSV.
- Open-Meteo ECMWF-IFS025 ensemble forecast (51 members) plus best_match deterministic anchor for weather markets resolving in D+0..D+2; rendered inline below the question on the market detail screen as `mean / p10 / p90 / 7-cell histogram / anchor` for both Tmax and Tmin. 30-min in-memory + on-disk cache (`forecast.json`, versioned).
- ICAO + airport-local time (`h:mm am/pm`) shown next to the question on the Markets list and the market detail header for every weather market.
- New `local time` sort mode for the Markets tab (cycled with `s`): weather markets ordered ascending by current minute-of-day at their resolution station; non-weather markets fall to the bottom by volume desc.
- One-shot warning flash + `tracing::warn!` when a weather market is opened with an ICAO not in the airport lookup table (prompts re-running the generator).
- `9h` option in the markets date filter cycle (between `6h` and `12h`).

### Changed
- Migrated to Polymarket CLOB V2 (cutover 2026-04-28). EIP-712 domain bumps to version `"2"` with new exchange addresses (`0xE111...996B` standard, `0xe222...0F59` neg-risk); the signed Order struct drops `nonce`/`feeRateBps`/`taker`/`expiration` and gains `timestamp`/`metadata`/`builder`. Collateral switches from USDC.e to pUSD (`0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB`, 6 decimals, 1:1 USDC-backed). Fees are now protocol-set at match time; `/fee-rate` is no longer fetched before signing. `cancel-all` moves from `DELETE /orders` to `POST /cancel-all`. `POLY_CLOB_URL` env var added to override the CLOB base URL (e.g., for staging tests).
- Removed ambiguous colored status dots from market lists.
- Moved market filters into a dedicated sub-header row for improved scannability.

### Fixed
- Balance chart now ignores $0 data points to prevent misleading massive drops in net worth.
- Added an example address hint to the Viewer tab empty state.

## [0.4.2] - 2026-04-27

### Changed
- `setup::validate_private_key` now verifies the key is a valid secp256k1 scalar (rejects all-zeros and values ≥ curve order), not just 64 hex chars. Closes #111.

## [0.4.1] - 2026-04-27

### Added
- Analytics tab gains a "Most Accurate Recurring Series" panel: ranks Polymarket recurring series (by `group_slug`) by directional prediction accuracy at the configured calibration horizon (3 / 6 / 9 / 12 h, cycled with `t`). Series need ≥5 resolved binary markets to qualify; the top 10 are shown.
- One-shot `group_slug` backfill on TUI startup re-fetches the series identifier from Gamma for resolutions stored before the column existed (idempotent, capped at 1000 rows per run).
- `db::query_recurring_accuracy` plus 3 integration tests covering ordering by accuracy, the empty-group_slug filter, and the snapshot-fallback path.

### Changed
- Analytics layout switches from 2×2 to a 3-row grid (40 / 40 / 20%) to make room for the new panel; the existing four panels keep their positions.
- `resolutions` table grows a `group_slug TEXT NOT NULL DEFAULT ''` column; `MarketResolution` and `ResolutionRow` carry it through. Migration is automatic on next launch.

## [0.4.0] - 2026-04-26

### Added
- Integration tests for the WebSocket order-book and user channels (5 tests): protocol framing, event emission, HTTP fallback on connect failure, disconnect handling. Tests run against a local in-process WS server.
- End-to-end tests for the `poly setup` wizard core logic (20 tests): input validators (private key, Ethereum address, URL), config-file write/read round-trip, Unix file permissions (0600), `[tui]` section preservation, env-var precedence in `has_config`.
- `setup::validate_private_key`, `setup::validate_eth_address`, `setup::validate_url` — pure validators extracted from the wizard's input loops.
- `tasks::spawn_ws_order_book_at_url` and `tasks::spawn_ws_user_channel_at_url` — URL-parameterised variants of the WS spawners (the existing public functions still work; new variants exist for tests and future flexibility).

### Changed
- Crate renamed to `poly-cli` for crates.io publication (the binary remains `poly`). Install with `cargo install poly-cli`.
- Analytics tab loads markedly faster: preliminary stats render from cached DB state before any network round-trips, the six SQLite queries in `compute_analytics_stats` now run in parallel, and a new `market_peak_vol` table makes peak-volume lookups O(1) per row instead of O(snapshots).

## [0.3.3] - 2026-04-22

### Added
- `poly doctor` command — runs a configuration health check (config file + permissions, wallet key, CLOB credentials, API auth probe, USDC balance, CTF allowance) and exits non-zero if anything is broken
- Order entry `m` key fills the size field with the maximum placeable size (cash balance / effective price for buys, held shares for sells)
- `SECURITY.md` documenting threat model, what's stored/logged, best practices, and private vulnerability reporting
- ASCII screenshot in `README.md` and a `docs/demo.tape` [vhs](https://github.com/charmbracelet/vhs) script for recording real demo GIFs

## [0.3.2] - 2026-04-21

### Added
- Viewer tab (`5`): browse any Polymarket wallet's portfolio by address — no auth required
- Positions tab UI overhaul: two-line list items with question title, outcome, shares, price, value, and P&L columns

### Changed
- Balance tab "Summary" panel redesigned: compact 6-column layout (Cash, Allowance, Positions, Shares, Net Worth, Max Payout) with annotations row, replaces the tall 3-section text layout
- Summary panel title now shows position count, share count, and low-allowance warning inline

## [0.3.0] - 2026-04-19

### Added
- Shell completion generation for bash, zsh, fish (`poly completions <shell>`)
- Aggregate portfolio P&L summary on the Positions tab (value, cost, return %)
- Market description display in TUI detail view (collapsible, 20% cap)
- Net worth time-series chart on the Balance tab (logs to SQLite every 10 min, Braille line chart)
- Complete help screen (`?`) — now covers all tabs including Balance and Analytics keybindings
- TUI unit tests: 38 tests covering state transitions, filters, order form validation, fill detection
- Order fill notifications: flash message + terminal bell when orders are filled
- crates.io metadata in Cargo.toml (license, repository, keywords, categories)
- User WebSocket channel for live order/trade events — instant fill notifications without REST polling
- "WS" indicator in status bar when the user WebSocket channel is connected
- All tabs preload data at startup (positions, orders, balance, analytics run in parallel)
- MIT license
- Integration test suites: HMAC auth (5 tests), persist round-trip (10 tests)
- CI smoke test (`--version` check after build)

### Changed
- Improved `--help` text with logical command grouping and quick-start guide
- buy/sell `--help` now shows formatted examples for limit and market orders
- REST position polling interval extended to 4× when user WS is connected (safety net only)
- Analytics tab caches results — no longer recomputes on every tab switch
- TUI internals split into focused modules (state, events, keys, tasks)
- Resolution Bias chart bars dynamically fill panel width
- README updated with full architecture diagram, shell completions docs
- Balance tab redesigned with wallet/portfolio/totals sections

### Fixed
- Clipboard copy now works cross-platform (Linux: xclip/xsel/wl-copy, Windows: clip.exe)
- Neg-risk (weather) market detail from Positions tab no longer shows wrong market data
- Description panel capped at 20% height when collapsed (was 25%)
- Replaced bare `.unwrap()` calls in client with `.expect()` for clearer panic messages

## [0.2.0] - 2026-04-16

### Added
- Interactive TUI dashboard with tabs for Markets, Positions, Balance, Analytics
- Analytics dashboard: probability distribution, calibration, resolution stats
- SQLite persistence layer for market snapshots and resolution tracking
- Interactive setup wizard (`poly setup`) for credential configuration
- EIP-712 order signing with proxy wallet support
- Price history sparklines in market detail view
- WebSocket order book streaming with HTTP fallback
- Watchlist with per-market starring (persisted across sessions)
- Market sort modes (volume, end date, probability) and filters (date, prob, volume, category)
- Order management: place, cancel, cancel-all, cancel-by-market
- Position close/redeem workflows with confirmation screens
- Export to CSV (`poly export positions|orders`)
- `poly top` command with category filtering
- `poly watch` for live order book monitoring
- `poly history` for filled order history
- `poly derive-keys` for CLOB credential derivation
- `--json` flag for machine-readable output on all commands
- `--dry-run` flag for order validation without submission
- `--log-file` structured logging via tracing
- Config file support (`~/.config/poly/config.toml`)
- Security warning for overly permissive config file permissions
- API request concurrency limiting via semaphore
- Client retry logic with exponential backoff

### Changed
- Enhanced error messages with auth, network, and API error categories

## [0.1.0] - 2026-04-15

### Added
- Initial release
- `poly search` — keyword market search via Gamma API
- `poly market` — market detail with outcomes and order book
- `poly book` — standalone order book viewer
- `poly buy` / `poly sell` — limit order placement with EIP-712 signing
- `poly orders` — list open orders
- `poly positions` — list current positions
- `poly cancel` / `poly cancel-all` — order cancellation
- `poly balance` — on-chain USDC balance and CTF allowance check
- HMAC-SHA256 authentication for CLOB API
- Colored terminal output

[Unreleased]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.4.3...HEAD
[0.4.3]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.4.2...v0.4.3
[0.4.2]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.4.1...v0.4.2
[0.4.1]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.4.0...v0.4.1
[0.4.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.3.3...v0.4.0
[0.3.3]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.3.2...v0.3.3
[0.3.2]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.3.0...v0.3.2
[0.3.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.2.0...v0.3.0
[0.2.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.1.0...v0.2.0
[0.1.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/releases/tag/v0.1.0
