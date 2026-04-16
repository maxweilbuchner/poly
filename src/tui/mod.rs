pub mod screens {
    pub mod analytics;
    pub mod balance;
    pub mod detail;
    pub mod markets;
    pub mod order;
    pub mod positions;
    pub mod setup;
}

pub mod widgets {
    pub mod order_book;
    pub mod status_bar;
    pub mod tab_bar;
}

pub mod theme;
mod ui;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::watch;

use crate::client::{self, PolyClient};
use crate::error::AppError;
use crate::types::{
    Market, MarketStatus, Order, OrderBook, OrderType, OutcomeSeries, PlaceOrderParams, Position,
    PricePoint, Side,
};

// ── TUI configuration (from config file [tui] section) ───────────────────────

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct TuiConfig {
    /// Auto-refresh interval for positions/orders (seconds). Default: 30.
    pub refresh_interval_secs: Option<u64>,
    /// Cap on total markets loaded. Default: 2500.
    pub max_markets: Option<usize>,
    /// Open order forms in dry-run mode by default. Default: false.
    pub default_dry_run: Option<bool>,
}

// ── Domain enums ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Tab {
    Markets,
    Positions,
    Balance,
    Analytics,
}

#[derive(Debug, Clone)]
pub enum Screen {
    MarketList,
    MarketDetail,
    OrderEntry,
    CloseConfirm, // fast-path: confirm closing a full position without the order form
    CancelAllConfirm, // confirm cancelling all open orders
    RedeemConfirm, // confirm on-chain redemption of a single resolved position
    RedeemAllConfirm, // confirm on-chain redemption of all redeemable positions
    Help,
    QuitConfirm,
    Setup,
}

// ── Sort / filter modes ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum SortMode {
    #[default]
    Volume,
    EndDate,
    Probability,
}

impl SortMode {
    pub fn next(&self) -> Self {
        match self {
            SortMode::Volume => SortMode::EndDate,
            SortMode::EndDate => SortMode::Probability,
            SortMode::Probability => SortMode::Volume,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            SortMode::Volume => "vol",
            SortMode::EndDate => "end date",
            SortMode::Probability => "prob",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum DateFilter {
    #[default]
    All,
    Hours3,
    Hours6,
    Hours12,
    Hours24,
    Week,
    Month,
}

impl DateFilter {
    pub fn next(&self) -> Self {
        match self {
            DateFilter::All => DateFilter::Hours3,
            DateFilter::Hours3 => DateFilter::Hours6,
            DateFilter::Hours6 => DateFilter::Hours12,
            DateFilter::Hours12 => DateFilter::Hours24,
            DateFilter::Hours24 => DateFilter::Week,
            DateFilter::Week => DateFilter::Month,
            DateFilter::Month => DateFilter::All,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            DateFilter::All => "all",
            DateFilter::Hours3 => "3h",
            DateFilter::Hours6 => "6h",
            DateFilter::Hours12 => "12h",
            DateFilter::Hours24 => "24h",
            DateFilter::Week => "7d",
            DateFilter::Month => "30d",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum ProbFilter {
    #[default]
    All,
    Prob90_98,
    Prob85_98,
    Prob80_98,
}

impl ProbFilter {
    pub fn next(&self) -> Self {
        match self {
            ProbFilter::All => ProbFilter::Prob90_98,
            ProbFilter::Prob90_98 => ProbFilter::Prob85_98,
            ProbFilter::Prob85_98 => ProbFilter::Prob80_98,
            ProbFilter::Prob80_98 => ProbFilter::All,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            ProbFilter::All => "all",
            ProbFilter::Prob90_98 => "90-98%",
            ProbFilter::Prob85_98 => "85-98%",
            ProbFilter::Prob80_98 => "80-98%",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum VolumeFilter {
    #[default]
    All,
    K1,
    K10,
    K100,
}

impl VolumeFilter {
    pub fn next(&self) -> Self {
        match self {
            VolumeFilter::All => VolumeFilter::K1,
            VolumeFilter::K1 => VolumeFilter::K10,
            VolumeFilter::K10 => VolumeFilter::K100,
            VolumeFilter::K100 => VolumeFilter::All,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            VolumeFilter::All => "all",
            VolumeFilter::K1 => ">1K",
            VolumeFilter::K10 => ">10K",
            VolumeFilter::K100 => ">100K",
        }
    }
    pub fn min_volume(&self) -> f64 {
        match self {
            VolumeFilter::All => 0.0,
            VolumeFilter::K1 => 1_000.0,
            VolumeFilter::K10 => 10_000.0,
            VolumeFilter::K100 => 100_000.0,
        }
    }
}

// ── Order form ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct OrderForm {
    pub side: Option<Side>,
    pub token_id: String,
    pub outcome_name: String,
    pub size_input: String,
    pub price_input: String,
    pub order_type: OrderType,
    pub dry_run: bool,
    /// 0 = size, 1 = price, 2 = order_type
    pub focused_field: u8,
    /// True when the user has selected "Market" order mode.
    pub market_order: bool,
    /// Best ask (buy) or best bid (sell) fetched from the order book.
    pub market_price: Option<f64>,
    /// True when the market price fetch failed — lets the form show "failed [r retry]".
    pub market_price_failed: bool,
    /// Fee rate in basis points fetched from the CLOB API.
    pub fee_rate_bps: Option<u64>,
    /// Whether this token uses the Neg Risk CTF Exchange for signing.
    pub neg_risk: bool,
    /// True when opened via "close position" shortcut — changes form title.
    pub close_position: bool,
    /// When set, the size field is capped at this value (shares held).
    /// Used for sell/close-from-position so we can warn before submit.
    pub max_size: Option<f64>,
}

impl OrderForm {
    pub fn cost(&self) -> Option<f64> {
        let size: f64 = self.size_input.parse().ok()?;
        let price = if self.market_order {
            self.market_price?
        } else {
            self.price_input.parse().ok()?
        };
        Some(size * price)
    }
}

// ── App events (from background tasks → main loop) ────────────────────────────

pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    MarketsLoaded(Vec<Market>, bool),   // (markets, is_final_page)
    MarketsAppended(Vec<Market>, bool), // (markets, is_final_page)
    MarketDetailLoaded(Market, Vec<(String, OrderBook)>),
    OrderBookUpdated(Vec<(String, OrderBook)>),
    PositionsLoaded(Vec<Position>),
    OrdersLoaded(Vec<Order>),
    BalanceLoaded(f64, f64),
    OrderPlaced(String),
    OrderCancelled(String),
    /// On-chain redemption completed — payload is the tx hash.
    Redeemed(String),
    MarketPriceFetched(f64),
    FeeRateFetched(u64),
    Error(crate::error::AppError),
    /// Price history loaded: (condition_id, interval_label, prices_per_outcome)
    /// Each outcome entry is (outcome_name, Vec<(timestamp, price)>)
    PriceHistoryLoaded(String, String, Vec<OutcomeSeries>),

    /// Hourly market snapshot events.
    SnapshotProgress(usize),
    SnapshotComplete(usize),
    ResolutionsUpdated(usize, Vec<String>),
    SnapshotError(String),

    /// Analytics computation finished.
    AnalyticsComputed(Box<AnalyticsStats>),

    /// Calibration price fetch progress: (fetched_so_far, total_pending).
    CalibrationFetchProgress(usize, usize),

    /// Startup credential probe result.
    /// `None` = credentials absent or valid; `Some(msg)` = present but rejected by API.
    AuthChecked(Option<String>),

    /// Background-loaded startup data (avoids blocking App::new on disk I/O).
    SnapshotMetaLoaded(crate::persist::SnapshotMeta),
    ResolvedIdsLoaded(HashSet<String>),
}

// ── Analytics stats ───────────────────────────────────────────────────────────

/// Row labels for `calibration_matrix`. Index matches the outer array dim.
pub const CALIB_CATEGORIES: [&str; 6] = [
    "Politics", "Sports", "Crypto", "Finance", "Weather", "Other",
];

/// Per-cell predicted-vs-actual histogram feeding the calibration regression.
#[derive(Debug, Clone, Copy, Default)]
pub struct CalibCell {
    /// Per 10% predicted-probability bucket: (yes_resolutions, total).
    pub buckets: [(u32, u32); 10],
    pub n: u32,
}

#[derive(Debug, Clone, Default)]
pub struct AnalyticsStats {
    /// A — count of active markets per 5-% probability bucket (index 0 = 0–4 %).
    pub prob_buckets: [u64; 20],
    /// D — (volume_tier_label, mean_abs_error, count) per tier; 5 tiers <$1K..>$1M.
    pub edge_vs_vol: Vec<(String, f64, usize)>,
    /// C — resolution breakdown.
    pub res_yes: usize,
    pub res_no: usize,
    pub res_other: usize,
    /// D — high-confidence accuracy: markets where a >80% price was seen in
    /// the last 6 h before close, and whether that prediction was correct.
    pub hc_correct: usize,
    pub hc_wrong: usize,
    /// Summary bar — totals across the latest snapshot.
    pub total_markets: usize,
    pub total_volume: f64,
    /// D — calibration: per 10% bucket, (yes_resolutions, total_resolutions).
    pub calibration: [(usize, usize); 10],
    /// D — calibration per (category × volume tier). See CALIB_CATEGORIES / CALIB_VOL_TIERS.
    pub calibration_matrix: [[CalibCell; 5]; 6],
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct App {
    pub active_tab: Tab,
    pub screen_stack: Vec<Screen>,

    // Markets
    pub markets: Vec<Market>,
    pub search_query: String,
    pub search_mode: bool,
    pub market_list_state: ratatui::widgets::ListState,
    pub sort_mode: SortMode,
    pub date_filter: DateFilter,
    pub prob_filter: ProbFilter,
    pub volume_filter: VolumeFilter,
    pub category_filter: Option<String>,

    // Market detail
    pub selected_market: Option<Market>,
    pub order_books: Vec<(String, OrderBook)>,

    // Positions & orders
    pub positions: Vec<Position>,
    pub orders: Vec<Order>,
    pub positions_focus_orders: bool, // false = positions panel, true = orders panel
    pub positions_list_state: ratatui::widgets::ListState,
    pub orders_list_state: ratatui::widgets::ListState,

    // Balance
    pub balance: Option<f64>,
    pub allowance: Option<f64>,

    // Flash message — (text, shown_at, is_error). Errors stay 5s; others 3s.
    pub flash: Option<(String, Instant, bool)>,

    // Order form
    pub order_form: OrderForm,

    // Loading / error
    pub loading: bool,
    pub markets_loading_more: bool,
    pub last_error: Option<crate::error::AppError>,

    // Cached result of filtered_markets() — indices into self.markets.
    // Rebuilt eagerly whenever markets or any filter changes.
    pub filtered_indices: Vec<usize>,

    // O(1) dedup set — condition_ids of every market currently in `self.markets`.
    pub market_id_set: HashSet<String>,

    // Cached sorted category list, recomputed inside rebuild_filter().
    pub cached_categories: Vec<String>,

    // Watchlist — starred condition_ids, persisted across sessions
    pub watchlist: HashSet<String>,
    pub watchlist_only: bool,

    // Price history sparkline — keyed by condition_id + interval
    // Value: Vec<(outcome_name, prices)>
    pub price_history: HashMap<String, Vec<OutcomeSeries>>,
    /// Current interval for sparkline display: "1d" | "1w"
    pub sparkline_interval: &'static str,

    // WebSocket order book feed — cancel signal for the active WS task
    pub ws_cancel: Option<tokio::sync::watch::Sender<bool>>,
    /// Timestamp of the last received order book update (WS or HTTP fallback).
    pub order_book_updated_at: Option<Instant>,

    // Root menu cursor position
    pub menu_index: usize,

    // Which position is being closed (set when CloseConfirm is pushed)
    pub close_confirm_pos_idx: Option<usize>,
    // Which position is being redeemed (set when RedeemConfirm is pushed)
    pub redeem_confirm_pos_idx: Option<usize>,

    // Selected outcome index in the detail screen
    pub detail_outcome_index: usize,

    // Spinner frame counter (incremented on each Tick)
    pub tick: u64,

    // When positions data last arrived — used for auto-refresh timer and "Xs ago" display
    pub positions_refreshed_at: Option<Instant>,

    // Runtime config (from ~/.config/poly/config.toml [tui] section)
    pub refresh_interval_secs: u64,
    pub max_markets: usize,

    // Hourly market snapshot state
    pub db_path: std::path::PathBuf,
    pub snapshot_in_progress: bool,
    /// Wall-clock time of the last completed snapshot — loaded from disk at startup
    /// so the hourly schedule and "X ago" display survive restarts.
    pub snapshot_last_at: Option<chrono::DateTime<chrono::Utc>>,
    pub snapshot_last_count: usize,
    pub snapshot_fetched_so_far: usize,
    pub snapshot_error: Option<String>,
    pub known_resolved_ids: HashSet<String>,
    pub resolutions_new_last_run: usize,

    // Analytics dashboard
    pub analytics_stats: Option<AnalyticsStats>,
    /// Stats from the immediately preceding compute run — used to render ▲/▼ deltas.
    pub analytics_stats_prev: Option<AnalyticsStats>,
    pub analytics_loading: bool,
    /// Progress of the in-flight calibration price fetch (done, total). Both 0 when idle.
    pub calibration_fetch_done: usize,
    pub calibration_fetch_total: usize,
    /// Whether the snapshot status panel is collapsed to a 1-line strip.
    pub analytics_panel_collapsed: bool,
    /// Hours before market close used for Chart D calibration (3 / 6 / 9 / 12).
    pub calibration_hours: u64,
    /// Whether Chart C regression is observation-weighted (WLS) or uniform (OLS).
    pub regression_weighted: bool,

    /// Persistent warning set by the startup credential probe.
    /// Shown in the status bar until cleared (currently never cleared — restart to re-check).
    pub auth_warning: Option<String>,

    // Setup wizard form state
    pub setup_form: screens::setup::SetupForm,
    /// Set to true after setup completes — signals the event loop to exit for restart.
    pub setup_complete: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            active_tab: Tab::Markets,
            screen_stack: vec![Screen::MarketList],

            markets: Vec::new(),
            search_query: String::new(),
            search_mode: false,
            market_list_state: ratatui::widgets::ListState::default(),
            sort_mode: SortMode::default(),
            date_filter: DateFilter::default(),
            prob_filter: ProbFilter::default(),
            volume_filter: VolumeFilter::default(),
            category_filter: None,

            selected_market: None,
            order_books: Vec::new(),

            positions: Vec::new(),
            orders: Vec::new(),
            positions_focus_orders: false,
            positions_list_state: ratatui::widgets::ListState::default(),
            orders_list_state: ratatui::widgets::ListState::default(),

            balance: None,
            allowance: None,

            flash: None,

            order_form: OrderForm::default(),

            filtered_indices: Vec::new(),
            market_id_set: HashSet::new(),
            cached_categories: Vec::new(),

            watchlist: crate::persist::load_watchlist(),
            watchlist_only: false,

            price_history: HashMap::new(),
            sparkline_interval: "1d",

            ws_cancel: None,
            order_book_updated_at: None,

            loading: false,
            markets_loading_more: false,
            last_error: None,

            menu_index: 0,
            close_confirm_pos_idx: None,
            redeem_confirm_pos_idx: None,
            detail_outcome_index: 0,
            tick: 0,
            positions_refreshed_at: None,
            refresh_interval_secs: 30,
            max_markets: MAX_MARKETS,

            db_path: crate::persist::db_path(),
            snapshot_in_progress: false,
            snapshot_last_at: None,
            snapshot_last_count: 0,
            snapshot_fetched_so_far: 0,
            snapshot_error: None,
            known_resolved_ids: HashSet::new(),
            resolutions_new_last_run: 0,

            analytics_stats: None,
            analytics_stats_prev: None,
            analytics_loading: false,
            calibration_fetch_done: 0,
            calibration_fetch_total: 0,
            analytics_panel_collapsed: false,
            calibration_hours: 3,
            regression_weighted: true,

            auth_warning: None,

            setup_form: screens::setup::SetupForm::default(),
            setup_complete: false,
        }
    }

    pub fn set_flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now(), false));
    }

    pub fn set_error_flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now(), true));
    }

