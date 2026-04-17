# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Shell completion generation for bash, zsh, fish (`poly completions <shell>`)
- Aggregate portfolio P&L summary on the Positions tab (value, cost, return %)
- Market description display in TUI detail view (collapsible, 20% cap)
- TUI unit tests: 38 tests covering state transitions, filters, order form validation, fill detection
- Order fill notifications: flash message + terminal bell when orders are filled
- crates.io metadata in Cargo.toml (license, repository, keywords, categories)

### Changed
- Improved `--help` text with logical command grouping and quick-start guide
- buy/sell `--help` now shows formatted examples for limit and market orders

### Fixed
- Clipboard copy now works cross-platform (Linux: xclip/xsel/wl-copy, Windows: clip.exe)
- Neg-risk (weather) market detail from Positions tab no longer shows wrong market data
- Description panel capped at 20% height when collapsed (was 25%)

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

[Unreleased]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.2.0...HEAD
[0.2.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/compare/v0.1.0...v0.2.0
[0.1.0]: https://gitlab.com/maxweilbuchner/poly-v2/-/releases/tag/v0.1.0
