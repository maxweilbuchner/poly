pub mod screens {
    pub mod balance;
    pub mod detail;
    pub mod markets;
    pub mod order;
    pub mod positions;
}

pub mod widgets {
    pub mod order_book;
    pub mod status_bar;
    pub mod tab_bar;
}

pub mod theme;
mod ui;

use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::client::{self, PolyClient};
use crate::types::{Market, Order, OrderBook, OrderType, Position, Side};

// ── Domain enums ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Tab {
    Markets,
    Positions,
    Balance,
}

#[derive(Debug, Clone)]
pub enum Screen {
    MarketList,
    MarketDetail,
    OrderEntry,
    Help,
    QuitConfirm,
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
}

impl OrderForm {
    pub fn cost(&self) -> Option<f64> {
        let size: f64 = self.size_input.parse().ok()?;
        let price: f64 = self.price_input.parse().ok()?;
        Some(size * price)
    }
}

// ── App events (from background tasks → main loop) ────────────────────────────

pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    MarketsLoaded(Vec<Market>),
    MarketDetailLoaded(Market, Vec<(String, OrderBook)>),
    PositionsLoaded(Vec<Position>),
    OrdersLoaded(Vec<Order>),
    BalanceLoaded(f64, f64),
    OrderPlaced(String),
    OrderCancelled(String),
    Error(String),
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

    // Flash message
    pub flash: Option<(String, Instant)>,

    // Order form
    pub order_form: OrderForm,

    // Loading / error
    pub loading: bool,
    pub last_error: Option<String>,

    // Spinner frame counter (incremented on each Tick)
    pub tick: u64,
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

            loading: false,
            last_error: None,

            tick: 0,
        }
    }

    pub fn set_flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now()));
    }

    pub fn current_screen(&self) -> Option<&Screen> {
        self.screen_stack.last()
    }

    /// Filtered market list based on current search query.
    pub fn filtered_markets(&self) -> Vec<&Market> {
        if self.search_query.is_empty() {
            self.markets.iter().collect()
        } else {
            let q = self.search_query.to_lowercase();
            self.markets
                .iter()
                .filter(|m| m.question.to_lowercase().contains(&q))
                .collect()
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(client: PolyClient) -> client::Result<()> {
    use crossterm::{
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{backend::CrosstermBackend, Terminal};
    use std::io;

    // Install panic hook to restore terminal before printing the panic message.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, client).await;

    // Always restore terminal, even on error.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_app(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    client: PolyClient,
) -> client::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let client = Arc::new(client);

    // Spawn input reader task.
    let tx_input = tx.clone();
    tokio::spawn(async move {
        loop {
            if crossterm::event::poll(Duration::from_millis(50)).unwrap_or(false) {
                if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read() {
                    let _ = tx_input.send(AppEvent::Key(k));
                }
            } else {
                let _ = tx_input.send(AppEvent::Tick);
            }
        }
    });

    let mut app = App::new();
    app.loading = true;
    spawn_load_markets(Arc::clone(&client), tx.clone());

    loop {
        terminal.draw(|f| ui::render(f, &mut app))?;

        match rx.recv().await {
            Some(event) => {
                if handle_event(&mut app, event, Arc::clone(&client), &tx) {
                    break;
                }
            }
            None => break,
        }
    }

    Ok(())
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
            // Expire flash after 3s.
            if let Some((_, t)) = &app.flash {
                if t.elapsed() >= Duration::from_secs(3) {
                    app.flash = None;
                }
            }
        }

        AppEvent::Key(key) => {
            return handle_key(app, key, client, tx);
        }

        AppEvent::MarketsLoaded(markets) => {
            app.markets = markets;
            app.loading = false;
            if app.market_list_state.selected().is_none() && !app.markets.is_empty() {
                app.market_list_state.select(Some(0));
            }
        }

        AppEvent::MarketDetailLoaded(market, books) => {
            app.selected_market = Some(market);
            app.order_books = books;
            app.loading = false;
        }

        AppEvent::PositionsLoaded(positions) => {
            app.positions = positions;
            if app.positions_list_state.selected().is_none() && !app.positions.is_empty() {
                app.positions_list_state.select(Some(0));
            }
        }

        AppEvent::OrdersLoaded(orders) => {
            app.orders = orders;
            app.loading = false;
            if app.orders_list_state.selected().is_none() && !app.orders.is_empty() {
                app.orders_list_state.select(Some(0));
            }
        }

        AppEvent::BalanceLoaded(balance, allowance) => {
            app.balance = Some(balance);
            app.allowance = Some(allowance);
            app.loading = false;
        }

        AppEvent::OrderPlaced(order_id) => {
            app.loading = false;
            app.set_flash(format!("Order placed: {}", order_id));
            // Refresh orders
            spawn_load_orders(Arc::clone(&client), tx.clone());
        }

        AppEvent::OrderCancelled(order_id) => {
            app.loading = false;
            app.set_flash(format!("Cancelled: {}", order_id));
            spawn_load_orders(Arc::clone(&client), tx.clone());
        }

        AppEvent::Error(msg) => {
            app.loading = false;
            app.last_error = Some(msg.clone());
            app.set_flash(format!("Error: {}", msg));
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

    // Check for overlays first regardless of active tab
    if let Some(Screen::QuitConfirm) = app.current_screen() {
        return handle_quit_confirm_key(app, key);
    }
    if let Some(Screen::Help) = app.current_screen() {
        return handle_help_key(app, key);
    }

    match &app.active_tab.clone() {
        Tab::Positions => { handle_positions_key(app, key, client, tx); false }
        Tab::Balance => { handle_balance_key(app, key, client, tx); false }
        Tab::Markets => match app.current_screen().cloned() {
            Some(Screen::OrderEntry) => { handle_order_key(app, key, client, tx); false }
            Some(Screen::MarketDetail) => { handle_detail_key(app, key, client, tx); false }
            _ => { handle_markets_key(app, key, client, tx); false }
        }
    }
}

// ── Global tab / navigation helpers ──────────────────────────────────────────

fn switch_tab(app: &mut App, tab: Tab, client: Arc<PolyClient>, tx: &UnboundedSender<AppEvent>) {
    if app.active_tab == tab {
        return;
    }
    app.active_tab = tab.clone();
    app.screen_stack = match &tab {
        Tab::Markets => vec![Screen::MarketList],
        Tab::Positions => vec![Screen::MarketList], // reuse stack slot; render uses active_tab
        Tab::Balance => vec![Screen::MarketList],
    };

    match tab {
        Tab::Markets => {
            if app.markets.is_empty() {
                app.loading = true;
                spawn_load_markets(client, tx.clone());
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
            }
            KeyCode::Char(c) => {
                app.search_query.push(c);
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
        KeyCode::Tab => switch_tab(app, Tab::Positions, client, tx),
        KeyCode::Char('q') => {
            app.screen_stack.push(Screen::QuitConfirm);
        }
        KeyCode::Char('?') => {
            app.screen_stack.push(Screen::Help);
        }
        KeyCode::Char('/') => {
            app.search_mode = true;
            app.search_query.clear();
        }
        KeyCode::Char('r') => {
            app.loading = true;
            spawn_load_markets(client, tx.clone());
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
                    app.loading = true;
                    app.screen_stack.push(Screen::MarketDetail);
                    spawn_load_detail(Arc::clone(&client), tx.clone(), market);
                }
            }
        }
        _ => {}
    }
}

// ── Market detail key handler ─────────────────────────────────────────────────

fn handle_detail_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('h') => {
            app.screen_stack.pop();
        }
        KeyCode::Char('q') => {
            app.screen_stack.push(Screen::QuitConfirm);
        }
        KeyCode::Char('?') => {
            app.screen_stack.push(Screen::Help);
        }
        KeyCode::Char('b') => {
            if let Some(market) = &app.selected_market {
                if let Some(outcome) = market.outcomes.first() {
                    app.order_form = OrderForm {
                        side: Some(Side::Buy),
                        token_id: outcome.token_id.clone(),
                        outcome_name: outcome.name.clone(),
                        order_type: OrderType::Gtc,
                        ..Default::default()
                    };
                    app.screen_stack.push(Screen::OrderEntry);
                }
            }
        }
        KeyCode::Char('s') => {
            if let Some(market) = &app.selected_market {
                if let Some(outcome) = market.outcomes.first() {
                    app.order_form = OrderForm {
                        side: Some(Side::Sell),
                        token_id: outcome.token_id.clone(),
                        outcome_name: outcome.name.clone(),
                        order_type: OrderType::Gtc,
                        ..Default::default()
                    };
                    app.screen_stack.push(Screen::OrderEntry);
                }
            }
        }
        KeyCode::Char('r') => {
            if let Some(market) = app.selected_market.clone() {
                app.order_books.clear();
                app.loading = true;
                spawn_load_detail(client, tx.clone(), market);
            }
        }
        KeyCode::Char('1') => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('2') => switch_tab(app, Tab::Positions, client, tx),
        KeyCode::Char('3') => switch_tab(app, Tab::Balance, client, tx),
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
            app.order_form.focused_field =
                (app.order_form.focused_field + 2) % 3;
        }
        KeyCode::Char('d') => {
            app.order_form.dry_run = !app.order_form.dry_run;
        }
        KeyCode::Char(' ') if app.order_form.focused_field == 2 => {
            app.order_form.order_type = match app.order_form.order_type {
                OrderType::Gtc => OrderType::Fok,
                OrderType::Fok => OrderType::Ioc,
                OrderType::Ioc => OrderType::Gtc,
            };
        }
        KeyCode::Backspace => match app.order_form.focused_field {
            0 => { app.order_form.size_input.pop(); }
            1 => { app.order_form.price_input.pop(); }
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
    let price: f64 = match app.order_form.price_input.parse() {
        Ok(v) => v,
        Err(_) => {
            app.set_flash("Invalid price");
            return;
        }
    };
    if price <= 0.0 || price >= 1.0 {
        app.set_flash("Price must be between 0.01 and 0.99");
        return;
    }
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
        app.set_flash(format!(
            "DRY RUN — {} {} @ {:.4} (cost: ${:.4})",
            side, size, price, cost
        ));
        app.screen_stack.pop();
        return;
    }

    app.loading = true;
    app.screen_stack.pop();
    spawn_place_order(
        client,
        tx.clone(),
        app.order_form.token_id.clone(),
        price,
        size,
        side,
        app.order_form.order_type.clone(),
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
        KeyCode::Tab => {
            app.positions_focus_orders = !app.positions_focus_orders;
        }
        KeyCode::Char('q') => {
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
            // Cancel all — confirmed via flash since no dedicated modal yet
            app.loading = true;
            spawn_cancel_all(client, tx.clone());
        }
        _ => {}
    }
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
        KeyCode::Tab => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('r') => {
            app.loading = true;
            spawn_load_balance(client, tx.clone());
        }
        KeyCode::Char('q') => {
            app.screen_stack.push(Screen::QuitConfirm);
        }
        KeyCode::Char('?') => {
            app.screen_stack.push(Screen::Help);
        }
        _ => {}
    }
}