    pub fn save_ui_state(&self) {
        crate::persist::save_ui_state(&crate::persist::UiState {
            sort_mode: self.sort_mode.clone(),
            date_filter: self.date_filter.clone(),
            prob_filter: self.prob_filter.clone(),
            volume_filter: self.volume_filter.clone(),
            category_filter: self.category_filter.clone(),
        });
    }

    pub fn current_screen(&self) -> Option<&Screen> {
        self.screen_stack.last()
    }

    /// Rebuild `filtered_indices` from the current markets + filter state.
    /// Call this whenever `markets`, `search_query`, `sort_mode`, `date_filter`,
    /// `prob_filter`, or `category_filter` changes.
    pub fn rebuild_filter(&mut self) {
        use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};

        let now = Local::now();
        let q_lower = self.search_query.to_lowercase();

        let mut indices: Vec<usize> = self
            .markets
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                // text search
                if !q_lower.is_empty() && !m.question.to_lowercase().contains(&q_lower) {
                    return None;
                }
                // date filter
                if self.date_filter != DateFilter::All {
                    let cutoff: DateTime<Local> = match self.date_filter {
                        DateFilter::Hours3 => now + Duration::hours(3),
                        DateFilter::Hours6 => now + Duration::hours(6),
                        DateFilter::Hours12 => now + Duration::hours(12),
                        DateFilter::Hours24 => now + Duration::hours(24),
                        DateFilter::Week => now + Duration::days(7),
                        DateFilter::Month => now + Duration::days(30),
                        DateFilter::All => unreachable!(),
                    };
                    match &m.end_date {
                        Some(end) => {
                            // Try full ISO datetime first, fall back to date-only (treat as midnight UTC)
                            let market_dt: Option<DateTime<Local>> =
                                DateTime::parse_from_rfc3339(end)
                                    .map(|dt| dt.with_timezone(&Local))
                                    .ok()
                                    .or_else(|| {
                                        NaiveDate::parse_from_str(
                                            end.get(..10).unwrap_or(""),
                                            "%Y-%m-%d",
                                        )
                                        .ok()
                                        .and_then(|d| d.and_hms_opt(0, 0, 0))
                                        .map(|ndt| {
                                            Utc.from_utc_datetime(&ndt).with_timezone(&Local)
                                        })
                                    });
                            match market_dt {
                                Some(dt) if dt > now && dt <= cutoff => {}
                                _ => return None,
                            }
                        }
                        None => return None,
                    }
                }
                // category filter
                if let Some(cat) = &self.category_filter {
                    if market_category(m) != Some(cat.as_str()) {
                        return None;
                    }
                }
                // prob filter
                let (lo, hi) = match self.prob_filter {
                    ProbFilter::All => (0.0, 1.0),
                    ProbFilter::Prob90_98 => (0.90, 0.98),
                    ProbFilter::Prob85_98 => (0.85, 0.98),
                    ProbFilter::Prob80_98 => (0.80, 0.98),
                };
                if lo > 0.0 {
                    let best = m
                        .outcomes
                        .iter()
                        .map(|o| o.price)
                        .fold(f64::NEG_INFINITY, f64::max);
                    if best < lo || best > hi {
                        return None;
                    }
                }
                // volume filter
                let min_vol = self.volume_filter.min_volume();
                if min_vol > 0.0 && m.volume < min_vol {
                    return None;
                }
                // watchlist filter
                if self.watchlist_only && !self.watchlist.contains(&m.condition_id) {
                    return None;
                }
                Some(i)
            })
            .collect();

        match &self.sort_mode {
            SortMode::Volume => {
                indices.sort_by(|&a, &b| self.markets[b].volume.total_cmp(&self.markets[a].volume));
            }
            SortMode::EndDate => {
                indices.sort_by(|&a, &b| {
                    let a_d = self.markets[a].end_date.as_deref().unwrap_or("9999");
                    let b_d = self.markets[b].end_date.as_deref().unwrap_or("9999");
                    a_d.cmp(b_d)
                });
            }
            SortMode::Probability => {
                indices.sort_by(|&a, &b| {
                    let pa = self.markets[a]
                        .outcomes
                        .iter()
                        .map(|o| o.price)
                        .fold(f64::NEG_INFINITY, f64::max);
                    let pb = self.markets[b]
                        .outcomes
                        .iter()
                        .map(|o| o.price)
                        .fold(f64::NEG_INFINITY, f64::max);
                    pb.total_cmp(&pa)
                });
            }
        }

        self.filtered_indices = indices;

        // Recompute categories while we already touched every market.
        let mut seen = HashSet::new();
        for m in &self.markets {
            if let Some(cat) = market_category(m) {
                seen.insert(cat);
            }
        }
        let mut cats: Vec<String> = seen.into_iter().map(|s| s.to_string()).collect();
        cats.sort();
        self.cached_categories = cats;
    }

    /// Returns the cached filtered+sorted market list. O(n) on indices only.
    pub fn filtered_markets(&self) -> Vec<&Market> {
        self.filtered_indices
            .iter()
            .map(|&i| &self.markets[i])
            .collect()
    }
}

/// Infers a display category for a market from its question + slug.
/// The Gamma API no longer populates `category`/`tags` on market list endpoints.
pub(crate) fn market_category(m: &crate::types::Market) -> Option<&'static str> {
    market_category_from_parts(&m.question, &m.slug)
}

pub(crate) fn market_category_from_parts(question: &str, slug: &str) -> Option<&'static str> {
    let q = question.to_lowercase();
    let s = slug.to_lowercase();

    // Sports: slug prefix is the most reliable signal for league markets
    let sports_slug_kw = [
        "nba-",
        "nfl-",
        "mlb-",
        "nhl-",
        "mls-",
        "pga-",
        "ufc-",
        "epl-",
        "elc-",
        "efl-",
        "ucl-",
        "uel-",
        "aus-",
        "ligue1-",
        "bundesliga-",
        "laliga-",
        "seriea-",
        "a-league-",
    ];
    if sports_slug_kw.iter().any(|k| s.starts_with(k)) {
        return Some("Sports");
    }

    // Weather
    if q.contains("highest temperature")
        || q.contains("temperature in ")
        || q.contains("weather in ")
    {
        return Some("Weather");
    }

    // Crypto
    let crypto_kw = [
        "bitcoin",
        " btc ",
        "ethereum",
        " eth ",
        "solana",
        "dogecoin",
        " doge",
        " xrp",
        "crypto",
        "blockchain",
        " defi",
        " nft",
        "stablecoin",
        "market cap",
        "usdt",
        "usdc",
        "altcoin",
        "layer 2",
        "layer2",
        " fdv ",
        "fdv above",
        "launch a token",
        "token by ",
    ];
    if crypto_kw.iter().any(|k| q.contains(k) || s.contains(k)) {
        return Some("Crypto");
    }

    // Finance
    let finance_kw = [
        "nasdaq",
        "s&p 500",
        " spy ",
        "dow jones",
        "interest rate",
        " gdp",
        "inflation",
        "federal reserve",
        "bank of ",
        "treasury",
        "bond yield",
        "crude oil",
        " silver ",
        "gold price",
        "natural gas",
        "stock market",
        "ipo ",
        "earnings",
        "recession",
        "tariff",
    ];
    if finance_kw.iter().any(|k| q.contains(k) || s.contains(k)) {
        return Some("Finance");
    }

    // Sports: question-level keywords for leagues/sports not caught by slug
    let sports_q_kw = [
        "nba",
        "nfl",
        "mlb",
        "nhl",
        "mls",
        "premier league",
        "la liga",
        "bundesliga",
        "serie a",
        "ligue 1",
        "champions league",
        "europa league",
        "conference league",
        "world cup",
        "super bowl",
        "pga tour",
        "ufc",
        "formula 1",
        " f1 ",
        "wimbledon",
        " golf",
        "tennis",
        "nascar",
        "exact score:",
        "moneyline",
        "top 5 at",
        "top 10 at",
        "top 20 at",
        "finish in the top",
        " fc ",
        "united fc",
        "city fc",
        "town fc",
        " o/u ",
        "handicap:",
        "both teams to score",
        "spread:",
        "valero",
        "masters 20",
        "open championship",
        "the open",
        "esports",
        "dota 2",
        "counter-strike",
        " lcs",
        " lpl",
        "valorant",
        "win on 202",
        "win the 202",
        "end in a draw",
    ];
    if sports_q_kw.iter().any(|k| q.contains(k) || s.contains(k)) {
        return Some("Sports");
    }

    // Politics
    let politics_kw = [
        "trump",
        "election",
        " senate",
        " congress",
        "president",
        "democrat",
        "republican",
        " ballot",
        "prime minister",
        "parliament",
        "government",
        "sanction",
        " vote ",
        "military",
        "nato",
        "ceasefire",
        "legislation",
        "invade",
        "invasion",
        "blockade",
        "taiwan",
        "ukraine",
        "russia",
        "le pen",
        "macron",
        "zelensky",
        "recognize israel",
        "recognize ",
        "guilty of",
        "out of custody",
        "in custody",
        "criminal trial",
        "mayor of",
        "governor of",
    ];
    if politics_kw.iter().any(|k| q.contains(k) || s.contains(k)) {
        return Some("Politics");
    }

    None
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(client: PolyClient, tui_cfg: TuiConfig) -> client::Result<()> {
    use crossterm::{
        execute,
        terminal::{
            disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
        },
    };
    use ratatui::{backend::CrosstermBackend, Terminal};
    use std::io;

    // Install panic hook to restore terminal and write a crash log.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);

        // Write crash details to $XDG_DATA_HOME/poly/crash.log so the user
        // has something concrete to attach to a bug report.
        let crash_path = std::env::var("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .map(|h| h.join(".local").join("share"))
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
            })
            .join("poly")
            .join("crash.log");
        if let Some(parent) = crash_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let timestamp = chrono::Utc::now().to_rfc3339();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let entry = format!(
            "--- crash at {} ---\n{}\n\nBacktrace:\n{}\n\n",
            timestamp, info, backtrace
        );
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_path)
        {
            use std::io::Write;
            let _ = f.write_all(entry.as_bytes());
            eprintln!("poly crashed — details written to {}", crash_path.display());
        }

        original_hook(info);
    }));

    enable_raw_mode().map_err(AppError::other)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, SetTitle("POLY")).map_err(AppError::other)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(AppError::other)?;

    let (result, setup_done) = run_app(&mut terminal, client, tui_cfg).await;

    // Always restore terminal, even on error.
    disable_raw_mode().map_err(AppError::other)?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen).map_err(AppError::other)?;
    terminal.show_cursor().map_err(AppError::other)?;

    if setup_done {
        println!();
        println!("  Configuration saved. Run `poly` again to start with your new credentials.");
        println!();
    }

    result
}

