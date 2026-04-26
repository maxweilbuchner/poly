# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.4.0...HEAD
[0.4.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.3.3...v0.4.0
[0.3.3]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.3.2...v0.3.3
[0.3.2]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.3.0...v0.3.2
[0.3.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.2.0...v0.3.0
[0.2.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.1.0...v0.2.0
[0.1.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/releases/tag/v0.1.0
