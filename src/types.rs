use serde::{Deserialize, Serialize};

// ── Market ────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct Market {
    pub condition_id: String,
    pub question: String,
    pub slug: String,
    pub status: MarketStatus,
    pub end_date: Option<String>,
    pub volume: f64,
    pub liquidity: f64,
    pub outcomes: Vec<Outcome>,
    pub category: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum MarketStatus {
    Active,
    Closed,
    Unknown,
}

impl std::fmt::Display for MarketStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketStatus::Active => write!(f, "ACTIVE"),
            MarketStatus::Closed => write!(f, "CLOSED"),
            MarketStatus::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub name: String,
    pub token_id: String,
    pub price: f64,
    pub bid: f64,
    pub ask: f64,
    pub bid_depth: f64,
    pub ask_depth: f64,
}

// ── Order Book ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct OrderBook {
    pub token_id: String,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PriceLevel {
    pub price: f64,
    pub size: f64,
}

// ── Orders ────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct Order {
    pub id: String,
    pub asset_id: String,
    pub side: Side,
    pub price: f64,
    pub original_size: f64,
    pub size_matched: f64,
    pub status: OrderStatus,
    pub outcome: String,
    pub market: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Buy => write!(f, "BUY"),
            Side::Sell => write!(f, "SELL"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum OrderStatus {
    Live,
    Filled,
    #[allow(dead_code)]
    PartiallyFilled,
    Cancelled,
    Unknown,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderStatus::Live => write!(f, "LIVE"),
            OrderStatus::Filled => write!(f, "FILLED"),
            OrderStatus::PartiallyFilled => write!(f, "PARTIAL"),
            OrderStatus::Cancelled => write!(f, "CANCELLED"),
            OrderStatus::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

// ── Positions ────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct Position {
    pub market_id: String,
    pub market_question: String,
    pub outcome: String,
    pub token_id: String,
    pub size: f64,
    pub avg_price: f64,
    pub current_price: f64,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
}

// ── Order Type ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum OrderType {
    /// Good-til-cancelled limit order
    Gtc,
    /// Fill-or-kill (market-style)
    Fok,
    /// Immediate-or-cancel
    Ioc,
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderType::Gtc => write!(f, "GTC"),
            OrderType::Fok => write!(f, "FOK"),
            OrderType::Ioc => write!(f, "IOC"),
        }
    }
}
