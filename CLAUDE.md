# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
cargo build --release
cargo run -- <command>          # dev build
./target/release/poly <command> # release build

# Example commands
poly search "Trump" --limit 10
poly market will-trump-win-the-2024-us-presidential-election
poly market 0x<condition-id> --book
poly book <token-id> --label Yes
poly buy <token-id> 0.65 10
poly sell <token-id> 0.90 10 --order-type FOK
poly orders
poly positions
poly cancel <order-id>
poly cancel-all
poly balance
poly buy <token-id> 0.65 10 --dry-run
```

## Architecture

Single binary (`poly`) with clap subcommands. All Polymarket API interaction goes through `PolyClient` in `client.rs`.

```
src/
├── main.rs       Command dispatch, client construction, input validation
├── client.rs     PolyClient — Gamma API + CLOB API + EIP-712 order signing
├── auth.rs       HMAC-SHA256 signing for CLOB REST headers
├── types.rs      Shared data types (Market, Outcome, Order, Position, etc.)
├── display.rs    CLI terminal output (tables, colors)
├── db.rs         SQLite persistence (snapshots, resolutions, calibration)
├── persist.rs    JSON state persistence (UI state, watchlist, snapshot meta)
├── setup.rs      Interactive setup wizard (CLI + TUI shared logic)
├── error.rs      Typed error handling with actionable messages
├── tui/
│   ├── mod.rs        Event loop entry point, tests
│   ├── state.rs      App state, enums, AppEvent
│   ├── events.rs     Event dispatch (AppEvent → state mutations)
│   ├── keys.rs       Key bindings and input handling
│   ├── tasks.rs      Background task spawners (API, WS, analytics)
│   ├── ui.rs         Top-level layout, modal overlays
│   ├── theme.rs      Color constants
│   ├── screens/      Tab content (markets, positions, balance, analytics, setup, etc.)
│   └── widgets/      Reusable components (order book, status bar, tab bar)
└── lib.rs
```

## API Layers

**Gamma API** (`https://gamma-api.polymarket.com`) — public, no auth:
- `GET /markets?search=<q>&active=true&limit=N` — keyword search
- `GET /markets/<condition_id>` — market by condition ID
- `GET /events?slug=<slug>` — event by slug (contains nested markets)
- `GET /markets?market_slug=<slug>` — market by slug directly

**CLOB API** (`https://clob.polymarket.com`) — public reads, auth for writes:
- `GET /book?token_id=<id>` — order book (public)
- `GET /fee-rate?token_id=<id>` — fee rate (public)
- `POST /order` — place order (EIP-712 signed + HMAC auth headers)
- `DELETE /order` — cancel order (HMAC auth headers)
- `DELETE /orders` — cancel all (HMAC auth headers)
- `GET /data/orders?status=live` — open orders (HMAC auth headers)
- `GET /data/order/<id>` — single order status (HMAC auth headers)
- `GET /data/positions?user=<address>` — positions (HMAC auth headers)

## Authentication

Two independent auth mechanisms:

**HMAC-SHA256** (for CLOB REST, `auth.rs`):
- message = `timestamp + METHOD + /path + body`
- signature = `url_safe_base64(hmac_sha256(url_safe_base64_decode(secret), message))`
- headers: `POLY_ADDRESS`, `POLY_API_KEY`, `POLY_PASSPHRASE`, `POLY_TIMESTAMP`, `POLY_SIGNATURE`

**EIP-712** (for order signing, `client.rs::place_order`):
- Domain: name="Polymarket CTF Exchange", version="1", chainId=137, verifyingContract=CTF_EXCHANGE
- Type: `Order(uint256 salt, address maker, ...)` — exact camelCase field names required
- `maker` = funder/proxy address (or signer if no proxy), `signer` = EOA private key address
- `signatureType` = 0 (EOA) or 1 (proxy wallet)
- `owner` in POST body = CLOB API key UUID (not wallet address)
- Amounts are in USDC micro-units (6 decimals): `size_scaled = size * 1_000_000`

