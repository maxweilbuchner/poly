mod auth;
mod client;
mod db;
mod display;
mod error;
mod persist;
mod setup;
mod tui;
mod types;

use clap::{Parser, Subcommand};
use client::PolyClient;
use error::AppError;
use std::env;
use types::{OrderType, PlaceOrderParams, Side};

struct TradeArgs {
    token_id:   String,
    price:      Option<f64>,
    size:       f64,
    side:       Side,
    order_type: String,
    market:     bool,
    expiry:     Option<u64>,
    dry_run:    bool,
    json:       bool,
}

#[derive(Parser)]
#[command(
    name = "poly",
    about = "Universal Polymarket CLI trading tool",
    version,
    long_about = None
)]
struct Cli {
    /// Dry-run mode: build and validate the order but do not submit it
    #[arg(long, global = true)]
    dry_run: bool,

    /// Output result as JSON instead of formatted text
    #[arg(long, global = true)]
    json: bool,

    /// Write structured logs to a file (default: $XDG_DATA_HOME/poly/poly.log).
    /// Set RUST_LOG to control level (e.g. RUST_LOG=debug).
    #[arg(long, global = true)]
    log_file: Option<Option<String>>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Search markets by keyword
    Search {
        /// Search query (e.g. "Trump", "Bitcoin", "World Cup")
        query: String,

        /// Maximum number of results to show
        #[arg(short, long, default_value_t = 20)]
        limit: usize,

        /// Include closed markets in results
        #[arg(long)]
        all: bool,
    },

    /// Show detailed info and order book for a market
    ///
    /// Accepts a market slug (e.g. "will-trump-win-2024") or a condition ID (hex).
    Market {
        /// Market slug or condition ID
        id: String,

        /// Show the full CLOB order book for each outcome
        #[arg(long)]
        book: bool,
    },

    /// Show the order book for a specific outcome token
    Book {
        /// CLOB token ID (decimal integer string from `poly market`)
        token_id: String,

        /// Label to display (e.g. "Yes", "No")
        #[arg(long, default_value = "")]
        label: String,
    },

    /// Place a limit buy order
    ///
    /// Limit:  poly buy <token-id> <size> <price>
    ///   e.g.  poly buy 12345...789 10 0.65   — 10 shares at $0.65
    /// Market: poly buy <token-id> <size> --market
    ///   e.g.  poly buy 12345...789 10 --market — 10 shares at best ask (FOK)
    Buy {
        /// CLOB token ID (from `poly market`)
        token_id: String,

        /// Number of shares (min 5, min $1 total)
        size: f64,

        /// Limit price in USD (0.01 – 0.99); omit when using --market
        price: Option<f64>,

        /// Order type: GTC (default), FOK, IOC; ignored when --market is used
        #[arg(long, default_value = "GTC")]
        order_type: String,

        /// Buy at the best available ask price; submits as FOK
        #[arg(long)]
        market: bool,

        /// Order expiry as a Unix timestamp (seconds); omit for no expiry
        #[arg(long)]
        expiry: Option<u64>,
    },

    /// Place a limit sell order
    ///
    /// Limit:  poly sell <token-id> <size> <price>
    /// Market: poly sell <token-id> <size> --market   — sells at best bid (FOK)
    Sell {
        /// CLOB token ID (from `poly market` or `poly positions`)
        token_id: String,

        /// Number of shares to sell
        size: f64,

        /// Limit price in USD (0.01 – 0.99); omit when using --market
        price: Option<f64>,

        /// Order type: GTC (default), FOK, IOC; ignored when --market is used
        #[arg(long, default_value = "GTC")]
        order_type: String,

        /// Sell at the best available bid price; submits as FOK
        #[arg(long)]
        market: bool,

        /// Order expiry as a Unix timestamp (seconds); omit for no expiry
        #[arg(long)]
        expiry: Option<u64>,
    },

    /// List your open orders
    Orders,

    /// List your current positions
    Positions,

    /// Cancel a specific order
    Cancel {
        /// Order ID (from `poly orders`)
        order_id: String,
    },

    /// Cancel all open orders
    CancelAll,