async fn run_app(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    client: PolyClient,
    tui_cfg: TuiConfig,
) -> (client::Result<()>, bool) {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let client = Arc::new(client);

    // Spawn input reader task.
    let tx_input = tx.clone();
    tokio::spawn(async move {
        loop {
            if crossterm::event::poll(Duration::from_millis(50)).unwrap_or(false) {
                if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read() {
                    if tx_input.send(AppEvent::Key(k)).is_err() {
                        break;
                    }
                }
            } else if tx_input.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });

    // One-time migration: import legacy CSV data into SQLite if the DB is empty.
    // No-op for fresh installs and for users already on SQLite.
    {
        let db_p = crate::persist::db_path();
        let snap_p = crate::persist::snapshot_csv_path();
        let res_p = crate::persist::resolutions_csv_path();
        tokio::task::spawn_blocking(move || {
            crate::db::migrate_from_csvs(&db_p, &snap_p, &res_p);
        })
        .await
        .ok();
    }

    let mut app = App::new();
    app.refresh_interval_secs = tui_cfg.refresh_interval_secs.unwrap_or(30);
    app.max_markets = tui_cfg.max_markets.unwrap_or(MAX_MARKETS);
    app.order_form.dry_run = tui_cfg.default_dry_run.unwrap_or(false);

    // Restore persisted filter/sort state.
    let ui_state = crate::persist::load_ui_state();
    app.sort_mode = ui_state.sort_mode;
    app.date_filter = ui_state.date_filter;
    app.prob_filter = ui_state.prob_filter;
    app.volume_filter = ui_state.volume_filter;
    app.category_filter = ui_state.category_filter;

    // Auto-show setup wizard on first launch when no credentials are configured.
    if !crate::setup::has_config() && !client.has_credentials() {
        app.setup_form.is_first_launch = true;
        app.screen_stack.push(Screen::Setup);
    }

    app.loading = true;
    spawn_load_markets(Arc::clone(&client), tx.clone(), app.max_markets);

    // Probe credentials in the background so any auth problem is visible before
    // the user navigates to trading screens and attempts to place an order.
    {
        let client2 = Arc::clone(&client);
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let warning = client2.check_credentials().await;
            let _ = tx2.send(AppEvent::AuthChecked(warning));
        });
    }

    // Load snapshot metadata and resolved IDs off the main thread so the first
    // frame renders immediately instead of blocking on disk I/O.
    {
        let tx2 = tx.clone();
        tokio::task::spawn_blocking(move || {
            let meta = crate::persist::load_snapshot_meta();
            let _ = tx2.send(AppEvent::SnapshotMetaLoaded(meta));
        });
    }
    {
        let tx2 = tx.clone();
        let db = app.db_path.clone();
        tokio::task::spawn_blocking(move || {
            let ids = crate::db::open(&db)
                .and_then(|c| crate::db::load_resolved_ids(&c))
                .unwrap_or_default();
            let _ = tx2.send(AppEvent::ResolvedIdsLoaded(ids));
        });
    }

    loop {
        if let Err(e) = terminal.draw(|f| ui::render(f, &mut app)) {
            return (Err(AppError::other(e)), app.setup_complete);
        }

        match rx.recv().await {
            Some(event) => {
                if handle_event(&mut app, event, Arc::clone(&client), &tx) {
                    break;
                }
                if app.setup_complete {
                    break;
                }
            }
            None => break,
        }
    }

    (Ok(()), app.setup_complete)
}

// ── Event handler ─────────────────────────────────────────────────────────────

/// Returns `true` when the user has confirmed quit.
fn handle_event(
    app: &mut App,
    event: AppEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) -> bool {
    match event {
        AppEvent::Tick => {
            app.tick = app.tick.wrapping_add(1);
            // Expire flash: 5s for errors, 3s for info messages.
            if let Some((_, t, is_err)) = &app.flash {
                let ttl = if *is_err { 5 } else { 3 };
                if t.elapsed() >= Duration::from_secs(ttl) {
                    app.flash = None;
                }
            }
            // Auto-refresh positions every 30s while the Positions tab is active.
            // Skip if already loading, or if a modal (quit/help) is open.
            if app.active_tab == Tab::Positions
                && !app.loading
                && !matches!(
                    app.current_screen(),
                    Some(Screen::QuitConfirm) | Some(Screen::Help)
                )
                && app
                    .positions_refreshed_at
                    .is_some_and(|t| t.elapsed() >= Duration::from_secs(app.refresh_interval_secs))
            {
                app.loading = true;
                spawn_load_positions(Arc::clone(&client), tx.clone());
                spawn_load_orders(Arc::clone(&client), tx.clone());
            }

            // Hourly market snapshot.
            // First run: ~30 s after startup (if never run before in any session).
            // Subsequent runs: 1 h after the last completed snapshot (wall-clock).
            if !app.snapshot_in_progress {
                let should = match app.snapshot_last_at {
                    None => app.tick >= 600, // ~30 s at 50 ms/tick
                    Some(last) => (chrono::Utc::now() - last).num_seconds() >= 3600,
                };
                if should {
                    app.snapshot_in_progress = true;
                    app.snapshot_fetched_so_far = 0;
                    spawn_snapshot_markets(
                        Arc::clone(&client),
                        tx.clone(),
                        app.db_path.clone(),
                        app.known_resolved_ids.clone(),
                    );
                }
            }
        }

        AppEvent::Key(key) => {
            return handle_key(app, key, client, tx);
        }

        AppEvent::MarketsLoaded(markets, is_final) => {
            app.market_id_set = markets.iter().map(|m| m.condition_id.clone()).collect();
            app.markets = markets;
            app.rebuild_filter();
            app.loading = false;
            app.markets_loading_more = !is_final;
            app.last_error = None;
            if app.market_list_state.selected().is_none() && !app.markets.is_empty() {
                app.market_list_state.select(Some(0));
            }
            if app.active_tab == Tab::Analytics && !app.analytics_loading {
                app.analytics_loading = true;
                spawn_compute_analytics(
                    app.db_path.clone(),
                    tx.clone(),
                    Arc::clone(&client),
                    app.calibration_hours,
                );
            }
        }

        AppEvent::MarketsAppended(more, is_final) => {
            for m in more {
                // O(1) dedup via HashSet instead of O(n) linear scan.
                if app.market_id_set.insert(m.condition_id.clone()) {
                    app.markets.push(m);
                }
            }
            // Only pay the O(n log n) rebuild cost once per final batch,
            // not on every intermediate 100-market page.
            if is_final {
                app.rebuild_filter();
                app.markets_loading_more = false;
            }
        }

        AppEvent::MarketDetailLoaded(market, books) => {
            app.selected_market = Some(market);
            app.order_books = books;
            app.order_book_updated_at = Some(Instant::now());
            app.loading = false;
            app.last_error = None;
        }

        AppEvent::OrderBookUpdated(books) => {
            // Only apply if we're still on the detail/order screen
            if matches!(
                app.current_screen(),
                Some(Screen::MarketDetail) | Some(Screen::OrderEntry)
            ) {
                app.order_books = books;
                app.order_book_updated_at = Some(Instant::now());
            }
        }

        AppEvent::PositionsLoaded(mut positions) => {
            positions.sort_by(|a, b| b.current_price.total_cmp(&a.current_price));
            app.positions = positions;
            app.positions_refreshed_at = Some(Instant::now());
            app.last_error = None;
            if app.positions_list_state.selected().is_none() && !app.positions.is_empty() {
                app.positions_list_state.select(Some(0));
            }
        }

        AppEvent::OrdersLoaded(orders) => {
            app.orders = orders;
            app.loading = false;
            app.last_error = None;
            if app.orders_list_state.selected().is_none() && !app.orders.is_empty() {
                app.orders_list_state.select(Some(0));
            }
        }

        AppEvent::BalanceLoaded(balance, allowance) => {
            app.balance = Some(balance);
            app.allowance = Some(allowance);
            app.loading = false;
            app.last_error = None;
        }

        AppEvent::OrderPlaced(order_id) => {
            app.loading = false;
            app.last_error = None;
            app.set_flash(format!("Order placed: {}", order_id));
            spawn_load_orders(Arc::clone(&client), tx.clone());
        }

        AppEvent::OrderCancelled(order_id) => {
            app.loading = false;
            app.set_flash(format!("Cancelled: {}", order_id));
            spawn_load_orders(Arc::clone(&client), tx.clone());
        }

        AppEvent::Redeemed(tx_hash) => {
            app.loading = false;
            app.set_flash(format!("Redeemed! tx: {}", tx_hash));
            spawn_load_positions(Arc::clone(&client), tx.clone());
        }

        AppEvent::MarketPriceFetched(price) => {
            app.order_form.market_price = Some(price);
            app.order_form.market_price_failed = false;
        }

        AppEvent::FeeRateFetched(bps) => {
            app.order_form.fee_rate_bps = Some(bps);
        }

        AppEvent::Error(err) => {
            app.loading = false;
            // Mark the market price as failed if we were waiting for one in the order form.
            if app.order_form.market_order
                && app.order_form.market_price.is_none()
                && matches!(
                    app.current_screen(),
                    Some(Screen::OrderEntry) | Some(Screen::CloseConfirm)
                )
            {
                app.order_form.market_price_failed = true;
            }
            // Only flash non-auth errors — auth errors are shown persistently in the screen
            if !err.is_auth() {
                app.set_error_flash(format!("Error: {}", err));
            }
            app.last_error = Some(err);
        }

        AppEvent::PriceHistoryLoaded(condition_id, interval, data) => {
            let key = format!("{}:{}", condition_id, interval);
            app.price_history.insert(key, data);
        }

        AppEvent::SnapshotProgress(n) => {
            app.snapshot_fetched_so_far = n;
        }

        AppEvent::SnapshotComplete(n) => {
            let now = chrono::Utc::now();
            app.snapshot_in_progress = false;
            app.snapshot_last_at = Some(now);
            app.snapshot_last_count = n;
            app.snapshot_fetched_so_far = 0;
            app.snapshot_error = None;
            // Persist snapshot timing so the hourly schedule survives restarts.
            crate::persist::save_snapshot_meta(&crate::persist::SnapshotMeta {
                last_snapshot_at: Some(now.to_rfc3339()),
                last_snapshot_count: n,
            });
            app.set_flash(format!(
                "Snapshot complete: {} markets, {} new resolutions",
                n, app.resolutions_new_last_run
            ));
            // Re-run analytics if the dashboard has been viewed this session.
            if app.analytics_stats.is_some() || app.active_tab == Tab::Analytics {
                app.analytics_loading = true;
                spawn_compute_analytics(
                    app.db_path.clone(),
                    tx.clone(),
                    Arc::clone(&client),
                    app.calibration_hours,
                );
            }
        }

        AppEvent::AnalyticsComputed(stats) => {
            app.analytics_stats_prev = app.analytics_stats.take();
            app.analytics_stats = Some(*stats);
            app.analytics_loading = false;
            app.calibration_fetch_done = 0;
            app.calibration_fetch_total = 0;
        }

        AppEvent::CalibrationFetchProgress(done, total) => {
            app.calibration_fetch_done = done;
            app.calibration_fetch_total = total;
        }

        AppEvent::ResolutionsUpdated(new_count, new_ids) => {
            app.resolutions_new_last_run = new_count;
            for id in new_ids {
                app.known_resolved_ids.insert(id);
            }
        }

        AppEvent::SnapshotError(msg) => {
            app.snapshot_in_progress = false;
            app.snapshot_fetched_so_far = 0;
            app.snapshot_error = Some(msg.clone());
            app.set_error_flash(format!("Snapshot error: {}", msg));
        }

        AppEvent::AuthChecked(warning) => {
            app.auth_warning = warning;
        }

        AppEvent::SnapshotMetaLoaded(meta) => {
            app.snapshot_last_at = meta
                .last_snapshot_at
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&chrono::Utc));
            app.snapshot_last_count = meta.last_snapshot_count;
        }

        AppEvent::ResolvedIdsLoaded(ids) => {
            app.known_resolved_ids = ids;
        }
    }
    false
}

