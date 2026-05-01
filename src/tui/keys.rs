use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;

use crate::client::PolyClient;
use crate::types::{OrderType, PlaceOrderParams, Side};

use super::screens;
use super::state::{App, AppEvent, OrderForm, Screen, Tab};
use super::tasks;
use super::theme;

pub(super) fn handle_key(
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
            tasks::copy_to_clipboard(&text);
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
    // (Viewer tab handles `/` locally for address input.)
    if key.code == KeyCode::Char('/')
        && app.active_tab != Tab::Markets
        && app.active_tab != Tab::Viewer
    {
        switch_tab(app, Tab::Markets, Arc::clone(&client), tx);
        app.search_mode = true;
        app.search_query.clear();
        return false;
    }

    // MarketDetail can be pushed from any tab (Markets or Positions).
    if let Some(Screen::MarketDetail) = app.current_screen() {
        handle_detail_key(app, key, client, tx);
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
        Tab::Markets => {
            handle_markets_key(app, key, client, tx);
            false
        }
        Tab::Viewer => {
            handle_viewer_key(app, key, client, tx);
            false
        }
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
                           // Refresh balance + positions when leaving the Balance tab so other tabs see fresh values.
    if app.active_tab == Tab::Balance {
        tasks::spawn_load_balance(Arc::clone(&client), tx.clone());
        tasks::spawn_load_positions(Arc::clone(&client), tx.clone());
    }
    app.active_tab = tab.clone();
    app.screen_stack = match &tab {
        Tab::Markets => vec![Screen::MarketList],
        Tab::Positions => vec![Screen::MarketList], // reuse stack slot; render uses active_tab
        Tab::Balance => vec![Screen::MarketList],
        Tab::Analytics => vec![Screen::MarketList],
        Tab::Viewer => vec![Screen::MarketList],
    };

    match tab {
        Tab::Markets => {
            if app.markets.is_empty() {
                app.loading = true;
                tasks::spawn_load_markets(client, tx.clone(), app.max_markets);
            }
        }
        Tab::Positions => {
            // Skip reload if already preloaded; auto-refresh will keep it fresh.
            if app.positions_refreshed_at.is_none() {
                app.loading = true;
                tasks::spawn_load_positions(Arc::clone(&client), tx.clone());
                tasks::spawn_load_orders(client, tx.clone());
            }
        }
        Tab::Balance => {
            if app.balance.is_none() {
                app.loading = true;
            }
            tasks::spawn_load_balance(Arc::clone(&client), tx.clone());
            tasks::spawn_load_positions(client, tx.clone());
        }
        Tab::Analytics => {
            // Only compute on first visit; cached stats are shown instantly on
            // subsequent switches.  User can press 'r' to force a refresh.
            if app.analytics_stats.is_none() && !app.analytics_loading {
                app.analytics_loading = true;
                tasks::spawn_compute_analytics(
                    app.db_path.clone(),
                    tx.clone(),
                    Arc::clone(&client),
                    app.calibration_hours,
                );
            }
        }
        Tab::Viewer => {
            // Activate address editing if no address has been entered yet.
            if app.viewer_address.is_none() && app.viewer_address_input.is_empty() {
                app.viewer_address_editing = true;
            }
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
        KeyCode::Char('5') => switch_tab(app, Tab::Viewer, client, tx),
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
            tasks::spawn_load_markets(client, tx.clone(), app.max_markets);
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
                    app.description_expanded = false;
                    app.screen_stack.push(Screen::MarketDetail);
                    let outcome_names: Vec<String> =
                        market.outcomes.iter().map(|o| o.name.clone()).collect();
                    let interval = app.sparkline_interval;
                    tasks::spawn_load_price_history(
                        Arc::clone(&client),
                        tx.clone(),
                        market.condition_id.clone(),
                        outcome_names,
                        interval,
                    );
                    tasks::spawn_load_detail(Arc::clone(&client), tx.clone(), market.clone());
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
                        tasks::spawn_ws_order_book(
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
pub(super) fn stop_ws(app: &mut App) {
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
        KeyCode::Char('e') => {
            app.description_expanded = !app.description_expanded;
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
                    tasks::spawn_fetch_market_price(
                        Arc::clone(&client),
                        tx.clone(),
                        token_id.clone(),
                        Side::Buy,
                    );
                    tasks::spawn_fetch_fee_rate(Arc::clone(&client), tx.clone(), token_id);
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
                    tasks::spawn_fetch_market_price(
                        Arc::clone(&client),
                        tx.clone(),
                        token_id.clone(),
                        Side::Sell,
                    );
                    tasks::spawn_fetch_fee_rate(Arc::clone(&client), tx.clone(), token_id);
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
                tasks::spawn_load_price_history(
                    Arc::clone(&client),
                    tx.clone(),
                    market.condition_id.clone(),
                    outcome_names,
                    interval,
                );
                tasks::spawn_load_detail(client, tx.clone(), market);
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
                    tasks::spawn_load_price_history(
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
                let url = if !market.group_slug.is_empty() {
                    // Event group market: append market slug for direct link
                    format!(
                        "https://polymarket.com/event/{}/{}",
                        market.group_slug, market.slug
                    )
                } else {
                    format!("https://polymarket.com/event/{}", market.slug)
                };
                tasks::copy_to_clipboard(&url);
                app.set_flash("Link copied to clipboard");
            }
        }
        KeyCode::Char('1') => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('2') => switch_tab(app, Tab::Positions, client, tx),
        KeyCode::Char('3') => switch_tab(app, Tab::Balance, client, tx),
        KeyCode::Char('4') => switch_tab(app, Tab::Analytics, client, tx),
        KeyCode::Char('5') => switch_tab(app, Tab::Viewer, client, tx),
        _ => {}
    }
}

// ── Order entry key handler ─────────────────────────────────────────────────

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
        KeyCode::Char('m') => {
            fill_max_size(app);
        }
        KeyCode::Char('r') if app.order_form.market_order => {
            // Refresh market price
            app.order_form.market_price = None;
            app.order_form.market_price_failed = false;
            let token_id = app.order_form.token_id.clone();
            let side = app.order_form.side.unwrap_or(Side::Buy);
            tasks::spawn_fetch_market_price(Arc::clone(&client), tx.clone(), token_id, side);
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
                        let side = app.order_form.side.unwrap_or(Side::Buy);
                        tasks::spawn_fetch_market_price(
                            Arc::clone(&client),
                            tx.clone(),
                            token_id,
                            side,
                        );
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
        Some(s) => *s,
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
    tasks::spawn_place_order(
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

/// Fill the size field with the maximum placeable order size.
/// Buy: cash balance ÷ (price × fee factor), floored to 2 decimals with a small margin.
/// Sell: the held-shares cap (max_size) set when opening from a position.
fn fill_max_size(app: &mut App) {
    let max_shares: Option<f64> = match app.order_form.side {
        Some(Side::Sell) => match app.order_form.max_size {
            Some(m) => Some((m * 100.0).floor() / 100.0),
            None => {
                app.set_flash("Max size unknown for this sell");
                return;
            }
        },
        Some(Side::Buy) => {
            let balance = match app.balance {
                Some(b) if b > 0.0 => b,
                _ => {
                    app.set_flash("Balance not loaded");
                    return;
                }
            };
            let price = if app.order_form.market_order {
                match app.order_form.market_price {
                    Some(p) => p,
                    None => {
                        app.set_flash("Market price still loading");
                        return;
                    }
                }
            } else {
                match app.order_form.price_input.parse::<f64>() {
                    Ok(p) if (0.01..=0.99).contains(&p) => p,
                    _ => {
                        app.set_flash("Enter a valid price first");
                        return;
                    }
                }
            };
            let rate = app.order_form.fee_rate_bps.unwrap_or(0) as f64 / 10_000.0;
            let effective_price = price * (1.0 + rate * (1.0 - price));
            // Leave ~0.1% headroom for fee rounding on the exchange side.
            Some((balance / effective_price * 0.999 * 100.0).floor() / 100.0)
        }
        None => {
            app.set_flash("No side selected");
            return;
        }
    };

    match max_shares {
        Some(size) if size >= 5.0 => {
            app.order_form.size_input = format!("{:.2}", size);
        }
        Some(_) => {
            app.set_flash("Max size below 5-share minimum");
        }
        None => {
            app.set_flash("Max size unavailable");
        }
    }
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
        KeyCode::Char('5') => switch_tab(app, Tab::Viewer, client, tx),
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
            tasks::spawn_load_positions(Arc::clone(&client), tx.clone());
            tasks::spawn_load_orders(client, tx.clone());
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
                    tasks::spawn_cancel_order(client, tx.clone(), order_id);
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
                    tasks::spawn_fetch_market_price(
                        Arc::clone(&client),
                        tx.clone(),
                        token_id.clone(),
                        Side::Sell,
                    );
                    tasks::spawn_fetch_fee_rate(Arc::clone(&client), tx.clone(), token_id);
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
        // Enter — open market detail for the selected position
        KeyCode::Enter if !app.positions_focus_orders => {
            if let Some(idx) = app.positions_list_state.selected() {
                if let Some(pos) = app.positions.get(idx) {
                    let condition_id = pos.market_id.clone();
                    let token_id = pos.token_id.clone();
                    app.selected_market = None;
                    app.order_books.clear();
                    app.order_book_updated_at = None;
                    app.loading = true;
                    app.detail_outcome_index = 0;
                    app.description_expanded = false;
                    app.screen_stack.push(Screen::MarketDetail);

                    // Try to find the market in the loaded list; otherwise fetch by ID.
                    let market = app
                        .markets
                        .iter()
                        .find(|m| m.condition_id == condition_id)
                        .cloned();

                    if let Some(market) = market {
                        let outcome_names: Vec<String> =
                            market.outcomes.iter().map(|o| o.name.clone()).collect();
                        let interval = app.sparkline_interval;
                        tasks::spawn_load_price_history(
                            Arc::clone(&client),
                            tx.clone(),
                            market.condition_id.clone(),
                            outcome_names,
                            interval,
                        );
                        tasks::spawn_load_detail(Arc::clone(&client), tx.clone(), market.clone());
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
                            tasks::spawn_ws_order_book(
                                Arc::clone(&client),
                                tx.clone(),
                                token_pairs,
                                cancel_rx,
                            );
                        }
                    } else {
                        // Market not in local list — fetch from API
                        // (falls back to question search for neg-risk markets)
                        let market_question = pos.market_question.clone();
                        tasks::spawn_load_detail_by_id(
                            Arc::clone(&client),
                            tx.clone(),
                            condition_id,
                            token_id,
                            market_question,
                        );
                    }
                }
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
            let side = app.order_form.side.unwrap_or(Side::Sell);
            tasks::spawn_fetch_market_price(Arc::clone(&client), tx.clone(), token_id, side);
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
            tasks::spawn_place_order(
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
        side: Some(side),
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
    tasks::spawn_fetch_market_price(Arc::clone(client), tx.clone(), token_id.clone(), side);
    tasks::spawn_fetch_fee_rate(Arc::clone(client), tx.clone(), token_id);
}

// ── Viewer key handler ───────────────────────────────────────────────────────

fn handle_viewer_key(
    app: &mut App,
    key: KeyEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) {
    if app.viewer_address_editing {
        match key.code {
            KeyCode::Esc => {
                app.viewer_address_editing = false;
                app.viewer_recent_selected = None;
                if let Some(addr) = &app.viewer_address {
                    app.viewer_address_input = addr.clone();
                }
            }
            KeyCode::Enter => {
                let addr = if let Some(i) = app.viewer_recent_selected {
                    app.viewer_recent.get(i).cloned().unwrap_or_default()
                } else {
                    app.viewer_address_input.trim().to_string()
                };
                if addr.starts_with("0x") && addr.len() == 42 {
                    app.viewer_address_editing = false;
                    app.viewer_recent_selected = None;
                    app.viewer_address = Some(addr.clone());
                    app.viewer_positions.clear();
                    app.viewer_list_state = ratatui::widgets::ListState::default();
                    app.loading = true;

                    app.viewer_recent.retain(|a| !a.eq_ignore_ascii_case(&addr));
                    app.viewer_recent.insert(0, addr.clone());
                    app.viewer_recent
                        .truncate(crate::persist::VIEWER_RECENT_MAX);
                    crate::persist::save_viewer_recent(&app.viewer_recent);

                    tasks::spawn_load_viewer_positions(Arc::clone(&client), tx.clone(), addr);
                } else {
                    app.set_error_flash("Invalid address — must be 0x followed by 40 hex chars");
                }
            }
            KeyCode::Up => {
                if !app.viewer_recent.is_empty() {
                    let next = match app.viewer_recent_selected {
                        None => 0,
                        Some(i) => (i + 1).min(app.viewer_recent.len() - 1),
                    };
                    app.viewer_recent_selected = Some(next);
                    app.viewer_address_input = app.viewer_recent[next].clone();
                }
            }
            KeyCode::Down => match app.viewer_recent_selected {
                Some(0) | None => {
                    app.viewer_recent_selected = None;
                    app.viewer_address_input.clear();
                }
                Some(i) => {
                    let new_i = i - 1;
                    app.viewer_recent_selected = Some(new_i);
                    app.viewer_address_input = app.viewer_recent[new_i].clone();
                }
            },
            KeyCode::Backspace => {
                app.viewer_recent_selected = None;
                app.viewer_address_input.pop();
            }
            KeyCode::Char(c) => {
                app.viewer_recent_selected = None;
                app.viewer_address_input.push(c);
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
        KeyCode::Char('5') => {} // already here
        KeyCode::Tab => switch_tab(app, Tab::Markets, client, tx),
        KeyCode::Char('q') => {
            app.menu_index = 0;
            app.screen_stack.push(Screen::QuitConfirm);
        }
        KeyCode::Char('?') => {
            app.screen_stack.push(Screen::Help);
        }
        KeyCode::Char('/') => {
            app.viewer_address_editing = true;
            app.viewer_address_input.clear();
        }
        KeyCode::Char('r') => {
            if let Some(addr) = app.viewer_address.clone() {
                app.viewer_positions.clear();
                app.loading = true;
                tasks::spawn_load_viewer_positions(Arc::clone(&client), tx.clone(), addr);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let len = app.viewer_positions.len();
            if len > 0 {
                let i = app.viewer_list_state.selected().unwrap_or(0);
                app.viewer_list_state.select(Some((i + 1).min(len - 1)));
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.viewer_list_state.selected().unwrap_or(0);
            app.viewer_list_state.select(Some(i.saturating_sub(1)));
        }
        KeyCode::Enter => {
            // Open market detail for the selected position
            if let Some(idx) = app.viewer_list_state.selected() {
                if let Some(pos) = app.viewer_positions.get(idx) {
                    let condition_id = pos.market_id.clone();
                    let token_id = pos.token_id.clone();
                    app.selected_market = None;
                    app.order_books.clear();
                    app.order_book_updated_at = None;
                    app.loading = true;
                    app.detail_outcome_index = 0;
                    app.description_expanded = false;
                    app.screen_stack.push(Screen::MarketDetail);

                    let market = app
                        .markets
                        .iter()
                        .find(|m| m.condition_id == condition_id)
                        .cloned();

                    if let Some(market) = market {
                        let outcome_names: Vec<String> =
                            market.outcomes.iter().map(|o| o.name.clone()).collect();
                        let interval = app.sparkline_interval;
                        tasks::spawn_load_price_history(
                            Arc::clone(&client),
                            tx.clone(),
                            market.condition_id.clone(),
                            outcome_names,
                            interval,
                        );
                        tasks::spawn_load_detail(Arc::clone(&client), tx.clone(), market.clone());
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
                            tasks::spawn_ws_order_book(
                                Arc::clone(&client),
                                tx.clone(),
                                token_pairs,
                                cancel_rx,
                            );
                        }
                    } else {
                        let market_question = pos.market_question.clone();
                        tasks::spawn_load_detail_by_id(
                            Arc::clone(&client),
                            tx.clone(),
                            condition_id,
                            token_id,
                            market_question,
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

// ── Balance key handler ──────────────────────────────────────────────────────

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
        KeyCode::Char('5') => switch_tab(app, Tab::Viewer, client, tx),
        KeyCode::Tab => switch_tab(app, Tab::Analytics, client, tx),
        KeyCode::Char('r') => {
            app.loading = true;
            tasks::spawn_load_balance(client, tx.clone());
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
        KeyCode::Char('5') => switch_tab(app, Tab::Viewer, client, tx),
        KeyCode::Tab => switch_tab(app, Tab::Viewer, client, tx),
        KeyCode::Char('r') => {
            if !app.analytics_loading {
                app.analytics_loading = true;
                tasks::spawn_compute_analytics(
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
                tasks::spawn_compute_analytics(
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
                tasks::spawn_snapshot_markets(
                    Arc::clone(&client),
                    tx.clone(),
                    app.db_path.clone(),
                    app.known_resolved_ids.clone(),
                );
            }
        }
        KeyCode::Char('c') => {
            let path = app.db_path.display().to_string();
            tasks::copy_to_clipboard(&path);
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
            tasks::spawn_cancel_all(client, tx.clone());
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
                    tasks::spawn_redeem_position(client, tx.clone(), condition_id);
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
                tasks::spawn_redeem_all(client, tx.clone(), condition_ids);
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

// ── Mouse handler ─────────────────────────────────────────────────────────────

pub(super) fn handle_mouse(
    app: &mut App,
    mouse: MouseEvent,
    client: Arc<PolyClient>,
    tx: &UnboundedSender<AppEvent>,
) -> bool {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            return handle_key(
                app,
                KeyEvent::new(KeyCode::Up, KeyModifiers::empty()),
                Arc::clone(&client),
                tx,
            );
        }
        MouseEventKind::ScrollDown => {
            return handle_key(
                app,
                KeyEvent::new(KeyCode::Down, KeyModifiers::empty()),
                Arc::clone(&client),
                tx,
            );
        }
        MouseEventKind::Up(MouseButton::Left) | MouseEventKind::Down(MouseButton::Left) => {
            if mouse.row == 0 && matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
                // Tab bar clicked:
                // " 1 Markets │ 2 Positions │ 3 Balance │ 4 Analytics │ 5 Viewer "
                //  012345678901234567890123456789012345678901234567890123456789012
                let c = mouse.column;
                let new_tab = if (1..=11).contains(&c) {
                    Some(Tab::Markets)
                } else if (13..=25).contains(&c) {
                    Some(Tab::Positions)
                } else if (27..=37).contains(&c) {
                    Some(Tab::Balance)
                } else if (39..=51).contains(&c) {
                    Some(Tab::Analytics)
                } else if (53..=62).contains(&c) {
                    Some(Tab::Viewer)
                } else {
                    None
                };

                if let Some(t) = new_tab {
                    switch_tab(app, t, client, tx);
                    return false;
                }
            }
        }
        _ => {}
    }
    false
}
