use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::client::PolyClient;

use super::state::{App, AppEvent, Screen, Tab};
use super::tasks;

/// Returns `true` when the user has confirmed quit.
pub(super) fn handle_event(
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
            // Auto-refresh positions via REST polling.
            // When the user WebSocket is connected, fills trigger instant refreshes,
            // so we only need a slow safety-net poll (4× the configured interval).
            // Without WS, keep the normal interval.
            let poll_secs = if app.user_ws_connected {
                app.refresh_interval_secs * 4
            } else {
                app.refresh_interval_secs
            };
            if app.active_tab == Tab::Positions
                && !app.loading
                && !matches!(
                    app.current_screen(),
                    Some(Screen::QuitConfirm) | Some(Screen::Help)
                )
                && app
                    .positions_refreshed_at
                    .is_some_and(|t| t.elapsed() >= Duration::from_secs(poll_secs))
            {
                app.loading = true;
                tasks::spawn_load_positions(Arc::clone(&client), tx.clone());
                tasks::spawn_load_orders(Arc::clone(&client), tx.clone());
            }

            // Balance tab: refresh balance + positions every 2s while visible.
            if app.active_tab == Tab::Balance
                && !matches!(
                    app.current_screen(),
                    Some(Screen::QuitConfirm) | Some(Screen::Help)
                )
                && app
                    .balance_refreshed_at
                    .is_some_and(|t| t.elapsed() >= Duration::from_secs(2))
            {
                tasks::spawn_load_balance(Arc::clone(&client), tx.clone());
                tasks::spawn_load_positions(Arc::clone(&client), tx.clone());
                // Reset timer optimistically so we don't double-spawn before reply arrives.
                app.balance_refreshed_at = Some(Instant::now());
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
                    tasks::spawn_snapshot_markets(
                        Arc::clone(&client),
                        tx.clone(),
                        app.db_path.clone(),
                        app.known_resolved_ids.clone(),
                    );
                }
            }

            // 10-minute net worth logging.
            // First run: ~30 s after startup.
            // Subsequent runs: 10 min after the last completed log (wall-clock).
            if !app.net_worth_in_progress {
                let should = match app.net_worth_last_at {
                    None => app.tick >= 600, // ~30 s at 50 ms/tick
                    Some(last) => (chrono::Utc::now() - last).num_seconds() >= 600,
                };
                if should {
                    app.net_worth_in_progress = true;
                    tasks::spawn_log_net_worth(
                        Arc::clone(&client),
                        tx.clone(),
                        app.db_path.clone(),
                    );
                }
            }
        }

        AppEvent::Key(key) => {
            return super::keys::handle_key(app, key, client, tx);
        }

        AppEvent::Mouse(mouse) => {
            return super::keys::handle_mouse(app, mouse, client, tx);
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
            // Start WS + sparklines if not already running (e.g. opened from positions tab).
            if app.ws_cancel.is_none() {
                let token_pairs: Vec<(String, String)> = market
                    .outcomes
                    .iter()
                    .filter(|o| !o.token_id.is_empty())
                    .map(|o| (o.name.clone(), o.token_id.clone()))
                    .collect();
                if !token_pairs.is_empty() {
                    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
                    app.ws_cancel = Some(cancel_tx);
                    tasks::spawn_ws_order_book(
                        Arc::clone(&client),
                        tx.clone(),
                        token_pairs,
                        cancel_rx,
                    );
                }
                let outcome_names: Vec<String> =
                    market.outcomes.iter().map(|o| o.name.clone()).collect();
                let history_key = format!("{}:{}", market.condition_id, app.sparkline_interval);
                if !app.price_history.contains_key(&history_key) {
                    tasks::spawn_load_price_history(
                        Arc::clone(&client),
                        tx.clone(),
                        market.condition_id.clone(),
                        outcome_names,
                        app.sparkline_interval,
                    );
                }
            }
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
            // Detect fills: orders that were live last refresh but are now gone.
            // Cancels go through OrderCancelled which clears prev_live_order_ids,
            // so remaining disappearances are fills.
            if !app.prev_live_order_ids.is_empty() {
                let new_ids: HashSet<String> = orders.iter().map(|o| o.id.clone()).collect();
                let filled: Vec<&str> = app
                    .prev_live_order_ids
                    .iter()
                    .filter(|id| !new_ids.contains(id.as_str()))
                    .map(|s| s.as_str())
                    .collect();
                if !filled.is_empty() {
                    let msg = if filled.len() == 1 {
                        format!("Order filled: {}", &filled[0][..filled[0].len().min(12)])
                    } else {
                        format!("{} orders filled", filled.len())
                    };
                    app.set_flash(msg);
                    // Ring the terminal bell
                    print!("\x07");
                }
            }
            // Update tracked live order IDs for next comparison
            app.prev_live_order_ids = orders.iter().map(|o| o.id.clone()).collect();
            app.orders = orders;
            // Only clear the shared loading spinner when we're on the tab that
            // triggered this load; background preloads must not clobber it.
            if app.active_tab == Tab::Positions {
                app.loading = false;
            }
            app.last_error = None;
            if app.orders_list_state.selected().is_none() && !app.orders.is_empty() {
                app.orders_list_state.select(Some(0));
            }
        }

        AppEvent::BalanceLoaded(balance, allowance) => {
            app.balance = Some(balance);
            app.allowance = Some(allowance);
            app.balance_refreshed_at = Some(Instant::now());
            if app.active_tab == Tab::Balance {
                app.loading = false;
            }
            app.last_error = None;
        }

        AppEvent::OrderPlaced(order_id) => {
            app.loading = false;
            app.last_error = None;
            app.set_flash(format!("Order placed: {}", order_id));
            tasks::spawn_load_orders(Arc::clone(&client), tx.clone());
        }

        AppEvent::OrderCancelled(order_id) => {
            app.loading = false;
            // Remove cancelled orders from prev set so they don't trigger
            // a false "filled" notification on the next OrdersLoaded.
            if order_id == "all" {
                app.prev_live_order_ids.clear();
            } else {
                app.prev_live_order_ids.remove(&order_id);
            }
            app.set_flash(format!("Cancelled: {}", order_id));
            tasks::spawn_load_orders(Arc::clone(&client), tx.clone());
        }

        AppEvent::Redeemed(tx_hash) => {
            app.loading = false;
            app.set_flash(format!("Redeemed! tx: {}", tx_hash));
            tasks::spawn_load_positions(Arc::clone(&client), tx.clone());
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
                tasks::spawn_compute_analytics(
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

        AppEvent::GroupSlugBackfillComplete(filled) => {
            // Only re-run analytics if we actually wrote new group_slug values —
            // a 0-filled completion means we have nothing new to surface.
            if filled > 0 && !app.analytics_loading {
                app.analytics_loading = true;
                tasks::spawn_compute_analytics(
                    app.db_path.clone(),
                    tx.clone(),
                    Arc::clone(&client),
                    app.calibration_hours,
                );
            }
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

        AppEvent::UserOrderUpdate(order_id, status) => {
            let upper = status.to_uppercase();
            tracing::info!(order_id = %order_id, status = %upper, "user WS order event");
            if upper == "MATCHED" || upper == "FILLED" {
                app.set_flash(format!(
                    "Order filled: {}",
                    &order_id[..order_id.len().min(12)]
                ));
                // Ring the terminal bell.
                print!("\x07");
                // Remove from prev_live_order_ids so the REST-based fill detection
                // doesn't double-fire when the next OrdersLoaded arrives.
                app.prev_live_order_ids.remove(&order_id);
                // Trigger immediate refresh of orders + positions.
                tasks::spawn_load_orders(Arc::clone(&client), tx.clone());
                tasks::spawn_load_positions(Arc::clone(&client), tx.clone());
            } else if upper == "CANCELED" || upper == "CANCELLED" {
                app.prev_live_order_ids.remove(&order_id);
            }
        }

        AppEvent::UserWsConnected => {
            app.user_ws_connected = true;
        }

        AppEvent::UserWsDisconnected => {
            app.user_ws_connected = false;
        }

        AppEvent::NetWorthLogged(balance, _positions_value, _net_worth, history) => {
            app.net_worth_in_progress = false;
            app.net_worth_last_at = Some(chrono::Utc::now());
            app.net_worth_history = history;
            // Also update the live balance display as a side effect.
            app.balance = Some(balance);
        }

        AppEvent::NetWorthHistoryLoaded(history) => {
            app.net_worth_history = history;
        }

        AppEvent::ViewerPositionsLoaded(mut positions) => {
            positions.sort_by(|a, b| {
                let va = b.size * b.current_price;
                let vb = a.size * a.current_price;
                va.total_cmp(&vb)
            });
            app.viewer_positions = positions;
            if app.active_tab == Tab::Viewer {
                app.loading = false;
            }
            if app.viewer_list_state.selected().is_none() && !app.viewer_positions.is_empty() {
                app.viewer_list_state.select(Some(0));
            }
        }
    }
    false
}