    /// Show on-chain USDC balance and CTF allowance
    Balance,

    /// List top markets by trading volume
    Top {
        /// Maximum number of results to show
        #[arg(short, long, default_value_t = 20)]
        limit: usize,

        /// Filter by category (e.g. "politics", "crypto", "sports")
        #[arg(long)]
        category: Option<String>,
    },

    /// Show filled order history
    History {
        /// Maximum number of past orders to show (most recent first)
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },

    /// Watch an order book, refreshing in-place until Ctrl+C
    Watch {
        /// CLOB token ID (from `poly market`)
        token_id: String,

        /// Label to display (e.g. "Yes", "No")
        #[arg(long, default_value = "")]
        label: String,

        /// Refresh interval in seconds
        #[arg(short, long, default_value_t = 2)]
        interval: u64,
    },

    /// Cancel all open orders for a specific market
    ///
    /// Looks up the market's outcome token IDs and cancels any open orders
    /// whose asset matches. Accepts a condition ID (hex).
    CancelMarket {
        /// Market condition ID (hex, with or without 0x prefix)
        condition_id: String,
    },

    /// Export positions or orders to CSV (stdout or file)
    Export {
        /// What to export: "positions" or "orders"
        what: String,

        /// Write to a file instead of stdout
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Open the interactive TUI dashboard (default when no subcommand is given)
    Tui,

    /// Derive CLOB API credentials from your private key
    ///
    /// Signs a ClobAuth EIP-712 message with POLY_MARKET_KEY and calls
    /// /auth/api-key on the CLOB. Prints the credentials to add to .env.
    DeriveKeys,

    /// Import existing CSV snapshot and resolution data into the SQLite database.
    ///
    /// This is a one-time migration. It is also run automatically on the first
    /// TUI launch, so you only need this command if you want to migrate before
    /// opening the TUI.
    Migrate,

    /// Interactive setup wizard — configure credentials for trading
    ///
    /// Walks you through setting up your private key, CLOB API credentials,
    /// and optional RPC URL. Can auto-derive CLOB keys from your wallet.
    Setup,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    // ── Structured logging ───────────────────────────────────────────────────
    // Activated by `--log-file` (with optional path) or `RUST_LOG` env var.
    // File defaults to $XDG_DATA_HOME/poly/poly.log.
    let _log_guard = init_logging(&cli);

    let command = cli.command.unwrap_or(Command::Tui);

    // `poly setup` runs before building the client (which needs no credentials).
    if matches!(command, Command::Setup) {
        if let Err(e) = setup::run().await {
            display::print_error(&e.to_string());
            std::process::exit(1);
        }
        return;
    }

    // Suggest setup when no config exists and command needs credentials
    if !setup::has_config() && needs_auth(&command) {
        display::print_error(
            "No configuration found. Run `poly setup` to configure your credentials.",
        );
        std::process::exit(1);
    }

    let client = build_client();

    let json = cli.json;
    let result = match command {
        Command::Tui => tui::run(client.clone(), build_tui_config()).await,
        Command::Search { query, limit, all } => {
            cmd_search(&client, &query, limit, !all, json).await
        }
        Command::Market { id, book } => cmd_market(&client, &id, book, json).await,
        Command::Book { token_id, label } => cmd_book(&client, &token_id, &label, json).await,
        Command::Buy { token_id, size, price, order_type, market, expiry } => {
            cmd_trade(&client, TradeArgs { token_id, price, size, side: Side::Buy, order_type, market, expiry, dry_run: cli.dry_run, json }).await
        }
        Command::Sell { token_id, size, price, order_type, market, expiry } => {
            cmd_trade(&client, TradeArgs { token_id, price, size, side: Side::Sell, order_type, market, expiry, dry_run: cli.dry_run, json }).await
        }
        Command::Orders => cmd_orders(&client, json).await,
        Command::Positions => cmd_positions(&client, json).await,
        Command::Cancel { order_id } => cmd_cancel(&client, &order_id, json).await,
        Command::CancelAll => cmd_cancel_all(&client, json).await,
        Command::Balance => cmd_balance(&client, json).await,
        Command::History { limit } => cmd_history(&client, limit, json).await,
        Command::Watch { token_id, label, interval } => {
            cmd_watch(&client, &token_id, &label, interval).await
        }
        Command::Top { limit, category } => cmd_top(&client, limit, category.as_deref(), json).await,
        Command::CancelMarket { condition_id } => {
            cmd_cancel_market(&client, &condition_id, json).await
        }
        Command::Export { what, output } => {
            cmd_export(&client, &what, output.as_deref()).await
        }
        Command::DeriveKeys => cmd_derive_keys(&client).await,
        Command::Migrate => cmd_migrate().await,
        Command::Setup => unreachable!(),
    };

    if let Err(e) = result {
        display::print_error(&e.to_string());
        std::process::exit(1);
    }
}

// ── Command handlers ──────────────────────────────────────────────────────────

async fn cmd_search(
    client: &PolyClient,
    query: &str,
    limit: usize,
    active_only: bool,
    json: bool,
) -> client::Result<()> {
    if !json {
        display::print_info(&format!("Searching for \"{}\"…", query));
    }
    let markets = client.search_markets(query, active_only, limit).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&markets).map_err(AppError::other)?);
    } else {
        display::print_market_list(&markets);
    }
    Ok(())
}

