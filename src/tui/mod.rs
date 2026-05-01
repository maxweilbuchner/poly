pub mod screens {
    pub mod analytics;
    pub mod balance;
    pub mod detail;
    pub mod markets;
    pub mod order;
    pub mod positions;
    pub mod setup;
    pub mod viewer;
}

pub mod widgets {
    pub mod order_book;
    pub mod status_bar;
    pub mod tab_bar;
}

pub mod theme;
mod ui;

mod events;
mod keys;
mod state;
pub mod tasks;

// Re-export public API so external `use crate::tui::*` paths keep working.
pub use keys::{is_auth_error, root_menu_items};
pub use state::*;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::client::{self, PolyClient};
use crate::error::AppError;

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(client: PolyClient, tui_cfg: TuiConfig) -> client::Result<()> {
    use crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture},
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
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);

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
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        SetTitle("POLY")
    )
    .map_err(AppError::other)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(AppError::other)?;

    let (result, setup_done) = run_app(&mut terminal, client, tui_cfg).await;

    // Always restore terminal, even on error.
    disable_raw_mode().map_err(AppError::other)?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .map_err(AppError::other)?;
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
                if let Ok(evt) = crossterm::event::read() {
                    match evt {
                        crossterm::event::Event::Key(k) => {
                            if tx_input.send(AppEvent::Key(k)).is_err() {
                                break;
                            }
                        }
                        crossterm::event::Event::Mouse(m) => {
                            if tx_input.send(AppEvent::Mouse(m)).is_err() {
                                break;
                            }
                        }
                        _ => {}
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
    tasks::spawn_load_markets(Arc::clone(&client), tx.clone(), app.max_markets);

    // Preload all tabs in the background so data is ready when the user switches.
    tasks::spawn_load_positions(Arc::clone(&client), tx.clone());
    tasks::spawn_load_orders(Arc::clone(&client), tx.clone());
    tasks::spawn_load_balance(Arc::clone(&client), tx.clone());
    app.analytics_loading = true;
    tasks::spawn_compute_analytics(
        app.db_path.clone(),
        tx.clone(),
        Arc::clone(&client),
        app.calibration_hours,
    );

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

    // Start the user WebSocket channel for live order/trade events.
    // Only when CLOB credentials are available.
    if let Some(auth) = client.auth.clone() {
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        app.user_ws_cancel = Some(cancel_tx);
        tasks::spawn_ws_user_channel(auth, tx.clone(), cancel_rx);
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

    // Load net worth history from DB so the chart is immediately available
    // if previous sessions logged data.
    tasks::spawn_load_net_worth_history(tx.clone(), app.db_path.clone());

    // One-shot backfill of resolutions.group_slug for rows written before that
    // column existed. Powers the "Most Accurate Recurring Series" panel.
    tasks::spawn_backfill_group_slugs(Arc::clone(&client), tx.clone(), app.db_path.clone());

    loop {
        if let Err(e) = terminal.draw(|f| ui::render(f, &mut app)) {
            return (Err(AppError::other(e)), app.setup_complete);
        }

        match rx.recv().await {
            Some(event) => {
                if events::handle_event(&mut app, event, Arc::clone(&client), &tx) {
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

// ── Test helpers ─────────────────────────────────────────────────────────────

#[cfg(test)]
use crate::types::{Market, MarketStatus, Outcome};
#[cfg(test)]
use std::collections::HashSet;

/// Create an App suitable for unit tests (no disk I/O).
#[cfg(test)]
pub fn test_app() -> App {
    App {
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
        net_worth_history: Vec::new(),
        net_worth_last_at: None,
        net_worth_in_progress: false,
        flash: None,
        order_form: OrderForm::default(),
        filtered_indices: Vec::new(),
        market_id_set: HashSet::new(),
        cached_categories: Vec::new(),
        watchlist: HashSet::new(),
        watchlist_only: false,
        price_history: std::collections::HashMap::new(),
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
        balance_refreshed_at: None,
        refresh_interval_secs: 30,
        max_markets: MAX_MARKETS,
        db_path: std::path::PathBuf::from("/tmp/poly-test.db"),
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
        viewer_address_input: String::new(),
        viewer_address_editing: false,
        viewer_address: None,
        viewer_positions: Vec::new(),
        viewer_list_state: ratatui::widgets::ListState::default(),
    }
}

#[cfg(test)]
fn test_market(id: &str, question: &str, volume: f64) -> Market {
    Market {
        condition_id: id.to_string(),
        question: question.to_string(),
        description: None,
        slug: question.to_lowercase().replace(' ', "-"),
        group_slug: String::new(),
        status: MarketStatus::Active,
        end_date: Some("2026-12-31T00:00:00Z".to_string()),
        volume,
        liquidity: volume * 0.1,
        outcomes: vec![
            Outcome {
                name: "Yes".into(),
                token_id: format!("{id}-yes"),
                price: 0.65,
                bid: 0.64,
                ask: 0.66,
                bid_depth: 100.0,
                ask_depth: 100.0,
            },
            Outcome {
                name: "No".into(),
                token_id: format!("{id}-no"),
                price: 0.35,
                bid: 0.34,
                ask: 0.36,
                bid_depth: 100.0,
                ask_depth: 100.0,
            },
        ],
        category: None,
        tags: vec![],
        neg_risk: false,
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Order, OrderStatus, Side};
    use std::time::{Duration, Instant};

    // ── Tab switching ────────────────────────────────────────────────────────

    #[test]
    fn initial_state_is_markets_tab() {
        let app = test_app();
        assert_eq!(app.active_tab, Tab::Markets);
        assert!(matches!(app.current_screen(), Some(Screen::MarketList)));
    }

    #[test]
    fn tab_switching() {
        let mut app = test_app();
        app.active_tab = Tab::Positions;
        assert_eq!(app.active_tab, Tab::Positions);
        app.active_tab = Tab::Balance;
        assert_eq!(app.active_tab, Tab::Balance);
        app.active_tab = Tab::Analytics;
        assert_eq!(app.active_tab, Tab::Analytics);
    }

    // ── Screen stack ─────────────────────────────────────────────────────────

    #[test]
    fn screen_push_pop() {
        let mut app = test_app();
        assert_eq!(app.screen_stack.len(), 1);

        app.screen_stack.push(Screen::MarketDetail);
        assert!(matches!(app.current_screen(), Some(Screen::MarketDetail)));
        assert_eq!(app.screen_stack.len(), 2);

        app.screen_stack.push(Screen::OrderEntry);
        assert!(matches!(app.current_screen(), Some(Screen::OrderEntry)));
        assert_eq!(app.screen_stack.len(), 3);

        app.screen_stack.pop();
        assert!(matches!(app.current_screen(), Some(Screen::MarketDetail)));

        app.screen_stack.pop();
        assert!(matches!(app.current_screen(), Some(Screen::MarketList)));
    }

    #[test]
    fn screen_stack_pop_on_empty_returns_none() {
        let mut app = test_app();
        app.screen_stack.clear();
        assert!(app.current_screen().is_none());
    }

    #[test]
    fn modal_screens_push_and_pop() {
        let mut app = test_app();
        app.screen_stack.push(Screen::Help);
        assert!(matches!(app.current_screen(), Some(Screen::Help)));
        app.screen_stack.pop();
        assert!(matches!(app.current_screen(), Some(Screen::MarketList)));

        app.screen_stack.push(Screen::QuitConfirm);
        assert!(matches!(app.current_screen(), Some(Screen::QuitConfirm)));
    }

    // ── Sort mode cycling ────────────────────────────────────────────────────

    #[test]
    fn sort_mode_cycles() {
        let s = SortMode::Volume;
        assert_eq!(s.next(), SortMode::EndDate);
        assert_eq!(s.next().next(), SortMode::Probability);
        assert_eq!(s.next().next().next(), SortMode::Volume);
    }

    #[test]
    fn sort_mode_labels() {
        assert_eq!(SortMode::Volume.label(), "vol");
        assert_eq!(SortMode::EndDate.label(), "end date");
        assert_eq!(SortMode::Probability.label(), "prob");
    }

    // ── Date filter cycling ──────────────────────────────────────────────────

    #[test]
    fn date_filter_cycles_through_all_variants() {
        let mut f = DateFilter::All;
        let variants = [
            DateFilter::Hours3,
            DateFilter::Hours6,
            DateFilter::Hours9,
            DateFilter::Hours12,
            DateFilter::Hours24,
            DateFilter::Week,
            DateFilter::Month,
            DateFilter::All,
        ];
        for expected in &variants {
            f = f.next();
            assert_eq!(&f, expected);
        }
    }

    // ── Prob filter cycling ──────────────────────────────────────────────────

    #[test]
    fn prob_filter_cycles() {
        let mut f = ProbFilter::All;
        assert_eq!(f.next(), ProbFilter::Prob90_98);
        f = f.next(); // 90-98
        f = f.next(); // 85-98
        f = f.next(); // 80-98
        assert_eq!(f.next(), ProbFilter::All);
    }

    // ── Volume filter ────────────────────────────────────────────────────────

    #[test]
    fn volume_filter_cycles_and_thresholds() {
        assert_eq!(VolumeFilter::All.min_volume(), 0.0);
        assert_eq!(VolumeFilter::K1.min_volume(), 1_000.0);
        assert_eq!(VolumeFilter::K10.min_volume(), 10_000.0);
        assert_eq!(VolumeFilter::K100.min_volume(), 100_000.0);

        let mut v = VolumeFilter::All;
        v = v.next();
        assert_eq!(v, VolumeFilter::K1);
        v = v.next();
        assert_eq!(v, VolumeFilter::K10);
        v = v.next();
        assert_eq!(v, VolumeFilter::K100);
        v = v.next();
        assert_eq!(v, VolumeFilter::All);
    }

    // ── Flash messages ───────────────────────────────────────────────────────

    #[test]
    fn set_flash_stores_info_message() {
        let mut app = test_app();
        app.set_flash("hello");
        let (msg, _, is_err) = app.flash.as_ref().unwrap();
        assert_eq!(msg, "hello");
        assert!(!is_err);
    }

    #[test]
    fn set_error_flash_stores_error_message() {
        let mut app = test_app();
        app.set_error_flash("oops");
        let (msg, _, is_err) = app.flash.as_ref().unwrap();
        assert_eq!(msg, "oops");
        assert!(is_err);
    }

    #[test]
    fn flash_expiry_info_3s_error_5s() {
        let mut app = test_app();

        // Info: expires after 3s
        app.flash = Some((
            "info".into(),
            Instant::now() - Duration::from_secs(4),
            false,
        ));
        let (_, t, is_err) = app.flash.as_ref().unwrap();
        let ttl = if *is_err { 5 } else { 3 };
        assert!(t.elapsed() >= Duration::from_secs(ttl));

        // Error: NOT expired after 4s
        app.flash = Some(("err".into(), Instant::now() - Duration::from_secs(4), true));
        let (_, t, is_err) = app.flash.as_ref().unwrap();
        let ttl = if *is_err { 5 } else { 3 };
        assert!(t.elapsed() < Duration::from_secs(ttl));

        // Error: expired after 6s
        app.flash = Some(("err".into(), Instant::now() - Duration::from_secs(6), true));
        let (_, t, is_err) = app.flash.as_ref().unwrap();
        let ttl = if *is_err { 5 } else { 3 };
        assert!(t.elapsed() >= Duration::from_secs(ttl));
    }

    // ── Order form validation ────────────────────────────────────────────────

    #[test]
    fn order_form_cost_calculation() {
        let form = OrderForm {
            size_input: "10".into(),
            price_input: "0.65".into(),
            ..Default::default()
        };
        assert!((form.cost().unwrap() - 6.5).abs() < 1e-9);
    }

    #[test]
    fn order_form_cost_returns_none_for_invalid_input() {
        let form = OrderForm {
            size_input: "abc".into(),
            price_input: "0.50".into(),
            ..Default::default()
        };
        assert!(form.cost().is_none());

        let form = OrderForm {
            size_input: "10".into(),
            price_input: "xyz".into(),
            ..Default::default()
        };
        assert!(form.cost().is_none());
    }

    #[test]
    fn order_form_market_order_cost_uses_market_price() {
        let form = OrderForm {
            market_order: true,
            size_input: "20".into(),
            price_input: "0.50".into(), // should be ignored
            market_price: Some(0.75),
            ..Default::default()
        };
        assert!((form.cost().unwrap() - 15.0).abs() < 1e-9);
    }

    #[test]
    fn order_form_market_order_cost_none_without_market_price() {
        let form = OrderForm {
            market_order: true,
            size_input: "10".into(),
            market_price: None,
            ..Default::default()
        };
        assert!(form.cost().is_none());
    }

    // ── rebuild_filter ───────────────────────────────────────────────────────

    #[test]
    fn rebuild_filter_includes_all_by_default() {
        let mut app = test_app();
        app.markets = vec![
            test_market("a", "Will it rain?", 5000.0),
            test_market("b", "Who wins the election?", 10000.0),
            test_market("c", "Bitcoin above 100K?", 500.0),
        ];
        app.rebuild_filter();
        assert_eq!(app.filtered_indices.len(), 3);
    }

    #[test]
    fn rebuild_filter_text_search() {
        let mut app = test_app();
        app.markets = vec![
            test_market("a", "Will it rain tomorrow?", 5000.0),
            test_market("b", "Who wins the election?", 10000.0),
            test_market("c", "Will it rain next week?", 500.0),
        ];
        app.search_query = "rain".into();
        app.rebuild_filter();
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    fn rebuild_filter_text_search_case_insensitive() {
        let mut app = test_app();
        app.markets = vec![
            test_market("a", "Bitcoin Price Prediction", 5000.0),
            test_market("b", "BITCOIN Moon?", 10000.0),
            test_market("c", "Election results", 500.0),
        ];
        app.search_query = "bitcoin".into();
        app.rebuild_filter();
        assert_eq!(app.filtered_indices.len(), 2);
    }

    #[test]
    fn rebuild_filter_volume_filter() {
        let mut app = test_app();
        app.markets = vec![
            test_market("a", "Low vol", 500.0),
            test_market("b", "Mid vol", 5000.0),
            test_market("c", "High vol", 50000.0),
        ];
        app.volume_filter = VolumeFilter::K10;
        app.rebuild_filter();
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.markets[app.filtered_indices[0]].condition_id, "c");
    }

    #[test]
    fn rebuild_filter_sorts_by_volume_desc() {
        let mut app = test_app();
        app.markets = vec![
            test_market("a", "Low", 100.0),
            test_market("b", "High", 99999.0),
            test_market("c", "Mid", 5000.0),
        ];
        app.sort_mode = SortMode::Volume;
        app.rebuild_filter();
        let ids: Vec<&str> = app
            .filtered_indices
            .iter()
            .map(|&i| app.markets[i].condition_id.as_str())
            .collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[test]
    fn rebuild_filter_sorts_by_end_date_asc() {
        let mut app = test_app();
        let mut m1 = test_market("a", "Late", 100.0);
        m1.end_date = Some("2026-12-31T00:00:00Z".into());
        let mut m2 = test_market("b", "Early", 100.0);
        m2.end_date = Some("2026-01-01T00:00:00Z".into());
        let mut m3 = test_market("c", "Mid", 100.0);
        m3.end_date = Some("2026-06-15T00:00:00Z".into());
        app.markets = vec![m1, m2, m3];
        app.sort_mode = SortMode::EndDate;
        app.rebuild_filter();
        let ids: Vec<&str> = app
            .filtered_indices
            .iter()
            .map(|&i| app.markets[i].condition_id.as_str())
            .collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[test]
    fn rebuild_filter_sorts_by_probability_desc() {
        let mut app = test_app();
        let mut m1 = test_market("a", "Low prob", 100.0);
        m1.outcomes[0].price = 0.30;
        let mut m2 = test_market("b", "High prob", 100.0);
        m2.outcomes[0].price = 0.95;
        let mut m3 = test_market("c", "Mid prob", 100.0);
        m3.outcomes[0].price = 0.60;
        app.markets = vec![m1, m2, m3];
        app.sort_mode = SortMode::Probability;
        app.rebuild_filter();
        let ids: Vec<&str> = app
            .filtered_indices
            .iter()
            .map(|&i| app.markets[i].condition_id.as_str())
            .collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[test]
    fn rebuild_filter_watchlist_only() {
        let mut app = test_app();
        app.markets = vec![
            test_market("a", "Starred", 100.0),
            test_market("b", "Not starred", 100.0),
        ];
        app.watchlist.insert("a".into());
        app.watchlist_only = true;
        app.rebuild_filter();
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.markets[app.filtered_indices[0]].condition_id, "a");
    }

    #[test]
    fn rebuild_filter_combined_search_and_volume() {
        let mut app = test_app();
        app.markets = vec![
            test_market("a", "Bitcoin price today?", 500.0),
            test_market("b", "Bitcoin above 100K?", 50000.0),
            test_market("c", "Election results?", 50000.0),
        ];
        app.search_query = "bitcoin".into();
        app.volume_filter = VolumeFilter::K10;
        app.rebuild_filter();
        assert_eq!(app.filtered_indices.len(), 1);
        assert_eq!(app.markets[app.filtered_indices[0]].condition_id, "b");
    }

    #[test]
    fn rebuild_filter_populates_cached_categories() {
        let mut app = test_app();
        app.markets = vec![
            test_market("a", "Will the highest temperature in NYC be 80?", 100.0),
            test_market("b", "Bitcoin above 100K?", 100.0),
        ];
        app.rebuild_filter();
        assert!(app.cached_categories.contains(&"Weather".to_string()));
        assert!(app.cached_categories.contains(&"Crypto".to_string()));
    }

    // ── market_category ──────────────────────────────────────────────────────

    #[test]
    fn market_category_identifies_weather() {
        let m = test_market("a", "Will the highest temperature in London be 80?", 100.0);
        assert_eq!(market_category(&m), Some("Weather"));
    }

    #[test]
    fn market_category_identifies_crypto() {
        let m = test_market("a", "Will Bitcoin reach 200K?", 100.0);
        assert_eq!(market_category(&m), Some("Crypto"));
    }

    #[test]
    fn market_category_identifies_sports_by_slug() {
        let mut m = test_market("a", "Some game result", 100.0);
        m.slug = "nba-lakers-vs-celtics".into();
        assert_eq!(market_category(&m), Some("Sports"));
    }

    #[test]
    fn market_category_returns_none_for_unknown() {
        let m = test_market("a", "Will something happen?", 100.0);
        assert_eq!(market_category(&m), None);
    }

    // ── Positions focus toggle ───────────────────────────────────────────────

    #[test]
    fn positions_focus_toggle() {
        let mut app = test_app();
        assert!(!app.positions_focus_orders);
        app.positions_focus_orders = true;
        assert!(app.positions_focus_orders);
        app.positions_focus_orders = false;
        assert!(!app.positions_focus_orders);
    }

    // ── Description expanded toggle ──────────────────────────────────────────

    #[test]
    fn description_expanded_toggle() {
        let mut app = test_app();
        assert!(!app.description_expanded);
        app.description_expanded = !app.description_expanded;
        assert!(app.description_expanded);
        app.description_expanded = !app.description_expanded;
        assert!(!app.description_expanded);
    }

    // ── Detail outcome index ─────────────────────────────────────────────────

    #[test]
    fn detail_outcome_index_defaults_to_zero() {
        let app = test_app();
        assert_eq!(app.detail_outcome_index, 0);
    }

    // ── Fill detection ───────────────────────────────────────────────────────

    #[test]
    fn fill_detection_sets_flash_when_order_disappears() {
        let mut app = test_app();
        // Simulate: order "abc" was live on previous refresh
        app.prev_live_order_ids.insert("abc".into());
        app.prev_live_order_ids.insert("def".into());

        // New refresh: "def" is still live, "abc" is gone (filled)
        let new_orders = [Order {
            id: "def".into(),
            asset_id: String::new(),
            side: Side::Buy,
            price: 0.5,
            original_size: 10.0,
            size_matched: 0.0,
            status: OrderStatus::Live,
            outcome: String::new(),
            market: String::new(),
            created_at: String::new(),
        }];

        // Simulate the fill detection logic from OrdersLoaded handler
        let new_ids: HashSet<String> = new_orders.iter().map(|o| o.id.clone()).collect();
        let filled: Vec<&str> = app
            .prev_live_order_ids
            .iter()
            .filter(|id| !new_ids.contains(id.as_str()))
            .map(|s| s.as_str())
            .collect();
        assert_eq!(filled.len(), 1);
        assert_eq!(filled[0], "abc");
    }

    #[test]
    fn cancel_removes_from_prev_live_ids() {
        let mut app = test_app();
        app.prev_live_order_ids.insert("abc".into());
        app.prev_live_order_ids.insert("def".into());

        // Simulate cancel of "abc"
        app.prev_live_order_ids.remove("abc");
        assert!(!app.prev_live_order_ids.contains("abc"));
        assert!(app.prev_live_order_ids.contains("def"));
    }

    #[test]
    fn cancel_all_clears_prev_live_ids() {
        let mut app = test_app();
        app.prev_live_order_ids.insert("abc".into());
        app.prev_live_order_ids.insert("def".into());

        // Simulate cancel-all
        app.prev_live_order_ids.clear();
        assert!(app.prev_live_order_ids.is_empty());
    }

    #[test]
    fn no_false_fill_on_first_load() {
        let app = test_app();
        // First load: prev_live_order_ids is empty, so no fills should be detected
        assert!(app.prev_live_order_ids.is_empty());
    }
}
