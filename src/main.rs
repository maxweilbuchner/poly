mod auth;
mod client;
mod display;
mod types;

use clap::{Parser, Subcommand};
use client::PolyClient;
use std::env;
use types::{OrderType, Side};

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

    #[command(subcommand)]
    command: Command,
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
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    let client = build_client();

    let result = match cli.command {
        Command::Search { query, limit, all } => {
            cmd_search(&client, &query, limit, !all).await
        }
        Command::Market { id, book } => cmd_market(&client, &id, book).await,
        Command::Book { token_id, label } => cmd_book(&client, &token_id, &label).await,
        Command::Buy { token_id, size, price, order_type, market, expiry } => {
            cmd_trade(&client, &token_id, price, size, Side::Buy, &order_type, market, expiry, cli.dry_run).await
        }
        Command::Sell { token_id, size, price, order_type, market, expiry } => {
            cmd_trade(&client, &token_id, price, size, Side::Sell, &order_type, market, expiry, cli.dry_run).await
        }
        Command::Orders => cmd_orders(&client).await,
        Command::Positions => cmd_positions(&client).await,
        Command::Cancel { order_id } => cmd_cancel(&client, &order_id).await,
        Command::CancelAll => cmd_cancel_all(&client).await,
        Command::Balance => cmd_balance(&client).await,
        Command::History { limit } => cmd_history(&client, limit).await,
        Command::Watch { token_id, label, interval } => {
            cmd_watch(&client, &token_id, &label, interval).await
        }
        Command::Top { limit, category } => cmd_top(&client, limit, category.as_deref()).await,
        Command::CancelMarket { condition_id } => {
            cmd_cancel_market(&client, &condition_id).await
        }
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
) -> client::Result<()> {
    display::print_info(&format!("Searching for \"{}\"…", query));
    let markets = client.search_markets(query, active_only, limit).await?;
    display::print_market_list(&markets);
    Ok(())
}