// ── Key handler ───────────────────────────────────────────────────────────────

fn handle_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) -> bool {
    // Ctrl+C always quits immediately.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }

    // c copies the visible error message to clipboard (no modifiers, error flash showing).
    if key.code == KeyCode::Char('c') && key.modifiers.is_empty() {
        if let Some((msg, _, true)) = &app.flash {
            let text = msg.strip_prefix("Error: ").unwrap_or(msg).to_string();
            copy_to_clipboard(&text);
            app.set_flash("Error copied to clipboard");
            return false;
        }
    }

    // Check for overlays first regardless of active tab
    if let Some(Screen::Setup) = app.current_screen() {
        return handle_setup_key(app, key);
    }
    if let Some(Screen::QuitConfirm) = app.current_screen() {
        return handle_quit_confirm_key(app, key);
    }
    if let Some(Screen::Help) = app.current_screen() {
        return handle_help_key(app, key);
    }
    // OrderEntry and CloseConfirm can be pushed from any tab.
    if let Some(Screen::OrderEntry) = app.current_screen() {
        handle_order_key(app, key, client, tx);
        return false;
    }
    if let Some(Screen::CloseConfirm) = app.current_screen() {
        return handle_close_confirm_key(app, key, client, tx);
    }
    if let Some(Screen::CancelAllConfirm) = app.current_screen() {
        handle_cancel_all_confirm_key(app, key, client, tx);
        return false;
    }
    if let Some(Screen::RedeemConfirm) = app.current_screen() {
        handle_redeem_confirm_key(app, key, client, tx);
        return false;
    }
    if let Some(Screen::RedeemAllConfirm) = app.current_screen() {
        handle_redeem_all_confirm_key(app, key, client, tx);
        return false;
    }

    // Global `/` — jump to Markets tab and activate search from any screen.
    if key.code == KeyCode::Char('/') && app.active_tab != Tab::Markets {
        switch_tab(app, Tab::Markets, Arc::clone(&client), tx);
        app.search_mode = true;
        app.search_query.clear();
        return false;
    }

    match &app.active_tab.clone() {
        Tab::Positions => {
            handle_positions_key(app, key, client, tx);
            false
        }
        Tab::Balance => {
            handle_balance_key(app, key, client, tx);
            false
        }
        Tab::Analytics => {
            handle_analytics_key(app, key, client, tx);
            false
        }
        Tab::Markets => match app.current_screen().cloned() {
            Some(Screen::MarketDetail) => {
                handle_detail_key(app, key, client, tx);
                false
            }
            _ => {
                handle_markets_key(app, key, client, tx);
                false
            }
        },
    }
}

// ── Global tab / navigation helpers ──────────────────────────────────────────

/// Returns true when an error originates from a missing-credentials condition.
/// Used to decide whether to show a persistent error panel vs. a transient flash.
pub fn is_auth_error(err: &crate::error::AppError) -> bool {
    err.is_auth()
}

fn switch_tab(app: &mut App, tab: Tab, client: Arc<PolyClient>, tx: &UnboundedSender<AppEvent>) {
    if app.active_tab == tab {
        return;
    }
    stop_ws(app); // disconnect WS when leaving detail screen via tab switch
    app.last_error = None; // clear stale errors from previous tab
    app.active_tab = tab.clone();
    app.screen_stack = match &tab {
        Tab::Markets => vec![Screen::MarketList],
        Tab::Positions => vec![Screen::MarketList], // reuse stack slot; render uses active_tab
        Tab::Balance => vec![Screen::MarketList],
        Tab::Analytics => vec![Screen::MarketList],
    };

    match tab {
        Tab::Markets => {
            if app.markets.is_empty() {
                app.loading = true;
                spawn_load_markets(client, tx.clone(), app.max_markets);
            }
        }
        Tab::Positions => {
            app.loading = true;
            spawn_load_positions(Arc::clone(&client), tx.clone());
            spawn_load_orders(client, tx.clone());
        }
        Tab::Balance => {
            app.loading = true;
            spawn_load_balance(client, tx.clone());
        }
        Tab::Analytics => {
            app.analytics_loading = true;
            spawn_compute_analytics(
                app.db_path.clone(),
                tx.clone(),
                Arc::clone(&client),
                app.calibration_hours,
            );
        }
    }
}

// ── Markets screen key handler ────────────────────────────────────────────────