async fn cmd_market(client: &PolyClient, id: &str, show_book: bool, json: bool) -> client::Result<()> {
    // Accept full polymarket.com URLs, e.g.
    //   https://polymarket.com/event/will-trump-win-the-2024-us-presidential-election
    // Strip query string, fragment, and trailing slash, then take the last path segment.
    let slug_buf: String;
    let id: &str = if id.starts_with("http") {
        slug_buf = id
            .split(['?', '#'])
            .next()
            .unwrap_or(id)
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(id)
            .to_string();
        &slug_buf
    } else {
        id
    };

    if !json {
        display::print_info(&format!("Fetching market \"{}\"…", id));
    }

    // Try condition ID (64-char hex) first, then slug
    let markets = if id.len() == 66 || (id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()))
    {
        // Looks like a condition ID
        let cid = id.strip_prefix("0x").unwrap_or(id);
        let full_id = format!("0x{}", cid);
        match client.get_market_by_id(&full_id).await? {
            Some(m) => vec![m],
            None => vec![],
        }
    } else {
        client.get_market_by_slug(id).await?
    };

    if markets.is_empty() {
        display::print_error("Market not found. Try `poly search <keyword>` to find the right slug or ID.");
        return Ok(());
    }

    if json {
        if show_book {
            let mut out = Vec::new();
            for market in &markets {
                let mut books = Vec::new();
                for outcome in &market.outcomes {
                    if outcome.token_id.is_empty() {
                        continue;
                    }
                    let book = client.get_order_book(&outcome.token_id).await?;
                    books.push(serde_json::json!({
                        "outcome": outcome.name,
                        "book": book,
                    }));
                }
                out.push(serde_json::json!({ "market": market, "books": books }));
            }
            println!("{}", serde_json::to_string_pretty(&out).map_err(AppError::other)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&markets).map_err(AppError::other)?);
        }
    } else {
        for market in &markets {
            display::print_market_detail(market);

            if show_book {
                for outcome in &market.outcomes {
                    if outcome.token_id.is_empty() {
                        continue;
                    }
                    let book = client.get_order_book(&outcome.token_id).await?;
                    display::print_order_book(&book, &outcome.name);
                }
            }
        }
    }

    Ok(())
}

async fn cmd_book(client: &PolyClient, token_id: &str, label: &str, json: bool) -> client::Result<()> {
    let book = client.get_order_book(token_id).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&book).map_err(AppError::other)?);
    } else {
        let name = if label.is_empty() { token_id } else { label };
        display::print_order_book(&book, name);
    }
    Ok(())
}

