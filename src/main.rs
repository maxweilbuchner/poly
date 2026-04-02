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
    /// Example: poly buy 12345...789 0.65 10
    ///   buys 10 shares of token 12345...789 at $0.65 each
    Buy {
        /// CLOB token ID (from `poly market`)
        token_id: String,

        /// Limit price in USD (0.01 – 0.99)
        price: f64,

        /// Number of shares (min 5, min $1 total)
        size: f64,

        /// Order type: GTC (default), FOK, IOC
        #[arg(long, default_value = "GTC")]
        order_type: String,
    },

    /// Place a limit sell order
    Sell {
        /// CLOB token ID (from `poly market` or `poly positions`)
        token_id: String,

        /// Limit price in USD (0.01 – 0.99)
        price: f64,

        /// Number of shares to sell
        size: f64,

        /// Order type: GTC (default), FOK, IOC
        #[arg(long, default_value = "GTC")]
        order_type: String,
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
        Command::Buy { token_id, price, size, order_type } => {
            cmd_trade(&client, &token_id, price, size, Side::Buy, &order_type, cli.dry_run).await
        }
        Command::Sell { token_id, price, size, order_type } => {
            cmd_trade(&client, &token_id, price, size, Side::Sell, &order_type, cli.dry_run).await
        }
        Command::Orders => cmd_orders(&client).await,
        Command::Positions => cmd_positions(&client).await,
        Command::Cancel { order_id } => cmd_cancel(&client, &order_id).await,
        Command::CancelAll => cmd_cancel_all(&client).await,
        Command::Balance => cmd_balance(&client).await,
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
    price: f64,
    size: f64,
    side: Side,
    order_type_str: &str,
    dry_run: bool,
) -> client::Result<()> {
    // Validate inputs
    if price <= 0.0 || price >= 1.0 {
        return Err("Price must be between 0.01 and 0.99".into());
    }
    if size < 5.0 {
        return Err("Minimum order size is 5 shares".into());
    }
    if size * price < 1.0 {
        return Err("Minimum order value is $1.00 USDC".into());
    }

    let order_type = parse_order_type(order_type_str)?;

    if dry_run {
        let cost = size * price;
        display::print_info(&format!(
            "DRY RUN — {} {} shares of {} @ {:.4} (cost: ${:.4})",
            side,
            size,
            token_id,
            price,
            cost
        ));
        return Ok(());
    }

    let order_id = client.place_order(token_id, price, size, side.clone(), order_type).await?;
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