fn handle_markets_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    if app.search_mode {
        match key.code {
            KeyCode::Esc => {
                app.search_mode = false;
            }
            KeyCode::Enter => {
                app.search_mode = false;
                app.market_list_state.select(Some(0));
            }
            KeyCode::Backspace => {
                app.search_query.pop();
                app.rebuild_filter();
            }
            KeyCode::Char(c) => {
                app.search_query.push(c);
                app.rebuild_filter();
                app.market_list_state.select(Some(0));
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('1') => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('2') => switch_tab(app, Tab::Positions, client, tx),
        KeyCode::Char('3') => switch_tab(app, Tab::Balance, client, tx),
        KeyCode::Char('4') => switch_tab(app, Tab::Analytics, client, tx),
        KeyCode::Tab => switch_tab(app, Tab::Positions, client, tx),
        KeyCode::Char('q') => {
            app.menu_index = 0;
            app.screen_stack.push(Screen::QuitConfirm);
        }
        KeyCode::Char('?') => {
            app.screen_stack.push(Screen::Help);
        }
        KeyCode::Char('/') => {
            app.search_mode = true;
            app.search_query.clear();
            app.rebuild_filter();
        }
        KeyCode::Char('r') => {
            app.loading = true;
            spawn_load_markets(client, tx.clone(), app.max_markets);
        }
        KeyCode::Char('s') => {
            app.sort_mode = app.sort_mode.next();
            app.rebuild_filter();
            app.market_list_state.select(Some(0));
            app.save_ui_state();
        }
        KeyCode::Char('d') => {
            app.date_filter = app.date_filter.next();
            app.rebuild_filter();
            app.market_list_state.select(Some(0));
            app.save_ui_state();
        }
        KeyCode::Char('p') => {
            app.prob_filter = app.prob_filter.next();
            app.rebuild_filter();
            app.market_list_state.select(Some(0));
            app.save_ui_state();
        }
        KeyCode::Char('v') => {
            app.volume_filter = app.volume_filter.next();
            app.rebuild_filter();
            app.market_list_state.select(Some(0));
            app.save_ui_state();
        }
        KeyCode::Char('c') => {
            let cats = app.cached_categories.clone();
            if !cats.is_empty() {
                app.category_filter = match &app.category_filter {
                    None => cats.into_iter().next(),
                    Some(current) => {
                        let idx = cats.iter().position(|c| c == current);
                        match idx {
                            Some(i) if i + 1 < cats.len() => Some(cats[i + 1].clone()),
                            _ => None,
                        }
                    }
                };
                app.rebuild_filter();
                app.market_list_state.select(Some(0));
                app.save_ui_state();
            }
        }
        KeyCode::Char('*') => {
            let filtered = app.filtered_markets();
            if let Some(idx) = app.market_list_state.selected() {
                if let Some(market) = filtered.get(idx) {
                    let cid = market.condition_id.clone();
                    if app.watchlist.contains(&cid) {
                        app.watchlist.remove(&cid);
                    } else {
                        app.watchlist.insert(cid);
                    }
                    crate::persist::save_watchlist(&app.watchlist);
                    // Rebuild so watchlist_only filter stays consistent
                    app.rebuild_filter();
                }
            }
        }
        KeyCode::Char('w') => {
            app.watchlist_only = !app.watchlist_only;
            app.rebuild_filter();
            app.market_list_state.select(Some(0));
        }
        KeyCode::Char('e') => {
            if app.watchlist.is_empty() {
                app.flash = Some((
                    "Watchlist is empty — star markets with *".to_string(),
                    std::time::Instant::now(),
                    false,
                ));
            } else {
                match crate::persist::export_watchlist(&app.watchlist, &app.markets) {
                    Ok(path) => {
                        app.flash = Some((
                            format!("Watchlist exported → {}", path.display()),
                            std::time::Instant::now(),
                            false,
                        ));
                    }
                    Err(e) => {
                        app.flash = Some((
                            format!("Export failed: {}", e),
                            std::time::Instant::now(),
                            true,
                        ));
                    }
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let filtered_len = app.filtered_markets().len();
            if filtered_len > 0 {
                let i = app.market_list_state.selected().unwrap_or(0);
                app.market_list_state
                    .select(Some((i + 1).min(filtered_len - 1)));
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.market_list_state.selected().unwrap_or(0);
            app.market_list_state.select(Some(i.saturating_sub(1)));
        }
        KeyCode::Enter => {
            let filtered = app.filtered_markets();
            if let Some(idx) = app.market_list_state.selected() {
                if let Some(market) = filtered.get(idx) {
                    let market = (*market).clone();
                    app.selected_market = None;
                    app.order_books.clear();
                    app.order_book_updated_at = None;
                    app.loading = true;
                    app.detail_outcome_index = 0;
                    app.screen_stack.push(Screen::MarketDetail);
                    let outcome_names: Vec<String> =
                        market.outcomes.iter().map(|o| o.name.clone()).collect();
                    let interval = app.sparkline_interval;
                    spawn_load_price_history(
                        Arc::clone(&client),
                        tx.clone(),
                        market.condition_id.clone(),
                        outcome_names,
                        interval,
                    );
                    spawn_load_detail(Arc::clone(&client), tx.clone(), market.clone());
                    // Start WebSocket feed for live order book updates.
                    stop_ws(app);
                    let token_pairs: Vec<(String, String)> = market
                        .outcomes
                        .iter()
                        .filter(|o| !o.token_id.is_empty())
                        .map(|o| (o.name.clone(), o.token_id.clone()))
                        .collect();
                    if !token_pairs.is_empty() {
                        let (cancel_tx, cancel_rx) = watch::channel(false);
                        app.ws_cancel = Some(cancel_tx);
                        spawn_ws_order_book(
                            Arc::clone(&client),
                            tx.clone(),
                            token_pairs,
                            cancel_rx,
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

// ── Market detail key handler ─────────────────────────────────────────────────

/// Signal the active WebSocket task to stop (if any).
fn stop_ws(app: &mut App) {
    if let Some(cancel) = app.ws_cancel.take() {
        let _ = cancel.send(true);
    }
}

fn handle_detail_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('h') => {
            stop_ws(app);
            app.screen_stack.pop();
        }
        KeyCode::Char('q') => {
            app.menu_index = 0;
            app.screen_stack.push(Screen::QuitConfirm);
        }
        KeyCode::Char('?') => {
            app.screen_stack.push(Screen::Help);
        }
        KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
            if let Some(market) = &app.selected_market {
                let n = market.outcomes.len();
                if n > 1 {
                    if key.code == KeyCode::Left {
                        app.detail_outcome_index = (app.detail_outcome_index + n - 1) % n;
                    } else {
                        app.detail_outcome_index = (app.detail_outcome_index + 1) % n;
                    }
                }
            }
        }
        KeyCode::Char('b') => {
            if let Some(market) = &app.selected_market {
                let idx = app
                    .detail_outcome_index
                    .min(market.outcomes.len().saturating_sub(1));
                if let Some(outcome) = market.outcomes.get(idx) {
                    let token_id = outcome.token_id.clone();
                    app.order_form = OrderForm {
                        side: Some(Side::Buy),
                        token_id: token_id.clone(),
                        outcome_name: outcome.name.clone(),
                        order_type: OrderType::Gtc,
                        market_order: true,
                        neg_risk: market.neg_risk,
                        ..Default::default()
                    };
                    app.screen_stack.push(Screen::OrderEntry);
                    spawn_fetch_market_price(
                        Arc::clone(&client),
                        tx.clone(),
                        token_id.clone(),
                        Side::Buy,
                    );
                    spawn_fetch_fee_rate(Arc::clone(&client), tx.clone(), token_id);
                }
            }
        }
        KeyCode::Char('s') => {
            if let Some(market) = &app.selected_market {
                let idx = app
                    .detail_outcome_index
                    .min(market.outcomes.len().saturating_sub(1));
                if let Some(outcome) = market.outcomes.get(idx) {
                    let token_id = outcome.token_id.clone();
                    app.order_form = OrderForm {
                        side: Some(Side::Sell),
                        token_id: token_id.clone(),
                        outcome_name: outcome.name.clone(),
                        order_type: OrderType::Gtc,
                        market_order: true,
                        neg_risk: market.neg_risk,
                        ..Default::default()
                    };
                    app.screen_stack.push(Screen::OrderEntry);
                    spawn_fetch_market_price(
                        Arc::clone(&client),
                        tx.clone(),
                        token_id.clone(),
                        Side::Sell,
                    );
                    spawn_fetch_fee_rate(Arc::clone(&client), tx.clone(), token_id);
                }
            }
        }
        KeyCode::Char('r') => {
            if let Some(market) = app.selected_market.clone() {
                app.order_books.clear();
                app.loading = true;
                let outcome_names: Vec<String> =
                    market.outcomes.iter().map(|o| o.name.clone()).collect();
                let interval = app.sparkline_interval;
                // Invalidate cached price history so it re-fetches
                let key = format!("{}:{}", market.condition_id, interval);
                app.price_history.remove(&key);
                spawn_load_price_history(
                    Arc::clone(&client),
                    tx.clone(),
                    market.condition_id.clone(),
                    outcome_names,
                    interval,
                );
                spawn_load_detail(client, tx.clone(), market);
            }
        }
        KeyCode::Char('t') => {
            // Toggle sparkline interval between 1d and 1w
            if let Some(market) = app.selected_market.clone() {
                app.sparkline_interval = if app.sparkline_interval == "1d" {
                    "1w"
                } else {
                    "1d"
                };
                let interval = app.sparkline_interval;
                let key = format!("{}:{}", market.condition_id, interval);
                if !app.price_history.contains_key(&key) {
                    let outcome_names: Vec<String> =
                        market.outcomes.iter().map(|o| o.name.clone()).collect();
                    spawn_load_price_history(
                        Arc::clone(&client),
                        tx.clone(),
                        market.condition_id.clone(),
                        outcome_names,
                        interval,
                    );
                }
            }
        }
        KeyCode::Char('c') => {
            if let Some(market) = &app.selected_market {
                let event_slug = if !market.group_slug.is_empty() {
                    &market.group_slug
                } else {
                    &market.slug
                };
                let url = format!("https://polymarket.com/event/{}", event_slug);
                copy_to_clipboard(&url);
                app.set_flash("Link copied to clipboard");
            }
        }
        KeyCode::Char('1') => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('2') => switch_tab(app, Tab::Positions, client, tx),
        KeyCode::Char('3') => switch_tab(app, Tab::Balance, client, tx),
        KeyCode::Char('4') => switch_tab(app, Tab::Analytics, client, tx),
        _ => {}
    }
}

// ── Order entry key handler ───────────────────────────────────────────────────

fn handle_order_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    match key.code {
        KeyCode::Esc => {
            app.screen_stack.pop();
        }
        KeyCode::Tab => {
            app.order_form.focused_field = (app.order_form.focused_field + 1) % 3;
        }
        KeyCode::BackTab => {
            app.order_form.focused_field = (app.order_form.focused_field + 2) % 3;
        }
        KeyCode::Char('d') => {
            app.order_form.dry_run = !app.order_form.dry_run;
        }
        KeyCode::Char('r') if app.order_form.market_order => {
            // Refresh market price
            app.order_form.market_price = None;
            app.order_form.market_price_failed = false;
            let token_id = app.order_form.token_id.clone();
            let side = app.order_form.side.clone().unwrap_or(Side::Buy);
            spawn_fetch_market_price(Arc::clone(&client), tx.clone(), token_id, side);
        }
        KeyCode::Char(' ') if app.order_form.focused_field == 2 => {
            if app.order_form.market_order {
                // Cycle back out of market mode to GTC
                app.order_form.market_order = false;
                app.order_form.market_price = None;
                app.order_form.order_type = OrderType::Gtc;
            } else {
                app.order_form.order_type = match app.order_form.order_type {
                    OrderType::Gtc => OrderType::Fok,
                    OrderType::Fok => OrderType::Ioc,
                    OrderType::Ioc => {
                        // Enter market mode: fetch best ask/bid
                        app.order_form.market_order = true;
                        app.order_form.market_price = None;
                        app.order_form.market_price_failed = false;
                        let token_id = app.order_form.token_id.clone();
                        let side = app.order_form.side.clone().unwrap_or(Side::Buy);
                        spawn_fetch_market_price(Arc::clone(&client), tx.clone(), token_id, side);
                        OrderType::Gtc // placeholder; market_order flag takes precedence
                    }
                };
            }
        }
        KeyCode::Backspace => match app.order_form.focused_field {
            0 => {
                app.order_form.size_input.pop();
            }
            1 => {
                app.order_form.price_input.pop();
            }
            _ => {}
        },
        KeyCode::Char(c) => match app.order_form.focused_field {
            0 => {
                if c.is_ascii_digit() || c == '.' {
                    app.order_form.size_input.push(c);
                }
            }
            1 => {
                if c.is_ascii_digit() || c == '.' {
                    app.order_form.price_input.push(c);
                }
            }
            _ => {}
        },
        KeyCode::Enter => {
            submit_order(app, client, tx);
        }
        _ => {}
    }
}

fn submit_order(app: &mut App, client: Arc<PolyClient>, tx: &UnboundedSender<AppEvent>) {
    let size: f64 = match app.order_form.size_input.parse() {
        Ok(v) => v,
        Err(_) => {
            app.set_flash("Invalid size");
            return;
        }
    };

    let (price, order_type) = if app.order_form.market_order {
        match app.order_form.market_price {
            Some(p) => (p, OrderType::Fok),
            None => {
                app.set_flash("Market price still loading — wait a moment");
                return;
            }
        }
    } else {
        let p: f64 = match app.order_form.price_input.parse() {
            Ok(v) => v,
            Err(_) => {
                app.set_flash("Invalid price");
                return;
            }
        };
        if p <= 0.0 || p >= 1.0 {
            app.set_flash("Price must be between 0.01 and 0.99");
            return;
        }
        (p, app.order_form.order_type)
    };

    if size < 5.0 {
        app.set_flash("Minimum size is 5 shares");
        return;
    }
    if size * price < 1.0 {
        app.set_flash("Minimum order value is $1.00");
        return;
    }

    let side = match &app.order_form.side {
        Some(s) => s.clone(),
        None => {
            app.set_flash("No side selected");
            return;
        }
    };

    if app.order_form.dry_run {
        let cost = size * price;
        let mode = if app.order_form.market_order {
            "MARKET "
        } else {
            ""
        };
        app.set_flash(format!(
            "DRY RUN — {}{} {} @ {:.4} (cost: ${:.4})",
            mode, side, size, price, cost
        ));
        app.screen_stack.pop();
        return;
    }

    app.loading = true;
    app.screen_stack.pop();
    spawn_place_order(
        client,
        tx.clone(),
        PlaceOrderParams {
            token_id: app.order_form.token_id.clone(),
            price,
            size,
            side,
            order_type,
            expiry: None,
            neg_risk: app.order_form.neg_risk,
        },
    );
}

// ── Positions key handler ─────────────────────────────────────────────────────

pub fn handle_positions_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    match key.code {
        KeyCode::Char('1') => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('2') => {} // already here
        KeyCode::Char('3') => switch_tab(app, Tab::Balance, client, tx),
        KeyCode::Char('4') => switch_tab(app, Tab::Analytics, client, tx),
        KeyCode::Tab => {
            app.positions_focus_orders = !app.positions_focus_orders;
        }
        KeyCode::Char('q') => {
            app.menu_index = 0;
            app.screen_stack.push(Screen::QuitConfirm);
        }
        KeyCode::Char('?') => {
            app.screen_stack.push(Screen::Help);
        }
        KeyCode::Char('r') => {
            app.loading = true;
            spawn_load_positions(Arc::clone(&client), tx.clone());
            spawn_load_orders(client, tx.clone());
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.positions_focus_orders {
                let len = app.orders.len();
                if len > 0 {
                    let i = app.orders_list_state.selected().unwrap_or(0);
                    app.orders_list_state.select(Some((i + 1).min(len - 1)));
                }
            } else {
                let len = app.positions.len();
                if len > 0 {
                    let i = app.positions_list_state.selected().unwrap_or(0);
                    app.positions_list_state.select(Some((i + 1).min(len - 1)));
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.positions_focus_orders {
                let i = app.orders_list_state.selected().unwrap_or(0);
                app.orders_list_state.select(Some(i.saturating_sub(1)));
            } else {
                let i = app.positions_list_state.selected().unwrap_or(0);
                app.positions_list_state.select(Some(i.saturating_sub(1)));
            }
        }
        KeyCode::Char('c') if app.positions_focus_orders => {
            if let Some(idx) = app.orders_list_state.selected() {
                if let Some(order) = app.orders.get(idx) {
                    let order_id = order.id.clone();
                    app.loading = true;
                    spawn_cancel_order(client, tx.clone(), order_id);
                }
            }
        }
        KeyCode::Char('C') if app.positions_focus_orders => {
            app.screen_stack.push(Screen::CancelAllConfirm);
        }
        KeyCode::Char('b') if !app.positions_focus_orders => {
            if let Some(idx) = app.positions_list_state.selected() {
                open_order_from_position(app, &client, tx, idx, Side::Buy, false);
            }
        }
        KeyCode::Char('s') if !app.positions_focus_orders => {
            if let Some(idx) = app.positions_list_state.selected() {
                open_order_from_position(app, &client, tx, idx, Side::Sell, false);
            }
        }
        KeyCode::Char('x') if !app.positions_focus_orders => {
            if let Some(idx) = app.positions_list_state.selected() {
                if let Some(pos) = app.positions.get(idx) {
                    let token_id = pos.token_id.clone();
                    app.close_confirm_pos_idx = Some(idx);
                    app.order_form = OrderForm {
                        side: Some(Side::Sell),
                        token_id: token_id.clone(),
                        outcome_name: pos.outcome.clone(),
                        size_input: format!("{:.2}", pos.size),
                        market_order: true,
                        close_position: true,
                        neg_risk: pos.neg_risk,
                        dry_run: app.order_form.dry_run,
                        ..Default::default()
                    };
                    app.screen_stack.push(Screen::CloseConfirm);
                    spawn_fetch_market_price(
                        Arc::clone(&client),
                        tx.clone(),
                        token_id.clone(),
                        Side::Sell,
                    );
                    spawn_fetch_fee_rate(Arc::clone(&client), tx.clone(), token_id);
                }
            }
        }
        // R — redeem highlighted position (only if redeemable)
        KeyCode::Char('R') if !app.positions_focus_orders => {
            if let Some(idx) = app.positions_list_state.selected() {
                if let Some(pos) = app.positions.get(idx) {
                    if pos.redeemable {
                        app.redeem_confirm_pos_idx = Some(idx);
                        app.screen_stack.push(Screen::RedeemConfirm);
                    } else {
                        app.set_error_flash("Position is not redeemable (market not resolved or outcome did not win)");
                    }
                }
            }
        }
        // A — redeem all redeemable positions
        KeyCode::Char('A') if !app.positions_focus_orders => {
            let count = app.positions.iter().filter(|p| p.redeemable).count();
            if count > 0 {
                app.screen_stack.push(Screen::RedeemAllConfirm);
            } else {
                app.set_error_flash("No redeemable positions found");
            }
        }
        _ => {}
    }
}

fn handle_close_confirm_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) -> bool {
    match key.code {
        KeyCode::Char('r') => {
            app.order_form.market_price = None;
            app.order_form.market_price_failed = false;
            let token_id = app.order_form.token_id.clone();
            let side = app.order_form.side.clone().unwrap_or(Side::Sell);
            spawn_fetch_market_price(Arc::clone(&client), tx.clone(), token_id, side);
            false
        }
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
            let price = match app.order_form.market_price {
                Some(p) => p,
                None if app.order_form.market_price_failed => {
                    app.set_flash("Price fetch failed — press r to retry");
                    return false;
                }
                None => {
                    app.set_flash("Market price still loading — wait a moment");
                    return false;
                }
            };
            let size: f64 = app.order_form.size_input.parse().unwrap_or(0.0);
            if size <= 0.0 {
                app.set_flash("Invalid position size");
                return false;
            }
            let token_id = app.order_form.token_id.clone();
            let neg_risk = app.order_form.neg_risk;
            if app.order_form.dry_run {
                app.set_flash(format!(
                    "DRY RUN — CLOSE {} shares of {} @ {:.4}",
                    size, app.order_form.outcome_name, price
                ));
                app.screen_stack.pop();
                return false;
            }
            app.loading = true;
            app.screen_stack.pop();
            spawn_place_order(
                client,
                tx.clone(),
                PlaceOrderParams {
                    token_id,
                    price,
                    size,
                    side: Side::Sell,
                    order_type: OrderType::Fok,
                    expiry: None,
                    neg_risk,
                },
            );
            false
        }
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
            app.screen_stack.pop();
            false
        }
        _ => false,
    }
}

fn open_order_from_position(
    app: &mut App,
    client: &Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
    pos_idx: usize,
    side: Side,
    close_position: bool,
) {
    let pos = match app.positions.get(pos_idx) {
        Some(p) => p,
        None => return,
    };
    let token_id = pos.token_id.clone();
    let outcome_name = pos.outcome.clone();
    let size = pos.size;
    let neg_risk = pos.neg_risk;

    // Pre-fill size for sell and close operations; leave blank for buy-more.
    let size_input = match side {
        Side::Sell => format!("{:.2}", size),
        Side::Buy => String::new(),
    };

    app.order_form = OrderForm {
        side: Some(side.clone()),
        token_id: token_id.clone(),
        outcome_name,
        size_input,
        order_type: OrderType::Gtc,
        market_order: true,
        close_position,
        neg_risk,
        // Cap size validation to shares held for sell/close operations.
        max_size: if matches!(side, Side::Sell) {
            Some(size)
        } else {
            None
        },
        ..Default::default()
    };
    app.screen_stack.push(Screen::OrderEntry);
    spawn_fetch_market_price(Arc::clone(client), tx.clone(), token_id.clone(), side);
    spawn_fetch_fee_rate(Arc::clone(client), tx.clone(), token_id);
}

// ── Balance key handler (called from ui.rs) ───────────────────────────────────

pub fn handle_balance_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    match key.code {
        KeyCode::Char('1') => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('2') => switch_tab(app, Tab::Positions, client, tx),
        KeyCode::Char('3') => {} // already here
        KeyCode::Char('4') => switch_tab(app, Tab::Analytics, client, tx),
        KeyCode::Tab => switch_tab(app, Tab::Analytics, client, tx),
        KeyCode::Char('r') => {
            app.loading = true;
            spawn_load_balance(client, tx.clone());
        }
        KeyCode::Char('q') => {
            app.menu_index = 0;
            app.screen_stack.push(Screen::QuitConfirm);
        }
        KeyCode::Char('?') => {
            app.screen_stack.push(Screen::Help);
        }
        _ => {}
    }
}

fn handle_analytics_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    match key.code {
        KeyCode::Char('s') => {
            app.analytics_panel_collapsed = !app.analytics_panel_collapsed;
        }
        KeyCode::Char('1') => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('2') => switch_tab(app, Tab::Positions, client, tx),
        KeyCode::Char('3') => switch_tab(app, Tab::Balance, client, tx),
        KeyCode::Char('4') => {} // already here
        KeyCode::Tab => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('r') => {
            if !app.analytics_loading {
                app.analytics_loading = true;
                spawn_compute_analytics(
                    app.db_path.clone(),
                    tx.clone(),
                    Arc::clone(&client),
                    app.calibration_hours,
                );
            }
        }
        KeyCode::Char('t') => {
            // Cycle calibration horizon: 3 → 6 → 9 → 12 → 3 hours.
            app.calibration_hours = match app.calibration_hours {
                3 => 6,
                6 => 9,
                9 => 12,
                _ => 3,
            };
            // Trigger a recompute so the chart updates immediately.
            if !app.analytics_loading {
                app.analytics_loading = true;
                spawn_compute_analytics(
                    app.db_path.clone(),
                    tx.clone(),
                    Arc::clone(&client),
                    app.calibration_hours,
                );
            }
        }
        KeyCode::Char('w') => {
            app.regression_weighted = !app.regression_weighted;
        }
        KeyCode::Char('p') => {
            if !app.snapshot_in_progress {
                app.snapshot_in_progress = true;
                app.snapshot_fetched_so_far = 0;
                spawn_snapshot_markets(
                    Arc::clone(&client),
                    tx.clone(),
                    app.db_path.clone(),
                    app.known_resolved_ids.clone(),
                );
            }
        }
        KeyCode::Char('c') => {
            let path = app.db_path.display().to_string();
            copy_to_clipboard(&path);
            app.set_flash("DB path copied to clipboard");
        }
        KeyCode::Char('o') => {
            if let Some(dir) = app.db_path.parent() {
                let dir = dir.to_path_buf();
                tokio::spawn(async move {
                    #[cfg(target_os = "macos")]
                    let _ = tokio::process::Command::new("open").arg(&dir).spawn();
                    #[cfg(target_os = "linux")]
                    let _ = tokio::process::Command::new("xdg-open").arg(&dir).spawn();
                    #[cfg(target_os = "windows")]
                    let _ = tokio::process::Command::new("explorer").arg(&dir).spawn();
                });
                app.set_flash("Opened snapshot folder");
            }
        }
        KeyCode::Char('q') => {
            app.menu_index = 0;
            app.screen_stack.push(Screen::QuitConfirm);
        }
        KeyCode::Char('?') => {
            app.screen_stack.push(Screen::Help);
        }
        _ => {}
    }
}

fn handle_cancel_all_confirm_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.screen_stack.pop();
            app.loading = true;
            spawn_cancel_all(client, tx.clone());
        }
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
            app.screen_stack.pop();
        }
        _ => {}
    }
}