async fn cmd_trade(client: &PolyClient, args: TradeArgs) -> client::Result<()> {
    let TradeArgs { token_id, price: price_arg, size, side, order_type: order_type_str, market, expiry, dry_run, json } = args;
    // Resolve price and order type: market orders fetch best price from the book
    // and always use FOK; limit orders require an explicit price.
    let (price, order_type) = if market {
        let book = client.get_order_book(&token_id).await?;
        let best = match &side {
            Side::Buy => book.asks.first().map(|l| l.price),
            Side::Sell => book.bids.first().map(|l| l.price),
        };
        match best {
            Some(p) => (p, OrderType::Fok),
            None => return Err("Order book is empty — cannot place market order".into()),
        }
    } else {
        match price_arg {
            Some(p) => (p, parse_order_type(&order_type_str)?),
            None => return Err("Price is required unless --market is used".into()),
        }
    };

    // Validate expiry is in the future when provided
    if let Some(exp) = expiry {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if exp <= now {
            return Err("--expiry timestamp must be in the future".into());
        }
    }

    // Validate price and size
    if price <= 0.0 || price >= 1.0 {
        return Err("Price must be between 0.01 and 0.99".into());
    }
    if size < 5.0 {
        return Err("Minimum order size is 5 shares".into());
    }
    if size * price < 1.0 {
        return Err("Minimum order value is $1.00 USDC".into());
    }

    if dry_run {
        let cost = size * price;
        if json {
            println!("{}", serde_json::json!({
                "dry_run": true,
                "side": side.to_string(),
                "token_id": token_id,
                "price": price,
                "size": size,
                "cost": cost,
                "expiry": expiry,
            }));
        } else {
            let expiry_note = expiry
                .map(|e| format!("  expiry: {}", e))
                .unwrap_or_default();
            display::print_info(&format!(
                "DRY RUN — {} {} shares of {} @ {:.4} (cost: ${:.4}){}",
                side, size, token_id, price, cost, expiry_note
            ));
        }
        return Ok(());
    }

    let neg_risk = client.get_neg_risk(&token_id).await;
    let params = PlaceOrderParams { token_id, price, size, side: side.clone(), order_type, expiry, neg_risk };
    let order_id = client.place_order(&params).await?;
    if json {
        println!("{}", serde_json::json!({ "order_id": order_id }));
    } else {
        display::print_order_placed(&order_id, &side, &params.token_id, price, size);
    }
    Ok(())
}

async fn cmd_orders(client: &PolyClient, json: bool) -> client::Result<()> {
    if !json {
        display::print_info("Fetching open orders…");
    }
    let orders = client.get_open_orders().await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&orders).map_err(AppError::other)?);
    } else {
        display::print_orders(&orders);
    }
    Ok(())
}

async fn cmd_positions(client: &PolyClient, json: bool) -> client::Result<()> {
    if !json {
        display::print_info("Fetching positions…");
    }
    let positions = client.get_positions().await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&positions).map_err(AppError::other)?);
    } else {
        display::print_positions(&positions);
    }
    Ok(())
}

async fn cmd_export(client: &PolyClient, what: &str, output: Option<&str>) -> client::Result<()> {
    let csv = match what.to_lowercase().as_str() {
        "positions" | "pos" | "p" => {
            display::print_info("Fetching positions…");
            let positions = client.get_positions().await?;
            let mut buf = String::from(
                "market_id,question,outcome,token_id,size,avg_price,current_price,realized_pnl,unrealized_pnl\n",
            );
            for p in &positions {
                buf.push_str(&format!(
                    "{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6}\n",
                    csv_escape(&p.market_id),
                    csv_escape(&p.market_question),
                    csv_escape(&p.outcome),
                    csv_escape(&p.token_id),
                    p.size,
                    p.avg_price,
                    p.current_price,
                    p.realized_pnl,
                    p.unrealized_pnl,
                ));
            }
            buf
        }
        "orders" | "ord" | "o" => {
            display::print_info("Fetching orders…");
            let orders = client.get_open_orders().await?;
            let mut buf = String::from(
                "id,side,outcome,market,price,original_size,size_matched,status,created_at\n",
            );
            for o in &orders {
                buf.push_str(&format!(
                    "{},{},{},{},{:.6},{:.6},{:.6},{},{}\n",
                    csv_escape(&o.id),
                    o.side,
                    csv_escape(&o.outcome),
                    csv_escape(&o.market),
                    o.price,
                    o.original_size,
                    o.size_matched,
                    o.status,
                    csv_escape(&o.created_at),
                ));
            }
            buf
        }
        other => {
            return Err(format!(
                "Unknown export target '{}'. Use 'positions' or 'orders'.",
                other
            )
            .into());
        }
    };

    match output {
        Some(path) => {
            std::fs::write(path, &csv)
                .map_err(|e| format!("Failed to write '{}': {}", path, e))?;
            display::print_info(&format!("Wrote {}", path));
        }
        None => print!("{}", csv),
    }
    Ok(())
}

