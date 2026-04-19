use colored::Colorize;

use crate::types::{Market, MarketStatus, Order, OrderBook, OrderStatus, Position, Side};

const COL_SEP: &str = "  ";

// ── Market search results ─────────────────────────────────────────────────────

pub fn print_market_list(markets: &[Market]) {
    if markets.is_empty() {
        println!("{}", "No markets found.".yellow());
        return;
    }

    let q_width = 60usize;
    println!(
        "{:<width$}{}{}",
        "QUESTION".bold(),
        COL_SEP,
        "STATUS   VOL($M)  ENDS".bold(),
        width = q_width
    );
    println!("{}", "─".repeat(100).dimmed());

    for m in markets {
        let q = truncate(&m.question, q_width);
        let vol = if m.volume >= 1_000_000.0 {
            format!("{:.1}M", m.volume / 1_000_000.0)
        } else if m.volume >= 1_000.0 {
            format!("{:.1}K", m.volume / 1_000.0)
        } else {
            format!("{:.0}", m.volume)
        };

        let end = m
            .end_date
            .as_deref()
            .and_then(|s| s.get(..10))
            .unwrap_or("—");

        let status_str = match m.status {
            MarketStatus::Active => "ACTIVE".green().to_string(),
            MarketStatus::Closed => "CLOSED".red().to_string(),
            MarketStatus::Unknown => "UNKNWN".yellow().to_string(),
        };

        println!(
            "{:<width$}{}{}   {:>7}  {}",
            q,
            COL_SEP,
            status_str,
            vol,
            end,
            width = q_width
        );
    }
}

// ── Single market detail ──────────────────────────────────────────────────────

pub fn print_market_detail(market: &Market) {
    let status_str = match market.status {
        MarketStatus::Active => "ACTIVE".green().bold(),
        MarketStatus::Closed => "CLOSED".red().bold(),
        MarketStatus::Unknown => "UNKNOWN".yellow().bold(),
    };

    println!();
    println!("{}", "═".repeat(80).dimmed());
    println!("{}", market.question.bold().white());
    println!("{}", "─".repeat(80).dimmed());
    if let Some(desc) = &market.description {
        if !desc.is_empty() {
            println!("  {}", desc.dimmed());
            println!("{}", "─".repeat(80).dimmed());
        }
    }
    println!("  Condition ID : {}", market.condition_id.cyan());
    println!("  Slug         : {}", market.slug.dimmed());
    println!("  Status       : {}", status_str);
    if let Some(end) = &market.end_date {
        println!("  Ends         : {}", end.get(..10).unwrap_or(end));
    }
    if let Some(cat) = &market.category {
        println!("  Category     : {}", cat);
    }
    let vol = format_volume(market.volume);
    let liq = format_volume(market.liquidity);
    println!("  Volume       : {}", vol.yellow());
    println!("  Liquidity    : {}", liq);

    if !market.outcomes.is_empty() {
        println!();
        println!("{}", "  OUTCOMES".bold());
        println!(
            "  {:<30} {:>8} {:>8} {:>8} {:>10}  TOKEN ID",
            "NAME", "BID", "ASK", "MID", "ASK DEPTH"
        );
        println!("  {}", "─".repeat(90).dimmed());
        for o in &market.outcomes {
            println!(
                "  {:<30} {:>8} {:>8} {:>8} {:>10}  {}",
                o.name,
                format!("{:.3}", o.bid).green().to_string(),
                format!("{:.3}", o.ask).red().to_string(),
                format!("{:.3}", o.price),
                format!("{:.1}", o.ask_depth),
                o.token_id.dimmed()
            );
        }
    }
    println!("{}", "═".repeat(80).dimmed());
    println!();
}

// ── Order book ────────────────────────────────────────────────────────────────

