# Security Policy

## Reporting a vulnerability

If you find a security issue in `poly`, please report it privately.

- **Email:** maxweilbuchner@gmail.com
- **GitLab:** file a confidential issue at
  <https://gitlab.com/maxweilbuchner/poly-v2/-/issues> (mark it "Confidential"
  when opening — the checkbox is in the issue form).

Please include steps to reproduce, the version affected (`poly --version`),
and your OS/terminal. I'll acknowledge within 7 days and try to ship a fix
within 30 days depending on severity.

Do **not** open a public issue or PR for anything that could let someone
drain a wallet, sign unauthorized orders, or impersonate a user.

## Threat model

`poly` is a local CLI/TUI. It is not a hosted service. The threat model is
therefore scoped to what happens on the user's own machine and in transit
to Polymarket's APIs.

### In scope

- **Key handling:** protecting the EOA private key and CLOB API credentials
  from accidental exposure (logs, error messages, stack traces, `--help`
  output, terminal scrollback).
- **Order signing:** ensuring EIP-712 order payloads and HMAC-SHA256 auth
  headers are constructed correctly, so a malformed signature cannot be
  replayed or reinterpreted by the CLOB.
- **Input validation:** rejecting malformed market IDs, token IDs, sizes,
  and prices before they are sent to the API or signed.
- **Config file permissions:** warning when `config.toml` is readable by
  other users on shared systems (we chmod 600 on write, and warn at
  startup if the mode has been widened).

### Out of scope

- Compromise of the user's machine or OS. If an attacker already has shell
  access, they can read the config file and keyring regardless of what
  `poly` does.
- Compromise of Polymarket's APIs, the Polygon network, or the underlying
  CTF Exchange contract.
- Phishing or social engineering that tricks a user into pasting a
  malicious private key into `poly setup`.
- Supply-chain attacks on upstream crates. `Cargo.lock` is committed so
  builds are reproducible, but we do not currently run `cargo audit` in CI
  (tracking issue welcome).

## What is stored, where, and how

| Data | Location | Protection |
|---|---|---|
| Private key (EOA) | `~/.config/poly/config.toml` or `~/.poly/config.toml` | File is chmod 600 on creation; startup warns if group/other bits are set. Plain TOML — no OS keyring yet (tracked for a future release). |
| CLOB API key / secret / passphrase | Same config file | Same as above. |
| Funder / proxy wallet address | Same config file | Not sensitive (public). |
| Polygon RPC URL | Same config file | Treated as sensitive because it usually contains an Alchemy/Infura API key. |
| Watchlist, UI state, snapshots | `$XDG_DATA_HOME/poly/` | Not secret. SQLite DB + JSON files, world-readable by default. |
| Logs (`--log-file`) | `$XDG_DATA_HOME/poly/poly.log` | Tracing output is filtered to never include raw private keys or HMAC secrets; request bodies may include wallet addresses and order payloads. |

Environment variables (`POLY_PRIVATE_KEY`, `POLY_API_KEY`, ...) and `.env`
files take precedence over the config file if present. If you use a `.env`
file, make sure it is `chmod 600` and excluded from version control — the
repo's `.gitignore` already excludes `.env`.

## What is logged

- **CLI output** never prints the private key, API secret, or passphrase.
  `poly derive-keys` prints the derived credentials once to stdout so you
  can paste them into `.env` or the config file — redirect or pipe with
  care.
- **`--log-file`** / `RUST_LOG` produces structured tracing output. It
  includes HTTP method + path + status codes, wallet addresses, and order
  payloads (size, price, token ID). It **does not** include the EIP-712
  signature preimage, raw HMAC secret, or the private key. If you find a
  log line that leaks a secret, that's a bug — please report it.

## Best practices for users

- Run `poly doctor` after setup to confirm your config file permissions,
  credential validity, and on-chain allowance before placing real orders.
- Keep `config.toml` at `chmod 600` (Unix) or restrict to your user only
  (Windows). `poly` warns at startup if this isn't the case.
- Use a dedicated trading wallet, not your main EOA. The CLOB API key can
  be rotated with `poly derive-keys`, but the underlying private key cannot.
- If you trade through a proxy / Gnosis Safe, set `POLY_FUNDER_ADDRESS` so
  orders are signed with `signatureType=1` (proxy) rather than EOA.
- Treat the Polygon RPC URL as sensitive — Alchemy/Infura keys in URLs are
  credentials too. Don't paste `config.toml` into chat or screenshots.

## Roadmap

The following improvements are tracked as open work:

- OS keyring integration (`keyring` crate) for the private key and CLOB
  secret, replacing plaintext TOML storage.
- `cargo audit` + `cargo deny` in CI.
- Reproducible release builds with signed checksums.