/// Wrap a field in quotes if it contains a comma, quote, or newline; escape inner quotes.
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

async fn cmd_cancel(client: &PolyClient, order_id: &str, json: bool) -> client::Result<()> {
    client.cancel_order(order_id).await?;
    if json {
        println!("{}", serde_json::json!({ "cancelled": order_id }));
    } else {
        display::print_cancelled(order_id);
    }
    Ok(())
}

async fn cmd_cancel_all(client: &PolyClient, json: bool) -> client::Result<()> {
    let orders = client.get_open_orders().await.unwrap_or_default();
    if orders.is_empty() {
        if json {
            println!("{}", serde_json::json!({ "cancelled_count": 0 }));
        } else {
            println!("{}", colored::Colorize::yellow("No open orders to cancel."));
        }
        return Ok(());
    }
    if !json {
        display::print_info(&format!("Cancelling {} open order(s)…", orders.len()));
    }
    client.cancel_all_orders().await?;
    if json {
        println!("{}", serde_json::json!({ "cancelled_count": orders.len() }));
    } else {
        display::print_cancelled_all();
    }
    Ok(())
}

async fn cmd_balance(client: &PolyClient, json: bool) -> client::Result<()> {
    let balance = client.get_balance().await?;
    let allowance = client.get_allowance().await.unwrap_or(0.0);
    if json {
        println!("{}", serde_json::json!({ "balance": balance, "allowance": allowance }));
    } else {
        display::print_balance(balance, allowance);
    }
    Ok(())
}

async fn cmd_history(client: &PolyClient, limit: usize, json: bool) -> client::Result<()> {
    if !json {
        display::print_info(&format!("Fetching last {} filled orders…", limit));
    }
    let orders = client.get_order_history(limit).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&orders).map_err(AppError::other)?);
    } else {
        display::print_history(&orders);
    }
    Ok(())
}

async fn cmd_watch(
    client: &PolyClient,
    token_id: &str,
    label: &str,
    interval_secs: u64,
) -> client::Result<()> {
    use std::io::Write;

    let name = if label.is_empty() { token_id } else { label };
    let duration = std::time::Duration::from_secs(interval_secs);

    loop {
        let book_result = client.get_order_book(token_id).await;

        // Clear screen and position cursor at top-left
        print!("\x1b[2J\x1b[H");
        std::io::stdout().flush().ok();

        let now = chrono::Local::now();
        println!(
            "  Watching {}   refreshes every {}s   Ctrl+C to quit   {}",
            name,
            interval_secs,
            now.format("%H:%M:%S"),
        );

        match book_result {
            Ok(book) => display::print_order_book(&book, name),
            Err(e) => display::print_error(&format!("Fetch failed: {}", e)),
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                print!("\x1b[2J\x1b[H");
                std::io::stdout().flush().ok();
                break;
            }
            _ = tokio::time::sleep(duration) => {}
        }
    }

    Ok(())
}

async fn cmd_top(
    client: &PolyClient,
    limit: usize,
    category: Option<&str>,
    json: bool,
) -> client::Result<()> {
    if !json {
        let label = match category {
            Some(c) => format!("top {} markets in \"{}\" by volume…", limit, c),
            None => format!("top {} markets by volume…", limit),
        };
        display::print_info(&label);
    }
    let markets = client.get_top_markets(limit, category).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&markets).map_err(AppError::other)?);
    } else {
        display::print_market_list(&markets);
    }
    Ok(())
}