async fn cmd_market(client: &PolyClient, id: &str, show_book: bool) -> client::Result<()> {
    // Accept full polymarket.com URLs, e.g.
    //   https://polymarket.com/event/will-trump-win-the-2024-us-presidential-election
    // Strip query string, fragment, and trailing slash, then take the last path segment.
    let slug_buf: String;
    let id: &str = if id.starts_with("http") {
        slug_buf = id
            .split(|c| c == '?' || c == '#')
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

    display::print_info(&format!("Fetching market \"{}\"…", id));

    // Try condition ID (64-char hex) first, then slug
    let markets = if id.len() == 66 || (id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()))
    {
        // Looks like a condition ID
        let cid = if id.starts_with("0x") { &id[2..] } else { id };
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

    Ok(())
}

async fn cmd_book(client: &PolyClient, token_id: &str, label: &str) -> client::Result<()> {
    let book = client.get_order_book(token_id).await?;
    let name = if label.is_empty() { token_id } else { label };
    display::print_order_book(&book, name);
    Ok(())
}

async fn cmd_trade(
    client: &PolyClient,
    token_id: &str,
    price_arg: Option<f64>,
    size: f64,
    side: Side,
    order_type_str: &str,
    market: bool,
    expiry: Option<u64>,
    dry_run: bool,
) -> client::Result<()> {
    // Resolve price and order type: market orders fetch best price from the book
    // and always use FOK; limit orders require an explicit price.
    let (price, order_type) = if market {
        let book = client.get_order_book(token_id).await?;
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
            Some(p) => (p, parse_order_type(order_type_str)?),
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
        let expiry_note = expiry
            .map(|e| format!("  expiry: {}", e))
            .unwrap_or_default();
        display::print_info(&format!(
            "DRY RUN — {} {} shares of {} @ {:.4} (cost: ${:.4}){}",
            side, size, token_id, price, cost, expiry_note
        ));
        return Ok(());
    }

    let order_id = client
        .place_order(token_id, price, size, side.clone(), order_type, expiry)
        .await?;
    display::print_order_placed(&order_id, &side, token_id, price, size);
    Ok(())
}

async fn cmd_orders(client: &PolyClient) -> client::Result<()> {
    display::print_info("Fetching open orders…");
    let orders = client.get_open_orders().await?;
    display::print_orders(&orders);
    Ok(())
}

async fn cmd_positions(client: &PolyClient) -> client::Result<()> {
    display::print_info("Fetching positions…");
    let positions = client.get_positions().await?;
    display::print_positions(&positions);
    Ok(())
}

async fn cmd_cancel(client: &PolyClient, order_id: &str) -> client::Result<()> {
    client.cancel_order(order_id).await?;
    display::print_cancelled(order_id);
    Ok(())
}

async fn cmd_cancel_all(client: &PolyClient) -> client::Result<()> {
    // List first so the user knows what was cancelled
    let orders = client.get_open_orders().await.unwrap_or_default();
    if orders.is_empty() {
        println!("{}", colored::Colorize::yellow("No open orders to cancel."));
        return Ok(());
    }
    display::print_info(&format!("Cancelling {} open order(s)…", orders.len()));
    client.cancel_all_orders().await?;
    display::print_cancelled_all();
    Ok(())
}

async fn cmd_balance(client: &PolyClient) -> client::Result<()> {
    let balance = client.get_balance().await?;
    let allowance = client.get_allowance().await.unwrap_or(0.0);
    display::print_balance(balance, allowance);
    Ok(())
}

async fn cmd_history(client: &PolyClient, limit: usize) -> client::Result<()> {
    display::print_info(&format!("Fetching last {} filled orders…", limit));
    let orders = client.get_order_history(limit).await?;
    display::print_history(&orders);
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
) -> client::Result<()> {
    let label = match category {
        Some(c) => format!("top {} markets in \"{}\" by volume…", limit, c),
        None => format!("top {} markets by volume…", limit),
    };
    display::print_info(&label);
    let markets = client.get_top_markets(limit, category).await?;
    display::print_market_list(&markets);
    Ok(())
}

async fn cmd_cancel_market(client: &PolyClient, condition_id: &str) -> client::Result<()> {
    // Normalise to 0x-prefixed hex
    let cid = if condition_id.starts_with("0x") {
        condition_id.to_string()
    } else {
        format!("0x{}", condition_id)
    };

    // Resolve market to get its outcome token IDs
    display::print_info(&format!("Fetching market {}…", cid));
    let market = client
        .get_market_by_id(&cid)
        .await?
        .ok_or_else(|| format!("Market not found: {}", cid))?;

    let token_ids: std::collections::HashSet<&str> =
        market.outcomes.iter().map(|o| o.token_id.as_str()).collect();

    // Find open orders whose asset_id matches one of the market's outcome tokens
    display::print_info("Fetching open orders…");
    let all_orders = client.get_open_orders().await?;
    let matching: Vec<_> = all_orders
        .into_iter()
        .filter(|o| token_ids.contains(o.asset_id.as_str()))
        .collect();

    if matching.is_empty() {
        println!("{}", colored::Colorize::yellow("No open orders for that market."));
        return Ok(());
    }

    println!("Market : {}", market.question);
    display::print_info(&format!("Cancelling {} order(s)…", matching.len()));
    display::print_orders(&matching);

    for order in &matching {
        client.cancel_order(&order.id).await?;
        display::print_cancelled(&order.id);
    }

    Ok(())
}

// ── Client construction ───────────────────────────────────────────────────────

fn build_client() -> PolyClient {
    let private_key = env::var("POLY_PRIVATE_KEY")
        .or_else(|_| env::var("POLY_MARKET_KEY"))
        .ok();
    let funder = env::var("POLY_FUNDER_ADDRESS").ok();
    let rpc = env::var("POLYGON_RPC_URL").ok();

    let auth = match (
        env::var("POLY_API_KEY").or_else(|_| env::var("POLY_KEY")),
        env::var("POLY_API_SECRET"),
        env::var("POLY_API_PASSPHRASE"),
    ) {
        (Ok(k), Ok(s), Ok(p)) => Some(auth::ClobAuth::new(k, s, p)),
        _ => None,
    };

    PolyClient::new(private_key, funder, auth, rpc)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_order_type(s: &str) -> client::Result<OrderType> {
    match s.to_uppercase().as_str() {
        "GTC" => Ok(OrderType::Gtc),
        "FOK" => Ok(OrderType::Fok),
        "IOC" => Ok(OrderType::Ioc),
        other => Err(format!("Unknown order type: {}. Use GTC, FOK, or IOC.", other).into()),
    }
}