// ── Quit confirm key handler ──────────────────────────────────────────────────

fn handle_quit_confirm_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => return true,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.screen_stack.pop();
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

pub fn spawn_load_markets(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.get_top_markets(50, None).await {
            Ok(m) => { let _ = tx.send(AppEvent::MarketsLoaded(m)); }
            Err(e) => { let _ = tx.send(AppEvent::Error(e.to_string())); }
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
                    let _ = tx.send(AppEvent::Error(e.to_string()));
                    return;
                }
            }
        }
        let _ = tx.send(AppEvent::MarketDetailLoaded(market, books));
    });
}

pub fn spawn_load_positions(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.get_positions().await {
            Ok(p) => { let _ = tx.send(AppEvent::PositionsLoaded(p)); }
            Err(e) => { let _ = tx.send(AppEvent::Error(e.to_string())); }
        }
    });
}

pub fn spawn_load_orders(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.get_open_orders().await {
            Ok(o) => { let _ = tx.send(AppEvent::OrdersLoaded(o)); }
            Err(e) => { let _ = tx.send(AppEvent::Error(e.to_string())); }
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

pub fn spawn_place_order(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    token_id: String,
    price: f64,
    size: f64,
    side: Side,
    order_type: OrderType,
) {
    tokio::spawn(async move {
        match client.place_order(&token_id, price, size, side, order_type, None).await {
            Ok(id) => { let _ = tx.send(AppEvent::OrderPlaced(id)); }
            Err(e) => { let _ = tx.send(AppEvent::Error(e.to_string())); }
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
            Ok(()) => { let _ = tx.send(AppEvent::OrderCancelled(order_id)); }
            Err(e) => { let _ = tx.send(AppEvent::Error(e.to_string())); }
        }
    });
}

pub fn spawn_cancel_all(client: Arc<PolyClient>, tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.cancel_all_orders().await {
            Ok(()) => { let _ = tx.send(AppEvent::OrderCancelled("all".into())); }
            Err(e) => { let _ = tx.send(AppEvent::Error(e.to_string())); }
        }
    });
}
