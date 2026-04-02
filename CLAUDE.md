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
└── display.rs    All terminal output (tables, colors via `colored`)
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

## Adding Commands

1. Add a variant to the `Command` enum in `main.rs`
2. Add a `cmd_*` async handler function in `main.rs`
3. Add the match arm in the `match cli.command` block
4. Add API methods to `PolyClient` in `client.rs` as needed
5. Add display helpers to `display.rs`