fn handle_redeem_confirm_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(idx) = app.redeem_confirm_pos_idx {
                if let Some(pos) = app.positions.get(idx) {
                    let condition_id = pos.market_id.clone();
                    app.loading = true;
                    app.screen_stack.pop();
                    spawn_redeem_position(client, tx.clone(), condition_id);
                } else {
                    app.screen_stack.pop();
                }
            } else {
                app.screen_stack.pop();
            }
        }
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
            app.screen_stack.pop();
        }
        _ => {}
    }
}

fn handle_redeem_all_confirm_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
            let condition_ids: Vec<String> = app
                .positions
                .iter()
                .filter(|p| p.redeemable)
                .map(|p| p.market_id.clone())
                .collect();
            if !condition_ids.is_empty() {
                app.loading = true;
                app.screen_stack.pop();
                spawn_redeem_all(client, tx.clone(), condition_ids);
            } else {
                app.screen_stack.pop();
            }
        }
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
            app.screen_stack.pop();
        }
        _ => {}
    }
}

// ── Root menu (triggered by q) ────────────────────────────────────────────────

/// Returns the items for the root menu depending on navigation context.
/// Each entry is (label, key_hint, color).
pub fn root_menu_items(app: &App) -> Vec<(&'static str, &'static str, ratatui::style::Color)> {
    let can_go_back = app.screen_stack.len() >= 2
        && matches!(
            app.screen_stack.get(app.screen_stack.len() - 2),
            Some(Screen::MarketDetail) | Some(Screen::OrderEntry)
        );

    let mut items: Vec<(&'static str, &'static str, ratatui::style::Color)> =
        vec![("Quit", "q", theme::RED)];
    if can_go_back {
        items.push(("Back", "h", ratatui::style::Color::Rgb(100, 150, 220)));
    }
    items.push(("Setup", "s", ratatui::style::Color::Rgb(62, 224, 126)));
    items.push(("Help", "?", theme::CYAN));
    items.push(("Cancel", "Esc", ratatui::style::Color::Rgb(140, 140, 165)));
    items
}

fn execute_menu_item(app: &mut App, index: usize) -> bool {
    let items = root_menu_items(app);
    match items.get(index).map(|(label, ..)| *label) {
        Some("Quit") => return true,
        Some("Back") => {
            app.screen_stack.pop(); // remove QuitConfirm
            app.screen_stack.pop(); // go back one real screen
        }
        Some("Setup") => {
            app.screen_stack.pop();
            app.setup_form = screens::setup::SetupForm::default();
            app.screen_stack.push(Screen::Setup);
        }
        Some("Help") => {
            app.screen_stack.pop();
            app.screen_stack.push(Screen::Help);
        }
        Some("Cancel") => {
            app.screen_stack.pop();
        }
        _ => {}
    }
    false
}

fn handle_quit_confirm_key(app: &mut App, key: KeyEvent) -> bool {
    let n = root_menu_items(app).len();

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.menu_index = (app.menu_index + n - 1) % n;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.menu_index = (app.menu_index + 1) % n;
        }
        KeyCode::Enter => return execute_menu_item(app, app.menu_index),
        // Direct shortcuts
        KeyCode::Char('q') | KeyCode::Char('Q') => return true,
        KeyCode::Char('?') => {
            app.screen_stack.pop();
            app.screen_stack.push(Screen::Help);
        }
        KeyCode::Char('s') => {
            app.screen_stack.pop();
            app.setup_form = screens::setup::SetupForm::default();
            app.screen_stack.push(Screen::Setup);
        }
        KeyCode::Char('h') => {
            // Back — only acts if Back item is present
            if root_menu_items(app).iter().any(|(l, ..)| *l == "Back") {
                app.screen_stack.pop();
                app.screen_stack.pop();
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.screen_stack.pop();
        }
        _ => {}
    }
    false
}

// ── Setup wizard key handler ─────────────────────────────────────────────────

fn handle_setup_key(app: &mut App, key: KeyEvent) -> bool {
    use screens::setup::SetupStep;

    match key.code {
        KeyCode::Esc => {
            app.screen_stack.pop();
            app.setup_form = screens::setup::SetupForm::default();
        }
        KeyCode::Enter => {
            let done = app.setup_form.advance();
            if done {
                match app.setup_form.save() {
                    Ok(path) => {
                        app.screen_stack.pop();
                        app.set_flash(format!(
                            "Config saved to {}. Restart poly to apply.",
                            path.display()
                        ));
                        app.setup_complete = true;
                    }
                    Err(e) => {
                        app.setup_form.error = Some(format!("Failed to save: {}", e));
                    }
                }
            }
        }
        KeyCode::BackTab => {
            app.setup_form.go_back();
        }
        KeyCode::Backspace => {
            if app.setup_form.current_input().is_empty()
                && app.setup_form.step != SetupStep::PrivateKey
            {
                app.setup_form.go_back();
            } else {
                app.setup_form.backspace();
            }
        }
        KeyCode::Char(c) => {
            if app.setup_form.step != SetupStep::Confirm {
                app.setup_form.push_char(c);
            }
        }
        _ => {}
    }
    false
}

// ── Help overlay key handler ──────────────────────────────────────────────────

fn handle_help_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
            app.screen_stack.pop();
        }
        _ => {}
    }
    false
}

// ── Background task spawners ──────────────────────────────────────────────────

/// Maximum number of markets to load in total. Keeps memory bounded and
/// prevents O(n²) dedup + repeated O(n log n) rebuilds from degrading over time.
pub const MAX_MARKETS: usize = 5000;

pub fn spawn_load_markets(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>, max: usize) {
    tokio::spawn(async move {
        use futures_util::future::join_all;

        // 500 items/page × 4 concurrent pages = 2000 markets/round.
        // For the default cap of 5000: ⌈10 pages / 4⌉ = 3 HTTP rounds instead of 50.
        const PAGE: usize = 500;
        const CONCURRENCY: usize = 4;
        let mut offset = 0usize;
        let mut is_first = true;

        loop {
            let remaining = max.saturating_sub(offset);
            if remaining == 0 {
                break;
            }

            let n_pages = CONCURRENCY.min(remaining.div_ceil(PAGE));

            // Launch up to CONCURRENCY page fetches in parallel.
            let futs: Vec<_> = (0..n_pages)
                .map(|i| {
                    let c = Arc::clone(&client);
                    let off = offset + i * PAGE;
                    async move { c.get_markets_page(off, PAGE).await }
                })
                .collect();

            let results = join_all(futs).await;

            // Collect pages in order; stop at the first short page or error.
            let mut pages: Vec<Vec<Market>> = Vec::new();
            let mut reached_end = false;

            for result in results {
                match result {
                    Ok(page) => {
                        let fetched = page.len();
                        let short = fetched < PAGE;
                        offset += fetched;
                        pages.push(page);
                        if short || offset >= max {
                            reached_end = true;
                            break;
                        }
                    }
                    Err(AppError::Api { status: 500, .. }) => {
                        // Polymarket returns 500 (not an empty array) when paginating
                        // past the end of a filtered result set. Treat as end-of-data.
                        reached_end = true;
                        break;
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(e));
                        return;
                    }
                }
            }

            // Stream pages to the UI; mark the last one final when we know we're done.
            let n = pages.len();
            for (i, page) in pages.into_iter().enumerate() {
                let is_final = reached_end && i == n - 1;
                if is_first {
                    let _ = tx.send(AppEvent::MarketsLoaded(page, is_final));
                    is_first = false;
                } else {
                    let _ = tx.send(AppEvent::MarketsAppended(page, is_final));
                }
            }

            if reached_end {
                break;
            }
        }
    });
}

pub fn spawn_load_detail(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>, market: Market) {
    tokio::spawn(async move {
        let mut books = Vec::new();
        for outcome in &market.outcomes {
            if outcome.token_id.is_empty() {
                continue;
            }
            match client.get_order_book(&outcome.token_id).await {
                Ok(book) => books.push((outcome.name.clone(), book)),
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(e));
                    return;
                }
            }
        }
        let _ = tx.send(AppEvent::MarketDetailLoaded(market, books));
    });
}