## Key Constraints

- Minimum order: 5 shares, $1.00 USDC total cost
- Price range: 0.01 – 0.99 (inclusive)
- Size precision: 2 decimal places; cost precision: 4 decimal places
- Token IDs are large decimal integers (from `clobTokenIds` in Gamma API response)
- USDC on Polygon has 6 decimals (not 18)

## Environment Variables

Copy `.env.example` to `.env`. Required for trading:
- `POLY_PRIVATE_KEY` — EOA private key (signs EIP-712 orders)
- `POLY_API_KEY`, `POLY_API_SECRET`, `POLY_API_PASSPHRASE` — CLOB credentials
- `POLYGON_RPC_URL` — Alchemy/Infura endpoint for on-chain balance checks
- `POLY_FUNDER_ADDRESS` — optional proxy/funder wallet address

Search and order book commands work without credentials.

## Workflow

- After completing a task, always commit and push to GitLab (`git push origin main`).
- Run `cargo fmt` before committing to avoid CI failures.
- CI runs `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings -A clippy::uninlined_format_args`, and `cargo test --all-targets`.

## Releasing

When bumping the version for a release:

1. Bump `version` in `Cargo.toml`, run `cargo update -p poly` to sync `Cargo.lock`
2. Move `[Unreleased]` entries in `CHANGELOG.md` into a dated `[x.y.z]` section, add the compare link at the bottom
3. Commit, push, then **create and push the git tag**:
   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
4. The tag push triggers the CI release pipeline (build Linux + macOS binaries, create GitLab release)

**Never skip the git tag.** Without it the CI release pipeline does not run and there is no downloadable binary.

## Keeping Docs in Sync

When adding or changing user-visible features, update these files **in the same commit**:

- **`README.md`** — tabs table, key bindings section, CLI commands list. If you add a tab, add it to the tabs table AND add a keybindings subsection. If you change a tab's name or purpose, update its description.
- **`CHANGELOG.md`** — add entries under `[Unreleased]`. Use Added/Changed/Fixed/Removed sections per [Keep a Changelog](https://keepachangelog.com). When bumping the version, move `[Unreleased]` entries into a dated `[x.y.z]` section and add the compare link at the bottom.
- **`CLAUDE.md`** — update the `Tab` enum line, architecture tree, or conventions if the change affects project structure or patterns.
- **`Cargo.toml`** — bump version when releasing. Keep `Cargo.lock` in sync (`cargo update -p poly`).

## TUI Architecture

The TUI uses a message-passing event loop. No subcommand → launches TUI (`tui::run()`).

**Event flow:** Background tasks → `AppEvent` via `mpsc::UnboundedSender` → `handle_event()` in `events.rs` → state mutation on `App` → re-render.

**Key types in `state.rs`:**
- `App` — all TUI state (markets, positions, balance, filters, UI state)
- `Tab` — `Markets | Positions | Balance | Analytics | Viewer`
- `Screen` — `MarketList | MarketDetail | OrderEntry | Help | QuitConfirm | Setup | ...`
- `AppEvent` — all events (data loads, user actions, errors, WebSocket updates)

**Screen stack:** `app.screen_stack: Vec<Screen>` allows pushing modal/detail screens on top. Tab switches clear the stack. `app.current_screen()` returns the topmost.

**Input handling in `keys.rs`:** `handle_key()` dispatches on screen first (modals checked before tabs), returns `bool` (true = quit).

### Adding a TUI Screen/Tab

1. Create `src/tui/screens/my_screen.rs` with `pub fn render(f: &mut Frame, area: Rect, app: &mut App)`
2. Add `mod my_screen;` in `src/tui/screens/mod.rs`
3. Add variant to `Screen` enum in `state.rs`
4. Wire rendering in `ui.rs` (match on screen/tab → call render)
5. Add key handling in `keys.rs`
6. If tab: add variant to `Tab` enum, update `Tab::next()`/`Tab::prev()`, update tab bar widget

### Adding a Background Task

1. Add `AppEvent` variant(s) in `state.rs` for the result
2. Add `spawn_*()` function in `tasks.rs`: takes `Arc<PolyClient>`, `UnboundedSender<AppEvent>`, spawns `tokio::spawn`, sends result as `AppEvent`, errors as `AppEvent::Error()`
3. Add match arm in `handle_event()` in `events.rs` to apply the result to `App` state
4. Trigger the spawn from a key handler or the `Tick` event (for periodic tasks)

## Error Handling

`AppError` in `error.rs` has four variants:

| Variant | Use |
|---------|-----|
| `Auth(String)` | Missing/invalid credentials — shown persistently in status bar |
| `Network(String)` | Timeout, DNS, connection errors |
| `Api { status, message }` | Non-2xx HTTP responses — message parsed from JSON |
| `Other(Box<dyn Error>)` | Catch-all via `AppError::other(e)` |

Auth errors display actionable hints (e.g., "run `poly setup`"). In the TUI, auth errors persist in the status bar; other errors flash for a few seconds.

## Database (`db.rs`)

SQLite at `$XDG_DATA_HOME/poly/poly.db`. Tables: `snapshots`, `resolutions`, `net_worth_log`.

**Adding a table:**
1. Add `CREATE TABLE IF NOT EXISTS ...` to the `SCHEMA` constant
2. Write `insert_*()` using batch transaction pattern: `conn.transaction()` → `prepare_cached()` → loop `execute()` → `commit()`
3. Write `query_*()` returning `Vec<T>`
4. Use `INSERT OR IGNORE` for idempotent inserts

## Persistence (`persist.rs`)

JSON files in `$XDG_DATA_HOME/poly/`:
- `ui_state.json` — sort/filter/category preferences (survives restarts)
- `snapshot_meta.json` — last snapshot timestamp + count
- `watchlist.json` — starred condition IDs (versioned format)

Uses versioned save/load: a `"version"` field is injected into the JSON. Unknown newer versions fall back to defaults (forward-compatible).

## Testing

**Unit tests:** In-source `#[cfg(test)] mod tests` blocks. Use `test_app()` helper (in `tui/mod.rs`) for `App` with defaults. Use `test_market(id, question, volume)` builder for test data.

**Integration tests** (`tests/`):
- `client_tests.rs` — WireMock HTTP mocking with JSON fixtures from `tests/fixtures/`
- `auth_tests.rs` — HMAC-SHA256 signature golden values
- `signing_tests.rs` — EIP-712 digest computation
- `db_tests.rs` — SQLite round-trips with `tempfile`
- `persist_tests.rs` — JSON serialization + version migration
- `cli_integration_tests.rs` — command parsing (no network)

**Test client:** `PolyClient::new_test(gamma_url, clob_url, data_url)` accepts custom base URLs for WireMock.

## Adding CLI Commands

1. Add a variant to the `Command` enum in `main.rs`
2. Add a `cmd_*` async handler function in `main.rs`
3. Add the match arm in the `match cli.command` block
4. Add API methods to `PolyClient` in `client.rs` as needed
5. Add display helpers to `display.rs`

## Conventions

- **Naming:** `spawn_*` for background tasks, `render` for screen drawing, `handle_*` for input/event processing
- **Screen render signature:** `pub fn render(f: &mut Frame, area: Rect, app: &mut App)`
- **Filter/sort enums:** implement `.next()` (cycling) and `.label()` (display string)
- **Theme:** color constants in `tui/theme.rs` — use these, don't hardcode colors
- **Imports:** `use super::*` for pulling parent module types; `crate::` paths for cross-module
- **Flash messages:** `(String, Instant, bool)` tuple — (text, shown_at, is_error); expiry checked on Tick