pub fn print_order_book(book: &OrderBook, outcome_name: &str) {
    let depth = 10usize;

    println!();
    println!("  {} {}", "ORDER BOOK —".bold(), outcome_name.bold().cyan());
    println!("  Token ID: {}", book.token_id.dimmed());
    println!();
    println!(
        "  {:>10}  {:>12}    {:>12}  {:>10}",
        "BID QTY".bold(),
        "BID PRICE".bold(),
        "ASK PRICE".bold(),
        "ASK QTY".bold()
    );
    println!("  {}", "─".repeat(54).dimmed());

    let bids: Vec<_> = book.bids.iter().take(depth).collect();
    let asks: Vec<_> = book.asks.iter().take(depth).collect();
    let rows = bids.len().max(asks.len());

    for i in 0..rows {
        let bid_str = bids
            .get(i)
            .map(|l| format!("{:>10.1}  {:>12.4}", l.size, l.price));
        let ask_str = asks
            .get(i)
            .map(|l| format!("{:>12.4}  {:>10.1}", l.price, l.size));

        let bid_display = bid_str
            .as_deref()
            .unwrap_or("                        ")
            .green()
            .to_string();
        let ask_display = ask_str
            .as_deref()
            .unwrap_or("                        ")
            .red()
            .to_string();

        println!("  {}    {}", bid_display, ask_display);
    }

    let spread = match (book.bids.first(), book.asks.first()) {
        (Some(b), Some(a)) => format!("{:.4}", a.price - b.price),
        _ => "—".to_string(),
    };
    println!();
    println!("  Spread: {}", spread.yellow());
    println!();
}

// ── Orders list ───────────────────────────────────────────────────────────────

pub fn print_orders(orders: &[Order]) {
    if orders.is_empty() {
        println!("{}", "No open orders.".yellow());
        return;
    }

    println!(
        "{:<44}  {:<5}  {:>8}  {:>8}  {:>8}  {}",
        "ORDER ID".bold(),
        "SIDE".bold(),
        "PRICE".bold(),
        "SIZE".bold(),
        "MATCHED".bold(),
        "STATUS".bold()
    );
    println!("{}", "─".repeat(100).dimmed());

    for o in orders {
        let side_str = match o.side {
            Side::Buy => "BUY".green().to_string(),
            Side::Sell => "SELL".red().to_string(),
        };
        let status_str = match o.status {
            OrderStatus::Live => "LIVE".cyan().to_string(),
            OrderStatus::Filled => "FILLED".green().to_string(),
            OrderStatus::PartiallyFilled => "PARTIAL".yellow().to_string(),
            OrderStatus::Cancelled => "CANCELLED".dimmed().to_string(),
            OrderStatus::Unknown => "UNKNOWN".dimmed().to_string(),
        };

        println!(
            "{:<44}  {:<5}  {:>8.4}  {:>8.2}  {:>8.2}  {}",
            o.id, side_str, o.price, o.original_size, o.size_matched, status_str
        );
        if !o.market.is_empty() {
            println!("  → {}", truncate(&o.market, 80).dimmed());
        }
    }
}

// ── Trade history ─────────────────────────────────────────────────────────────

pub fn print_history(orders: &[Order]) {
    if orders.is_empty() {
        println!("{}", "No trade history found.".yellow());
        return;
    }

    println!(
        "{:<44}  {:<5}  {:>8}  {:>8}  {:>8}  {}",
        "ORDER ID".bold(),
        "SIDE".bold(),
        "PRICE".bold(),
        "SIZE".bold(),
        "FILLED".bold(),
        "TIME (UTC)".bold(),
    );
    println!("{}", "─".repeat(104).dimmed());

    for o in orders {
        let side_str = match o.side {
            Side::Buy => "BUY".green().to_string(),
            Side::Sell => "SELL".red().to_string(),
        };
        // Trim to "YYYY-MM-DDTHH:MM:SS" if longer
        let time = o.created_at.get(..19).unwrap_or(&o.created_at);

        println!(
            "{:<44}  {:<5}  {:>8.4}  {:>8.2}  {:>8.2}  {}",
            o.id,
            side_str,
            o.price,
            o.original_size,
            o.size_matched,
            time.dimmed(),
        );
        if !o.market.is_empty() {
            println!("  → {}", truncate(&o.market, 80).dimmed());
        }
    }
}

// ── Positions list ────────────────────────────────────────────────────────────