pub fn spawn_load_price_history(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    condition_id: String,
    outcome_names: Vec<String>,
    interval: &'static str,
) {
    let fidelity = if interval == "1d" { 60 } else { 480 };
    tokio::spawn(async move {
        if let Ok(points) = client
            .get_price_history(&condition_id, interval, fidelity)
            .await
        {
            // The prices-history endpoint returns aggregate market prices.
            // We expose one series per market (labeled "Market") since the
            // data API doesn't break out per-outcome prices in this endpoint.
            let data = if outcome_names.len() == 2 {
                // Binary: Yes price = p, No price = 1 - p
                let yes: Vec<PricePoint> = points.clone();
                let no: Vec<PricePoint> = points.iter().map(|&(t, p)| (t, 1.0 - p)).collect();
                let yes_name = outcome_names
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Yes".to_string());
                let no_name = outcome_names
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| "No".to_string());
                vec![(yes_name, yes), (no_name, no)]
            } else {
                vec![("Price".to_string(), points)]
            };
            let _ = tx.send(AppEvent::PriceHistoryLoaded(
                condition_id,
                interval.to_string(),
                data,
            ));
        } // Err: silently ignore — sparkline is best-effort
    });
}

pub fn spawn_load_positions(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.get_positions().await {
            Ok(p) => {
                let _ = tx.send(AppEvent::PositionsLoaded(p));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(e));
            }
        }
    });
}

pub fn spawn_load_orders(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.get_open_orders().await {
            Ok(o) => {
                let _ = tx.send(AppEvent::OrdersLoaded(o));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(e));
            }
        }
    });
}

pub fn spawn_load_balance(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let balance = client.get_balance().await.unwrap_or(0.0);
        let allowance = client.get_allowance().await.unwrap_or(0.0);
        let _ = tx.send(AppEvent::BalanceLoaded(balance, allowance));
    });
}

pub fn spawn_redeem_position(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    condition_id: String,
) {
    tokio::spawn(async move {
        match client.redeem_position(&condition_id).await {
            Ok(tx_hash) => {
                let _ = tx.send(AppEvent::Redeemed(tx_hash));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(e));
            }
        }
    });
}

pub fn spawn_redeem_all(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    condition_ids: Vec<String>,
) {
    tokio::spawn(async move {
        let total = condition_ids.len();
        let mut succeeded = 0usize;
        let mut last_hash = String::new();
        for cid in &condition_ids {
            match client.redeem_position(cid).await {
                Ok(hash) => {
                    succeeded += 1;
                    last_hash = hash;
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(e));
                }
            }
        }
        if succeeded > 0 {
            let msg = if succeeded == total {
                format!(
                    "Redeemed {} position{} — last tx: {}",
                    succeeded,
                    if succeeded == 1 { "" } else { "s" },
                    last_hash
                )
            } else {
                format!(
                    "Redeemed {}/{} positions — last tx: {}",
                    succeeded, total, last_hash
                )
            };
            let _ = tx.send(AppEvent::Redeemed(msg));
        }
    });
}

// ── Analytics computation ─────────────────────────────────────────────────────

pub fn spawn_compute_analytics(
    db_path: std::path::PathBuf,
    tx: UnboundedSender<AppEvent>,
    client: Arc<PolyClient>,
    calibration_hours: u64,
) {
    tokio::spawn(async move {
        // ── Calibration price fetch ───────────────────────────────────────────
        // Fetch CLOB price-history for up to 200 resolved markets that don't yet
        // have a calibration price stored at the requested horizon.
        const CAL_BATCH: usize = 5_000;
        const CAL_CONCURRENCY: usize = 32;
        if let Ok(unpriced) = {
            let db = db_path.clone();
            let h = calibration_hours;
            tokio::task::spawn_blocking(move || {
                crate::db::open(&db)
                    .and_then(|c| crate::db::query_unpriced_resolutions(&c, h, CAL_BATCH))
            })
            .await
            .unwrap_or(Ok(vec![]))
        } {
            use futures_util::future::join_all;
            let total_unpriced = unpriced.len();
            let mut fetch_done = 0usize;
            if total_unpriced > 0 {
                let _ = tx.send(AppEvent::CalibrationFetchProgress(0, total_unpriced));
            }
            for chunk in unpriced.chunks(CAL_CONCURRENCY) {
                let futs: Vec<_> = chunk
                    .iter()
                    .map(|(cid, token_id, end_date)| {
                        let c = Arc::clone(&client);
                        let cid = cid.clone();
                        let token_id = token_id.clone();
                        let end_date = end_date.clone();
                        let hours = calibration_hours;
                        async move {
                            // Parse end_date to Unix timestamp.
                            let end_ts = end_date
                                .parse::<chrono::DateTime<chrono::Utc>>()
                                .map(|dt| dt.timestamp())
                                .unwrap_or(0);
                            if end_ts == 0 {
                                return None;
                            }
                            let price = c
                                .get_calibration_price(&token_id, end_ts, hours)
                                .await
                                .ok()
                                .flatten()?;
                            Some((cid, price))
                        }
                    })
                    .collect();

                let results = join_all(futs).await;
                fetch_done += chunk.len();
                let _ = tx.send(AppEvent::CalibrationFetchProgress(
                    fetch_done,
                    total_unpriced,
                ));

                let db = db_path.clone();
                let h = calibration_hours;
                let _: Result<rusqlite::Result<()>, _> = tokio::task::spawn_blocking(move || {
                    let conn = crate::db::open(&db)?;
                    for (cid, price) in results.into_iter().flatten() {
                        let _ = crate::db::update_calibration_price(&conn, &cid, price, h);
                    }
                    rusqlite::Result::Ok(())
                })
                .await;
            }
        }

        let stats = compute_analytics_stats(db_path, calibration_hours).await;
        let _ = tx.send(AppEvent::AnalyticsComputed(Box::new(stats)));
    });
}

async fn compute_analytics_stats(
    db_path: std::path::PathBuf,
    calibration_hours: u64,
) -> AnalyticsStats {
    tokio::task::spawn_blocking(move || {
        let conn = match crate::db::open(&db_path) {
            Ok(c) => c,
            Err(_) => return AnalyticsStats::default(),
        };

        let mut stats = AnalyticsStats::default();

        // Charts A, B: per-market data from the latest snapshot run.
        if let Ok(markets) = crate::db::query_latest_snapshot(&conn) {
            stats.total_markets = markets.len();
            for (_, volume, _liquidity, yes_price) in &markets {
                stats.total_volume += volume;
                if let Some(&price) = yes_price.as_ref() {
                    if (0.01..=0.99).contains(&price) {
                        let b20 = ((price * 20.0) as usize).min(19);
                        stats.prob_buckets[b20] += 1;
                    }
                }
            }
        }

        // Chart C: resolution bias.
        if let Ok((yes, no, other)) = crate::db::query_resolution_counts(&conn) {
            stats.res_yes = yes;
            stats.res_no = no;
            stats.res_other = other;
        }

        // Chart C: calibration curve.
        if let Ok(cal) = crate::db::query_calibration(&conn, calibration_hours) {
            stats.calibration = cal;
        }

        // Chart D: price accuracy vs market volume.
        if let Ok(evv) = crate::db::query_edge_vs_volume(&conn) {
            stats.edge_vs_vol = evv;
        }

        // Chart D: calibration fit per (category × volume tier).
        if let Ok(rows) = crate::db::query_calibration_raw(&conn, calibration_hours) {
            for (q, s, vol, yes_price, res) in rows {
                let cat_idx = match market_category_from_parts(&q, &s) {
                    Some("Politics") => 0,
                    Some("Sports") => 1,
                    Some("Crypto") => 2,
                    Some("Finance") => 3,
                    Some("Weather") => 4,
                    _ => 5,
                };
                let tier = if vol < 1_000.0 {
                    0
                } else if vol < 10_000.0 {
                    1
                } else if vol < 100_000.0 {
                    2
                } else if vol < 1_000_000.0 {
                    3
                } else {
                    4
                };
                let b = ((yes_price * 10.0) as usize).min(9);
                let cell = &mut stats.calibration_matrix[cat_idx][tier];
                cell.buckets[b].1 += 1;
                if res == "yes" {
                    cell.buckets[b].0 += 1;
                }
                cell.n += 1;
            }
        }

        // Keep high-confidence accuracy for potential future use.
        if let Ok((correct, wrong)) = crate::db::query_high_confidence_accuracy(&conn) {
            stats.hc_correct = correct;
            stats.hc_wrong = wrong;
        }

        stats
    })
    .await
    .unwrap_or_default()
}