async fn cmd_cancel_market(client: &PolyClient, condition_id: &str, json: bool) -> client::Result<()> {
    // Normalise to 0x-prefixed hex
    let cid = if condition_id.starts_with("0x") {
        condition_id.to_string()
    } else {
        format!("0x{}", condition_id)
    };

    // Resolve market to get its outcome token IDs
    if !json {
        display::print_info(&format!("Fetching market {}…", cid));
    }
    let market = client
        .get_market_by_id(&cid)
        .await?
        .ok_or_else(|| format!("Market not found: {}", cid))?;

    let token_ids: std::collections::HashSet<&str> =
        market.outcomes.iter().map(|o| o.token_id.as_str()).collect();

    // Find open orders whose asset_id matches one of the market's outcome tokens
    if !json {
        display::print_info("Fetching open orders…");
    }
    let all_orders = client.get_open_orders().await?;
    let matching: Vec<_> = all_orders
        .into_iter()
        .filter(|o| token_ids.contains(o.asset_id.as_str()))
        .collect();

    if matching.is_empty() {
        if json {
            println!("{}", serde_json::json!({ "cancelled": serde_json::Value::Array(vec![]) }));
        } else {
            println!("{}", colored::Colorize::yellow("No open orders for that market."));
        }
        return Ok(());
    }

    if !json {
        println!("Market : {}", market.question);
        display::print_info(&format!("Cancelling {} order(s)…", matching.len()));
        display::print_orders(&matching);
    }

    let mut cancelled_ids = Vec::new();
    for order in &matching {
        client.cancel_order(&order.id).await?;
        if json {
            cancelled_ids.push(order.id.clone());
        } else {
            display::print_cancelled(&order.id);
        }
    }

    if json {
        println!("{}", serde_json::json!({ "cancelled": cancelled_ids }));
    }

    Ok(())
}

async fn cmd_derive_keys(client: &PolyClient) -> client::Result<()> {
    let wallet = client.wallet_address_str();
    display::print_info(&format!("Deriving CLOB API credentials for wallet {}…", wallet));
    display::print_info("Fetching server timestamp from CLOB…");
    let (key, secret, passphrase) = client.derive_api_creds().await?;
    println!("\nSuccess! Add these to your .env:\n");
    println!("POLY_API_KEY={}", key);
    println!("POLY_API_SECRET={}", secret);
    println!("POLY_API_PASSPHRASE={}", passphrase);
    Ok(())
}

async fn cmd_migrate() -> client::Result<()> {
    let db_path  = persist::db_path();
    let snap_path = persist::snapshot_csv_path();
    let res_path  = persist::resolutions_csv_path();

    println!("Database : {}", db_path.display());
    println!("Snapshots: {}", snap_path.display());
    println!("Resolutions: {}", res_path.display());
    println!();

    let (snap_n, res_n) = tokio::task::spawn_blocking(move || {
        db::migrate_from_csvs(&db_path, &snap_path, &res_path)
    })
    .await
    .unwrap_or((0, 0));

    if snap_n == 0 && res_n == 0 {
        println!("Nothing imported — database already contains data or no CSV files found.");
    } else {
        println!(
            "Migration complete: {} snapshot rows and {} resolution rows imported.",
            snap_n, res_n
        );
    }

    Ok(())
}

// ── Config file ───────────────────────────────────────────────────────────────

/// Nested `[auth]` section (new format).
#[derive(serde::Deserialize, Default)]
struct AuthConfig {
    private_key:    Option<String>,
    api_key:        Option<String>,
    api_secret:     Option<String>,
    api_passphrase: Option<String>,
    funder_address: Option<String>,
    polygon_rpc_url: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct PolyConfig {
    // New nested format
    auth: Option<AuthConfig>,
    pub tui:  Option<tui::TuiConfig>,

    // Legacy flat format (backward-compat with existing ~/.poly/config.toml)
    private_key:    Option<String>,
    api_key:        Option<String>,
    api_secret:     Option<String>,
    api_passphrase: Option<String>,
    rpc_url:        Option<String>,
    funder_address: Option<String>,
}

/// Resolve the config file path:
///   1. `POLY_CONFIG` env var
///   2. `$XDG_CONFIG_HOME/poly/config.toml`  (or `~/.config/poly/config.toml`)
///   3. `~/.poly/config.toml`  (legacy fallback)
fn config_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = env::var("POLY_CONFIG") {
        return Some(std::path::PathBuf::from(p));
    }
    let home = dirs::home_dir()?;
    let xdg = env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"));
    let xdg_path = xdg.join("poly").join("config.toml");
    if xdg_path.exists() {
        return Some(xdg_path);
    }
    let legacy = home.join(".poly").join("config.toml");
    if legacy.exists() {
        return Some(legacy);
    }
    None
}