pub fn print_positions(positions: &[Position]) {
    if positions.is_empty() {
        println!("{}", "No open positions.".yellow());
        return;
    }

    let total_unrealized: f64 = positions.iter().map(|p| p.unrealized_pnl).sum();
    let total_realized: f64 = positions.iter().map(|p| p.realized_pnl).sum();

    println!(
        "{:<40}  {:<20}  {:>8}  {:>8}  {:>8}  {:>10}",
        "MARKET".bold(),
        "OUTCOME".bold(),
        "SIZE".bold(),
        "AVG".bold(),
        "CURR".bold(),
        "UNRLZD PNL".bold()
    );
    println!("{}", "─".repeat(110).dimmed());

    for p in positions {
        let pnl_str = if p.unrealized_pnl >= 0.0 {
            format!("{:>+.4}", p.unrealized_pnl).green().to_string()
        } else {
            format!("{:>+.4}", p.unrealized_pnl).red().to_string()
        };

        println!(
            "{:<40}  {:<20}  {:>8.2}  {:>8.4}  {:>8.4}  {}",
            truncate(&p.market_question, 40),
            truncate(&p.outcome, 20),
            p.size,
            p.avg_price,
            p.current_price,
            pnl_str
        );
    }

    println!("{}", "─".repeat(110).dimmed());
    let unrlzd = if total_unrealized >= 0.0 {
        format!("{:>+.4}", total_unrealized).green().to_string()
    } else {
        format!("{:>+.4}", total_unrealized).red().to_string()
    };
    println!(
        "{:<40}  {:<20}  {:>8}  {:>8}  {:>8}  {}",
        "TOTAL".bold(),
        "",
        "",
        "",
        "",
        unrlzd
    );

    let cost_basis: f64 = positions.iter().map(|p| p.size * p.avg_price).sum();
    let portfolio_value: f64 = positions.iter().map(|p| p.size * p.current_price).sum();
    let return_pct = if cost_basis > 0.0 {
        (portfolio_value - cost_basis) / cost_basis * 100.0
    } else {
        0.0
    };
    let val_str = format!("${:.2}", portfolio_value).bold();
    let cost_str = format!("${:.2}", cost_basis);
    println!(
        "  Portfolio value: {}  Cost basis: {}  Return: {}",
        val_str,
        cost_str,
        format_pnl_pct(return_pct),
    );
    println!("  Realized PnL: {}", format_pnl(total_realized));
}

// ── Balance ───────────────────────────────────────────────────────────────────

pub fn print_balance(balance: f64, allowance: f64) {
    println!();
    println!(
        "  USDC Balance  : {}",
        format!("${:.2}", balance).green().bold()
    );
    println!(
        "  CTF Allowance : {}",
        if allowance > 1e18 {
            "Unlimited".green().to_string()
        } else if allowance >= 10.0 {
            format!("${:.2}", allowance).green().to_string()
        } else {
            format!("${:.2} ⚠ low", allowance).yellow().to_string()
        }
    );
    println!();
}

// ── Order confirmation ────────────────────────────────────────────────────────

pub fn print_order_placed(order_id: &str, side: &Side, token_id: &str, price: f64, size: f64) {
    let side_str = match side {
        Side::Buy => "BUY".green().bold(),
        Side::Sell => "SELL".red().bold(),
    };
    println!();
    println!(
        "  {} {} shares @ {} on token {}",
        side_str,
        size,
        format!("{:.4}", price).yellow(),
        token_id.dimmed()
    );
    println!("  Order ID: {}", order_id.cyan().bold());
    println!();
}

pub fn print_cancelled(order_id: &str) {
    println!(
        "  {} order {}",
        "Cancelled".yellow().bold(),
        order_id.dimmed()
    );
}

pub fn print_cancelled_all() {
    println!("  {} all open orders.", "Cancelled".yellow().bold());
}

// ── Error / info helpers ──────────────────────────────────────────────────────

pub fn print_error(msg: &str) {
    eprintln!("{} {}", "error:".red().bold(), msg);
}

pub fn print_warning(msg: &str) {
    eprintln!("{} {}", "warning:".yellow().bold(), msg);
}

pub fn print_info(msg: &str) {
    println!("{} {}", "→".dimmed(), msg);
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!("{}…", chars[..max - 1].iter().collect::<String>())
    }
}

fn format_volume(v: f64) -> String {
    if v >= 1_000_000.0 {
        format!("${:.2}M", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("${:.1}K", v / 1_000.0)
    } else {
        format!("${:.2}", v)
    }
}

fn format_pnl(v: f64) -> String {
    if v >= 0.0 {
        format!("{:>+.4}", v).green().to_string()
    } else {
        format!("{:>+.4}", v).red().to_string()
    }
}

fn format_pnl_pct(v: f64) -> String {
    if v >= 0.0 {
        format!("{:>+.1}%", v).green().to_string()
    } else {
        format!("{:>+.1}%", v).red().to_string()
    }
}