pub fn spawn_snapshot_markets(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    db_path: std::path::PathBuf,
    known_resolved_ids: HashSet<String>,
) {
    tokio::spawn(async move {
        tracing::info!("starting market snapshot");
        use crate::db::{ResolutionRow, SnapshotRow};
        use futures_util::future::join_all;

        const PAGE: usize = 500;
        const CONCURRENCY: usize = 4;

        let snapshot_at = chrono::Utc::now().to_rfc3339();
        let mut offset = 0usize;
        let mut total_markets = 0usize;
        let mut snapshot_rows: Vec<SnapshotRow> = Vec::new();

        // ── Snapshot fetch loop ───────────────────────────────────────────────
        loop {
            let futs: Vec<_> = (0..CONCURRENCY)
                .map(|i| {
                    let c = Arc::clone(&client);
                    let off = offset + i * PAGE;
                    async move { c.get_markets_page_snapshot(off, PAGE).await }
                })
                .collect();

            let results = join_all(futs).await;
            let mut reached_end = false;

            for result in results {
                match result {
                    Ok(page) => {
                        let short = page.len() < PAGE;
                        offset += page.len();

                        for market in &page {
                            let cat = market_category(market).unwrap_or("").to_string();
                            let status = match market.status {
                                MarketStatus::Active => "Active",
                                MarketStatus::Closed => "Closed",
                                MarketStatus::Unknown => "Unknown",
                            };
                            let end_date = market.end_date.as_deref().unwrap_or("").to_string();
                            for outcome in &market.outcomes {
                                snapshot_rows.push(SnapshotRow {
                                    snapshot_at: snapshot_at.clone(),
                                    condition_id: market.condition_id.clone(),
                                    question: market.question.clone(),
                                    slug: market.slug.clone(),
                                    category: cat.clone(),
                                    status: status.to_string(),
                                    end_date: end_date.clone(),
                                    volume: market.volume,
                                    liquidity: market.liquidity,
                                    outcome: outcome.name.clone(),
                                    price: outcome.price,
                                });
                            }
                        }

                        total_markets += page.len();
                        let _ = tx.send(AppEvent::SnapshotProgress(total_markets));

                        if short {
                            reached_end = true;
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::SnapshotError(format!("Fetch error: {}", e)));
                        return;
                    }
                }
            }

            if reached_end {
                break;
            }
        }

        // ── Persist snapshot rows to DB ───────────────────────────────────────
        let db_p = db_path.clone();
        match tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
            let mut conn = crate::db::open(&db_p)?;
            crate::db::insert_snapshots(&mut conn, &snapshot_rows)
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = tx.send(AppEvent::SnapshotError(format!("DB write error: {}", e)));
                return;
            }
            Err(_) => {
                tracing::error!("snapshot DB task panicked");
                let _ = tx.send(AppEvent::SnapshotError("DB task panicked".to_string()));
                return;
            }
        }

        // ── Resolutions pass ──────────────────────────────────────────────────
        // Fetch recently-resolved markets from the API and buffer any we haven't
        // seen yet.  Deduplication against the full history is done via the DB's
        // PRIMARY KEY once we batch-insert below.
        let mut res_rows: Vec<ResolutionRow> = Vec::new();
        let mut seen_in_run: HashSet<String> = HashSet::new();

        // One-time backfill: if any resolved markets are missing their CLOB token ID
        // (e.g. they were stored before that column existed), we disable the early-stop
        // and include those known rows so insert_resolutions can UPDATE them via COALESCE.
        let needs_token_backfill = {
            let db_p = db_path.clone();
            tokio::task::spawn_blocking(move || -> bool {
                crate::db::open(&db_p)
                    .and_then(|c| {
                        c.query_row(
                            "SELECT COUNT(*) FROM resolutions WHERE clob_token_id IS NULL \
                         AND LOWER(resolution) IN ('yes','no')",
                            [],
                            |r| r.get::<_, i64>(0),
                        )
                    })
                    .map(|n| n > 0)
                    .unwrap_or(false)
            })
            .await
            .unwrap_or(false)
        };

        const RES_PAGE: usize = 500;
        const RES_MAX_PAGES: usize = 20; // up to 10 000 most-recently-resolved markets

        'res_loop: for page_idx in 0..RES_MAX_PAGES {
            let res_offset = page_idx * RES_PAGE;
            match client.get_resolved_markets_page(res_offset, RES_PAGE).await {
                Ok(page) => {
                    let is_last = page.len() < RES_PAGE;
                    let mut new_this_page = 0usize;
                    let mut backfill_this_page = 0usize;
                    for r in &page {
                        let is_new = !known_resolved_ids.contains(&r.condition_id)
                            && !seen_in_run.contains(&r.condition_id);
                        if is_new {
                            res_rows.push(ResolutionRow {
                                condition_id: r.condition_id.clone(),
                                question: r.question.clone(),
                                slug: r.slug.clone(),
                                end_date: r.end_date.as_deref().unwrap_or("").to_string(),
                                resolution: r.resolution.clone(),
                                last_trade_price: r.last_trade_price,
                                clob_token_id: r.clob_token_id.clone(),
                            });
                            seen_in_run.insert(r.condition_id.clone());
                            new_this_page += 1;
                        } else if needs_token_backfill && r.clob_token_id.is_some() {
                            // Known market but its clob_token_id may be NULL in the DB.
                            // INSERT OR IGNORE will skip the duplicate; the UPDATE COALESCE
                            // will fill in the token ID without touching other columns.
                            res_rows.push(ResolutionRow {
                                condition_id: r.condition_id.clone(),
                                question: r.question.clone(),
                                slug: r.slug.clone(),
                                end_date: r.end_date.as_deref().unwrap_or("").to_string(),
                                resolution: r.resolution.clone(),
                                last_trade_price: r.last_trade_price,
                                clob_token_id: r.clob_token_id.clone(),
                            });
                            backfill_this_page += 1;
                        }
                    }
                    // Early stop: a full page with no new data and no backfill work means
                    // we've caught up. When needs_token_backfill is true we keep paging
                    // until we've seen every market that might need its token ID filled in.
                    if is_last || (new_this_page == 0 && backfill_this_page == 0) {
                        break 'res_loop;
                    }
                }
                Err(e) => {
                    // Non-fatal — log but continue.
                    let _ = tx.send(AppEvent::SnapshotError(format!(
                        "Resolution fetch error: {}",
                        e
                    )));
                    break 'res_loop;
                }
            }
        }

        // Collect IDs before moving res_rows into spawn_blocking.
        let new_api_ids: Vec<String> = res_rows.iter().map(|r| r.condition_id.clone()).collect();
        let db_p = db_path.clone();
        let _ = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
            let mut conn = crate::db::open(&db_p)?;
            crate::db::insert_resolutions(&mut conn, &res_rows)
        })
        .await;

        // ── Cross-reference pass ──────────────────────────────────────────────
        // Query the DB for Closed markets that still lack a resolution — these
        // fell outside the 10 000-entry API window above.
        let db_p = db_path.clone();
        let mut candidates = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<_>> {
            let conn = crate::db::open(&db_p)?;
            crate::db::query_unresolved_closed(&conn)
        })
        .await
        .unwrap_or(Ok(vec![]))
        .unwrap_or_default();

        const XREF_CAP: usize = 2_000;
        const XREF_CONCURRENCY: usize = 16;

        candidates.sort_unstable_by(|a, b| {
            b.3.as_deref()
                .unwrap_or("")
                .cmp(a.3.as_deref().unwrap_or(""))
        });
        candidates.truncate(XREF_CAP);

        let mut xref_rows: Vec<ResolutionRow> = Vec::new();
        for chunk in candidates.chunks(XREF_CONCURRENCY) {
            let futs: Vec<_> = chunk
                .iter()
                .map(|(cid, _, _, _)| {
                    let c = Arc::clone(&client);
                    let cid = cid.clone();
                    async move { c.get_market_resolution(&cid).await }
                })
                .collect();

            for result in join_all(futs).await {
                if let Ok(Some(mr)) = result {
                    xref_rows.push(ResolutionRow {
                        condition_id: mr.condition_id.clone(),
                        question: mr.question.clone(),
                        slug: mr.slug.clone(),
                        end_date: mr.end_date.as_deref().unwrap_or("").to_string(),
                        resolution: mr.resolution.clone(),
                        last_trade_price: mr.last_trade_price,
                        clob_token_id: mr.clob_token_id.clone(),
                    });
                }
            }
        }

        let mut all_new_ids = new_api_ids;
        all_new_ids.extend(xref_rows.iter().map(|r| r.condition_id.clone()));
        let new_count = all_new_ids.len();

        let db_p = db_path.clone();
        let _ = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
            let mut conn = crate::db::open(&db_p)?;
            crate::db::insert_resolutions(&mut conn, &xref_rows)
        })
        .await;

        let _ = tx.send(AppEvent::ResolutionsUpdated(new_count, all_new_ids));
        tracing::info!(total_markets, "market snapshot complete");
        let _ = tx.send(AppEvent::SnapshotComplete(total_markets));
    });
}

/// Connects to the Polymarket WebSocket order book feed for the given token IDs.
/// Sends `AppEvent::OrderBookUpdated` whenever a `book` snapshot arrives.
/// Disconnects when the cancel receiver fires (or on error), then falls back to HTTP polling.
pub fn spawn_ws_order_book(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    token_ids: Vec<(String, String)>, // Vec<(outcome_name, token_id)>
    mut cancel: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        use futures_util::{SinkExt, StreamExt};
        use serde::Deserialize;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        const WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

        #[derive(Deserialize)]
        struct WsBookEntry {
            price: String,
            size: String,
        }

        #[derive(Deserialize)]
        struct WsBookMsg {
            #[serde(rename = "type")]
            msg_type: String,
            #[serde(default)]
            asset_id: String,
            #[serde(default)]
            bids: Vec<WsBookEntry>,
            #[serde(default)]
            asks: Vec<WsBookEntry>,
        }

        // Keep a cached version of each book so we can send the full list on each update.
        let mut book_cache: Vec<(String, OrderBook)> = token_ids
            .iter()
            .map(|(name, id)| {
                (
                    name.clone(),
                    OrderBook {
                        token_id: id.clone(),
                        bids: vec![],
                        asks: vec![],
                    },
                )
            })
            .collect();

        // WS phase: try to connect and stream updates until the connection drops or cancel fires.
        // Uses a labeled block so any exit (initial failure or mid-session drop) falls through
        // to the HTTP polling fallback below.
        'ws: {
            let ws_stream = match connect_async(WS_URL).await {
                Ok((stream, _)) => stream,
                Err(_) => break 'ws, // initial connect failed → go straight to HTTP fallback
            };

            let (mut write, mut read) = ws_stream.split();

            // Subscribe to all token IDs.
            let ids_json: Vec<String> = token_ids
                .iter()
                .map(|(_, id)| format!("\"{}\"", id))
                .collect();
            let subscribe = format!(
                r#"{{"assets_ids": [{}], "type": "market"}}"#,
                ids_json.join(", ")
            );
            if write.send(Message::Text(subscribe)).await.is_err() {
                break 'ws;
            }

            // Send a ping every 15 s so the server has a reason to reply even on
            // quiet markets.  A 30-second read timeout detects dead TCP: if nothing
            // arrives (including the expected pong) within that window we break to
            // the HTTP polling fallback.
            const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
            const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
            let mut ping_interval = tokio::time::interval(PING_INTERVAL);
            ping_interval.tick().await; // consume the immediate first tick

            loop {
                tokio::select! {
                    result = tokio::time::timeout(READ_TIMEOUT, read.next()) => {
                        match result {
                            Err(_) => break 'ws, // 30 s silence → dead TCP → HTTP fallback
                            Ok(msg) => match msg {
                                Some(Ok(Message::Text(text))) => {
                                    let msgs: Vec<WsBookMsg> =
                                        match serde_json::from_str::<Vec<WsBookMsg>>(&text)
                                            .or_else(|_| serde_json::from_str::<WsBookMsg>(&text).map(|m| vec![m]))
                                        {
                                            Ok(v) => v,
                                            Err(_) => break 'ws, // unrecognised frame → HTTP fallback
                                        };
                                    for m in msgs {
                                        if m.msg_type != "book" { continue; }
                                        let parse_levels = |entries: Vec<WsBookEntry>| -> Vec<crate::types::PriceLevel> {
                                            let mut levels: Vec<crate::types::PriceLevel> = entries
                                                .into_iter()
                                                .filter_map(|e| {
                                                    let price = e.price.parse().ok()?;
                                                    let size = e.size.parse().ok()?;
                                                    Some(crate::types::PriceLevel { price, size })
                                                })
                                                .collect();
                                            levels.sort_by(|a, b| b.price.total_cmp(&a.price));
                                            levels
                                        };
                                        let mut bids = parse_levels(m.bids);
                                        let mut asks = parse_levels(m.asks);
                                        bids.sort_by(|a, b| b.price.total_cmp(&a.price));
                                        asks.sort_by(|a, b| a.price.total_cmp(&b.price));

                                        if let Some(entry) = book_cache.iter_mut().find(|(_, b)| b.token_id == m.asset_id) {
                                            entry.1.bids = bids;
                                            entry.1.asks = asks;
                                        }
                                        let _ = tx.send(AppEvent::OrderBookUpdated(book_cache.clone()));
                                    }
                                }
                                Some(Ok(Message::Ping(d))) => { let _ = write.send(Message::Pong(d)).await; }
                                Some(Ok(Message::Pong(_))) => {} // keepalive reply — connection is alive
                                Some(Ok(_)) => {}
                                _ => break 'ws, // connection closed or error → HTTP fallback
                            }
                        }
                    }
                    _ = ping_interval.tick() => {
                        if write.send(Message::Ping(vec![])).await.is_err() {
                            break 'ws;
                        }
                    }
                    _ = cancel.changed() => {
                        if *cancel.borrow() { return; }
                    }
                }
            }
        }

        // HTTP polling fallback — reached on initial WS connect failure or mid-session disconnect.
        loop {
            if *cancel.borrow() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            if *cancel.borrow() {
                return;
            }
            let mut books = Vec::new();
            for (name, token_id) in &token_ids {
                if let Ok(book) = client.get_order_book(token_id).await {
                    books.push((name.clone(), book));
                }
            }
            if !books.is_empty() {
                let _ = tx.send(AppEvent::OrderBookUpdated(books));
            }
        }
    });
}

fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    if let Ok(mut child) = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

pub fn spawn_place_order(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    params: PlaceOrderParams,
) {
    tokio::spawn(async move {
        match client.place_order(&params).await {
            Ok(id) => {
                tracing::info!(order_id = %id, "order placed");
                let _ = tx.send(AppEvent::OrderPlaced(id));
            }
            Err(e) => {
                // Polymarket's CLOB sometimes returns 400 validation errors but still
                // executes the order. Wait briefly then do a precise match on
                // token_id + side + price + size to avoid false-positives.
                tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
                if let Ok(id) = client
                    .find_recent_order(&params.token_id, &params.side, params.price, params.size)
                    .await
                {
                    let _ = tx.send(AppEvent::OrderPlaced(id));
                } else {
                    copy_to_clipboard(&e.to_string());
                    let _ = tx.send(AppEvent::Error(e));
                }
            }
        }
    });
}

pub fn spawn_cancel_order(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    order_id: String,
) {
    tokio::spawn(async move {
        match client.cancel_order(&order_id).await {
            Ok(()) => {
                tracing::info!(order_id = %order_id, "order cancelled");
                let _ = tx.send(AppEvent::OrderCancelled(order_id));
            }
            Err(e) => {
                tracing::error!(order_id = %order_id, error = %e, "cancel failed");
                let _ = tx.send(AppEvent::Error(e));
            }
        }
    });
}

fn spawn_fetch_fee_rate(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>, token_id: String) {
    tokio::spawn(async move {
        // Default to 0: most active markets return 404 (no fee), only special
        // markets return a non-zero base_fee. This matches py-clob-client behaviour.
        let bps = client.get_fee_rate(&token_id).await.unwrap_or(0);
        let _ = tx.send(AppEvent::FeeRateFetched(bps));
    });
}

fn spawn_fetch_market_price(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    token_id: String,
    side: Side,
) {
    tokio::spawn(async move {
        match client.get_order_book(&token_id).await {
            Ok(book) => {
                let best = match side {
                    Side::Buy => book.asks.first().map(|l| l.price),
                    Side::Sell => book.bids.first().map(|l| l.price),
                };
                match best {
                    Some(p) => {
                        let _ = tx.send(AppEvent::MarketPriceFetched(p));
                    }
                    None => {
                        let _ = tx.send(AppEvent::Error(crate::error::AppError::Other(
                            "Order book empty — no market price available".into(),
                        )));
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(e));
            }
        }
    });
}

pub fn spawn_cancel_all(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.cancel_all_orders().await {
            Ok(()) => {
                let _ = tx.send(AppEvent::OrderCancelled("all".into()));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(e));
            }
        }
    });
}