fn load_config() -> PolyConfig {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

// ── Logging ──────────────────────────────────────────────────────────────────

fn default_log_path() -> std::path::PathBuf {
    let base = env::var("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .map(|h| h.join(".local").join("share"))
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        });
    base.join("poly").join("poly.log")
}

fn init_logging(cli: &Cli) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let has_rust_log = env::var("RUST_LOG").is_ok();
    let log_file_flag = cli.log_file.as_ref();

    if log_file_flag.is_none() && !has_rust_log {
        return None;
    }

    let path = log_file_flag
        .and_then(|inner| inner.as_deref())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_log_path);

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("warning: could not open log file {}: {}", path.display(), e);
            return None;
        }
    };

    let (writer, guard) = tracing_appender::non_blocking(file);

    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .init();

    tracing::info!(log_path = %path.display(), "poly logging initialized");
    Some(guard)
}

// ── Client construction ───────────────────────────────────────────────────────

fn build_client() -> PolyClient {
    let cfg = load_config();

    // Helper: env var takes priority, then nested [auth], then legacy flat field.
    let ev = |key: &str| env::var(key).ok();
    let a = cfg.auth.as_ref();

    let private_key = ev("POLY_PRIVATE_KEY")
        .or_else(|| ev("POLY_MARKET_KEY"))
        .or_else(|| a.and_then(|a| a.private_key.clone()))
        .or(cfg.private_key);
    let funder = ev("POLY_FUNDER_ADDRESS")
        .or_else(|| a.and_then(|a| a.funder_address.clone()))
        .or(cfg.funder_address);
    let rpc = ev("POLYGON_RPC_URL")
        .or_else(|| a.and_then(|a| a.polygon_rpc_url.clone()))
        .or(cfg.rpc_url);

    let api_key = ev("POLY_API_KEY")
        .or_else(|| ev("POLY_KEY"))
        .or_else(|| a.and_then(|a| a.api_key.clone()))
        .or(cfg.api_key);
    let api_secret = ev("POLY_API_SECRET")
        .or_else(|| a.and_then(|a| a.api_secret.clone()))
        .or(cfg.api_secret);
    let api_passphrase = ev("POLY_API_PASSPHRASE")
        .or_else(|| a.and_then(|a| a.api_passphrase.clone()))
        .or(cfg.api_passphrase);

    let auth = match (api_key, api_secret, api_passphrase) {
        (Some(k), Some(s), Some(p)) => Some(auth::ClobAuth::new(k, s, p)),
        _ => None,
    };

    PolyClient::new(private_key, funder, auth, rpc)
}

fn build_tui_config() -> tui::TuiConfig {
    load_config().tui.unwrap_or_default()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn needs_auth(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Buy { .. }
            | Command::Sell { .. }
            | Command::Orders
            | Command::Positions
            | Command::Cancel { .. }
            | Command::CancelAll
            | Command::CancelMarket { .. }
            | Command::Balance
            | Command::History { .. }
            | Command::Export { .. }
            | Command::DeriveKeys
    )
}

fn parse_order_type(s: &str) -> client::Result<OrderType> {
    match s.to_uppercase().as_str() {
        "GTC" => Ok(OrderType::Gtc),
        "FOK" => Ok(OrderType::Fok),
        "IOC" => Ok(OrderType::Ioc),
        other => Err(format!("Unknown order type: {}. Use GTC, FOK, or IOC.", other).into()),
    }
}
