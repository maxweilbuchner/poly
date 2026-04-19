use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crossterm::event::KeyEvent;

use crate::types::{Market, Order, OrderBook, OrderType, OutcomeSeries, Position, Side};

use super::screens;

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
    CloseConfirm,      // fast-path: confirm closing a full position without the order form
    CancelAllConfirm,  // confirm cancelling all open orders
    RedeemConfirm,     // confirm on-chain redemption of a single resolved position
    RedeemAllConfirm,  // confirm on-chain redemption of all redeemable positions
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
    pub positions: Vec<crate::types::Position>,
    pub orders: Vec<crate::types::Order>,
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

    // User WebSocket channel — cancel signal and connection state
    pub user_ws_cancel: Option<tokio::sync::watch::Sender<bool>>,
    pub user_ws_connected: bool,

    // Root menu cursor position
    pub menu_index: usize,

    // Which position is being closed (set when CloseConfirm is pushed)
    pub close_confirm_pos_idx: Option<usize>,
    // Which position is being redeemed (set when RedeemConfirm is pushed)
    pub redeem_confirm_pos_idx: Option<usize>,

    // Selected outcome index in the detail screen
    pub detail_outcome_index: usize,
    /// Whether the description panel in market detail is expanded
    pub description_expanded: bool,

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

    /// IDs of orders that were live on the last refresh — used to detect fills.
    pub prev_live_order_ids: HashSet<String>,

    // Setup wizard form state
    pub setup_form: screens::setup::SetupForm,
    /// Set to true after setup completes — signals the event loop to exit for restart.
    pub setup_complete: bool,
}

/// Maximum number of markets to load in total. Keeps memory bounded and
/// prevents O(n²) dedup + repeated O(n log n) rebuilds from degrading over time.
pub const MAX_MARKETS: usize = 5000;

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
            user_ws_cancel: None,
            user_ws_connected: false,

            loading: false,
            markets_loading_more: false,
            last_error: None,

            menu_index: 0,
            close_confirm_pos_idx: None,
            redeem_confirm_pos_idx: None,
            detail_outcome_index: 0,
            description_expanded: false,
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

            prev_live_order_ids: HashSet::new(),

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

// ── TUI event enum ───────────────────────────────────────────────────────────

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

    /// User WebSocket channel: an order status changed (order_id, new_status).
    UserOrderUpdate(String, String),
    /// User WebSocket channel connected.
    UserWsConnected,
    /// User WebSocket channel disconnected (fell back to REST polling).
    UserWsDisconnected,
}
