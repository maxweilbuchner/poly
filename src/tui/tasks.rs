use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;

use crate::client::PolyClient;
use crate::types::{Market, MarketStatus, OrderBook, PlaceOrderParams, PricePoint, Side};

use super::state::{market_category, market_category_from_parts, AnalyticsStats, AppEvent};

pub fn copy_to_clipboard(text: &str) {
    use std::io::Write;

    // Platform-specific clipboard commands, tried in order.
    let candidates: &[&[&str]] = if cfg!(target_os = "macos") {
        &[&["pbcopy"]]
    } else if cfg!(target_os = "windows") {
        &[&["clip.exe"]]
    } else {
        // Linux / BSD — prefer xclip, fall back to xsel, then wl-copy (Wayland)
        &[
            &["xclip", "-selection", "clipboard"],
            &["xsel", "--clipboard", "--input"],
            &["wl-copy"],
        ]
    };

    for cmd in candidates {
        let program = cmd[0];
        let args = &cmd[1..];
        if let Ok(mut child) = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
    tracing::warn!("no clipboard command available");
}

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
                        let short = page.len() < PAGE;
                        offset += page.len();
                        pages.push(page);
                        if short {
                            reached_end = true;
                            break;
                        }
                    }
                    Err(e) => {
                        if is_first {
                            let _ = tx.send(AppEvent::Error(e));
                            return;
                        }
                        reached_end = true;
                        break;
                    }
                }
            }

            // Flatten and send.
            let batch: Vec<Market> = pages.into_iter().flatten().collect();
            if !batch.is_empty() {
                if is_first {
                    let _ = tx.send(AppEvent::MarketsLoaded(batch, reached_end));
                    is_first = false;
                } else {
                    let _ = tx.send(AppEvent::MarketsAppended(batch, reached_end));
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

/// Fetch a market by condition ID (falling back to token ID lookup for neg-risk
/// markets) and then load its order books.
pub fn spawn_load_detail_by_id(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    condition_id: String,
    token_id: String,
    market_question: String,
) {
    tokio::spawn(async move {
        // Try condition ID first.
        // Neg-risk markets return 422 (Err) — their position asset is NOT a
        // CLOB token ID, so the clobTokenIds fallback would return the WRONG
        // market.  For those we search by question text instead.
        // Normal 404 (Ok(None)) can still try the token ID fallback safely.
        let market = match client.get_market_by_id(&condition_id).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                // Normal not-found — token ID fallback is safe here
                match client.get_market_by_token_id(&token_id).await {
                    Ok(Some(m)) => m,
                    Ok(None) => {
                        let _ = tx.send(AppEvent::Error(crate::error::AppError::Api {
                            status: 404,
                            message: "Market not found".to_string(),
                        }));
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(e));
                        return;
                    }
                }
            }
            Err(_) => {
                // Likely neg-risk (422) — search by question text
                let found = if !market_question.is_empty() {
                    client
                        .get_market_by_question(&market_question)
                        .await
                        .ok()
                        .flatten()
                } else {
                    None
                };
                match found {
                    Some(m) => m,
                    None => {
                        // Build a minimal Market from position data so we at
                        // least show the correct question instead of wrong data.
                        use crate::types::Outcome;
                        Market {
                            condition_id: condition_id.clone(),
                            question: market_question.clone(),
                            description: None,
                            slug: String::new(),
                            group_slug: String::new(),
                            status: MarketStatus::Active,
                            end_date: None,
                            volume: 0.0,
                            liquidity: 0.0,
                            outcomes: vec![Outcome {
                                name: "Yes".into(),
                                token_id: token_id.clone(),
                                price: 0.0,
                                bid: 0.0,
                                ask: 0.0,
                                bid_depth: 0.0,
                                ask_depth: 0.0,
                            }],
                            category: None,
                            tags: vec![],
                            neg_risk: true,
                        }
                    }
                }
            }
        };
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

pub fn spawn_log_net_worth(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    db_path: std::path::PathBuf,
) {
    tokio::spawn(async move {
        // Fetch balance and positions concurrently.
        let (bal_result, pos_result) = tokio::join!(client.get_balance(), client.get_positions(),);

        // If either API call fails (e.g. no internet), skip logging to avoid
        // recording bogus $0 data points that distort the chart.
        let (balance, positions) = match (bal_result, pos_result) {
            (Ok(b), Ok(p)) => (b, p),
            _ => return,
        };
        let positions_value: f64 = positions.iter().map(|p| p.size * p.current_price).sum();
        let net_worth = balance + positions_value;

        // Insert into DB and load history.
        let history = tokio::task::spawn_blocking(move || -> Vec<(f64, f64)> {
            let conn = match crate::db::open(&db_path) {
                Ok(c) => c,
                Err(_) => return vec![],
            };
            let logged_at = chrono::Utc::now().to_rfc3339();
            let _ =
                crate::db::insert_net_worth(&conn, &logged_at, balance, positions_value, net_worth);
            crate::db::query_net_worth_history(&conn).unwrap_or_default()
        })
        .await
        .unwrap_or_default();

        let _ = tx.send(AppEvent::NetWorthLogged(
            balance,
            positions_value,
            net_worth,
            history,
        ));
    });
}

pub fn spawn_load_net_worth_history(tx: UnboundedSender<AppEvent>, db_path: std::path::PathBuf) {
    tokio::task::spawn_blocking(move || {
        let history = crate::db::open(&db_path)
            .and_then(|c| crate::db::query_net_worth_history(&c))
            .unwrap_or_default();
        if !history.is_empty() {
            let _ = tx.send(AppEvent::NetWorthHistoryLoaded(history));
        }
    });
}

pub fn spawn_load_viewer_positions(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    address: String,
) {
    tokio::spawn(async move {
        match client.get_positions_for_address(&address).await {
            Ok(p) => {
                let _ = tx.send(AppEvent::ViewerPositionsLoaded(p));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(e));
            }
        }
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
        // Render whatever's already in the DB immediately so the tab stops
        // showing "Computing analytics…" before the network fetch finishes.
        let stats = compute_analytics_stats(db_path.clone(), calibration_hours).await;
        let _ = tx.send(AppEvent::AnalyticsComputed(Box::new(stats)));

        // ── Calibration price fetch ───────────────────────────────────────────
        // Fetch CLOB price-history for resolved markets that don't yet have a
        // calibration price stored at the requested horizon. After the fetch
        // completes (and only if any new prices arrived), recompute and resend.
        const CAL_BATCH: usize = 5_000;
        const CAL_CONCURRENCY: usize = 32;
        let unpriced: Vec<(String, String, String)> = {
            let db = db_path.clone();
            let h = calibration_hours;
            tokio::task::spawn_blocking(move || {
                crate::db::open(&db)
                    .and_then(|c| crate::db::query_unpriced_resolutions(&c, h, CAL_BATCH))
            })
            .await
            .unwrap_or(Ok(vec![]))
            .unwrap_or_default()
        };

        if unpriced.is_empty() {
            return;
        }

        use futures_util::future::join_all;
        let total_unpriced = unpriced.len();
        let mut fetch_done = 0usize;
        let mut new_prices_total = 0usize;
        let _ = tx.send(AppEvent::CalibrationFetchProgress(0, total_unpriced));

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
            let written: usize = tokio::task::spawn_blocking(move || -> usize {
                let Ok(conn) = crate::db::open(&db) else {
                    return 0;
                };
                let mut written = 0usize;
                for (cid, price) in results.into_iter().flatten() {
                    if crate::db::update_calibration_price(&conn, &cid, price, h).is_ok() {
                        written += 1;
                    }
                }
                written
            })
            .await
            .unwrap_or(0);
            new_prices_total += written;
        }

        if new_prices_total > 0 {
            let stats = compute_analytics_stats(db_path, calibration_hours).await;
            let _ = tx.send(AppEvent::AnalyticsComputed(Box::new(stats)));
        } else {
            // Still need to clear the calibration-fetch spinner.
            let _ = tx.send(AppEvent::CalibrationFetchProgress(0, 0));
        }
    });
}

async fn compute_analytics_stats(
    db_path: std::path::PathBuf,
    calibration_hours: u64,
) -> AnalyticsStats {
    // Each query runs on its own connection in a separate blocking task. SQLite
    // in WAL mode supports concurrent readers, so the slowest query (typically
    // query_calibration_raw) no longer serializes behind the others.
    let blocking = |f: Box<dyn FnOnce(&rusqlite::Connection) + Send>| {
        let p = db_path.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = crate::db::open(&p) {
                f(&conn);
            }
        })
    };

    use std::sync::Mutex;
    let stats = Arc::new(Mutex::new(AnalyticsStats::default()));

    // Charts A, B: per-market data from the latest snapshot run.
    let s1 = Arc::clone(&stats);
    let t1 = blocking(Box::new(move |conn| {
        if let Ok(markets) = crate::db::query_latest_snapshot(conn) {
            let mut s = s1.lock().unwrap();
            s.total_markets = markets.len();
            for (_, volume, _liquidity, yes_price) in &markets {
                s.total_volume += volume;
                if let Some(&price) = yes_price.as_ref() {
                    if (0.01..=0.99).contains(&price) {
                        let b20 = ((price * 20.0) as usize).min(19);
                        s.prob_buckets[b20] += 1;
                    }
                }
            }
        }
    }));

    // Chart B: resolution bias.
    let s2 = Arc::clone(&stats);
    let t2 = blocking(Box::new(move |conn| {
        if let Ok((yes, no, other)) = crate::db::query_resolution_counts(conn) {
            let mut s = s2.lock().unwrap();
            s.res_yes = yes;
            s.res_no = no;
            s.res_other = other;
        }
    }));

    // Chart C: calibration curve.
    let s3 = Arc::clone(&stats);
    let t3 = blocking(Box::new(move |conn| {
        if let Ok(cal) = crate::db::query_calibration(conn, calibration_hours) {
            s3.lock().unwrap().calibration = cal;
        }
    }));

    // Chart D: price accuracy vs market volume.
    let s4 = Arc::clone(&stats);
    let t4 = blocking(Box::new(move |conn| {
        if let Ok(evv) = crate::db::query_edge_vs_volume(conn) {
            s4.lock().unwrap().edge_vs_vol = evv;
        }
    }));

    // Chart D: calibration fit per (category × volume tier).
    let s5 = Arc::clone(&stats);
    let t5 = blocking(Box::new(move |conn| {
        if let Ok(rows) = crate::db::query_calibration_raw(conn, calibration_hours) {
            let mut s = s5.lock().unwrap();
            for (q, slug, vol, yes_price, res) in rows {
                let cat_idx = match market_category_from_parts(&q, &slug) {
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
                let cell = &mut s.calibration_matrix[cat_idx][tier];
                cell.buckets[b].1 += 1;
                if res == "yes" {
                    cell.buckets[b].0 += 1;
                }
                cell.n += 1;
            }
        }
    }));

    // High-confidence accuracy.
    let s6 = Arc::clone(&stats);
    let t6 = blocking(Box::new(move |conn| {
        if let Ok((correct, wrong)) = crate::db::query_high_confidence_accuracy(conn) {
            let mut s = s6.lock().unwrap();
            s.hc_correct = correct;
            s.hc_wrong = wrong;
        }
    }));

    // Most-accurate recurring series at the current calibration horizon.
    // Constants: at least 5 resolved markets per series, top 10 series shown.
    let s7 = Arc::clone(&stats);
    let t7 = blocking(Box::new(move |conn| {
        if let Ok(rows) = crate::db::query_recurring_accuracy(conn, calibration_hours, 5, 10) {
            s7.lock().unwrap().recurring_accuracy = rows;
        }
    }));

    let _ = tokio::join!(t1, t2, t3, t4, t5, t6, t7);

    Arc::try_unwrap(stats)
        .ok()
        .and_then(|m| m.into_inner().ok())
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
                                group_slug: r.group_slug.clone(),
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
                                group_slug: r.group_slug.clone(),
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

            for mr in join_all(futs).await.into_iter().flatten().flatten() {
                xref_rows.push(ResolutionRow {
                    condition_id: mr.condition_id.clone(),
                    question: mr.question.clone(),
                    slug: mr.slug.clone(),
                    end_date: mr.end_date.as_deref().unwrap_or("").to_string(),
                    resolution: mr.resolution.clone(),
                    last_trade_price: mr.last_trade_price,
                    clob_token_id: mr.clob_token_id.clone(),
                    group_slug: mr.group_slug.clone(),
                });
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
    cancel: watch::Receiver<bool>,
) {
    spawn_ws_order_book_at_url(
        client,
        tx,
        token_ids,
        cancel,
        "wss://ws-subscriptions-clob.polymarket.com/ws/market".to_string(),
    );
}

/// Same as [`spawn_ws_order_book`] but accepts a custom WebSocket URL.
/// Exposed for integration tests so they can drive a local mock server.
pub fn spawn_ws_order_book_at_url(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    token_ids: Vec<(String, String)>,
    mut cancel: watch::Receiver<bool>,
    ws_url: String,
) {
    tokio::spawn(async move {
        use futures_util::{SinkExt, StreamExt};
        use serde::Deserialize;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

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
            let ws_stream = match connect_async(ws_url.as_str()).await {
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

/// Connects to the Polymarket user WebSocket channel for live order/trade events.
/// Sends `AppEvent::UserOrderUpdate` when an order status changes (fills, cancels).
/// Falls back to a no-op sleep loop on auth failure or disconnect — REST polling
/// in the Tick handler provides the safety net.
pub fn spawn_ws_user_channel(
    auth: crate::auth::ClobAuth,
    tx: UnboundedSender<AppEvent>,
    cancel: watch::Receiver<bool>,
) {
    spawn_ws_user_channel_at_url(
        auth,
        tx,
        cancel,
        "wss://ws-subscriptions-clob.polymarket.com/ws/user".to_string(),
    );
}

/// Same as [`spawn_ws_user_channel`] but accepts a custom WebSocket URL.
/// Exposed for integration tests so they can drive a local mock server.
pub fn spawn_ws_user_channel_at_url(
    auth: crate::auth::ClobAuth,
    tx: UnboundedSender<AppEvent>,
    mut cancel: watch::Receiver<bool>,
    ws_url: String,
) {
    tokio::spawn(async move {
        use futures_util::{SinkExt, StreamExt};
        use serde::Deserialize;
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        #[derive(Deserialize)]
        struct WsUserEvent {
            #[serde(default)]
            id: String,
            #[serde(default)]
            status: String,
        }

        'ws: {
            let ws_stream = match connect_async(ws_url.as_str()).await {
                Ok((stream, _)) => stream,
                Err(e) => {
                    tracing::warn!("user WS connect failed: {}", e);
                    break 'ws;
                }
            };

            let (mut write, mut read) = ws_stream.split();

            // Authenticate with CLOB credentials.
            let auth_msg = auth.ws_auth_message().to_string();
            if write.send(Message::Text(auth_msg)).await.is_err() {
                break 'ws;
            }

            let _ = tx.send(AppEvent::UserWsConnected);
            tracing::info!("user WS channel connected");

            const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
            const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
            let mut ping_interval = tokio::time::interval(PING_INTERVAL);
            ping_interval.tick().await; // consume immediate tick

            loop {
                tokio::select! {
                    result = tokio::time::timeout(READ_TIMEOUT, read.next()) => {
                        match result {
                            Err(_) => break 'ws, // 30s silence → dead connection
                            Ok(msg) => match msg {
                                Some(Ok(Message::Text(text))) => {
                                    // Parse as array or single event.
                                    let events: Vec<WsUserEvent> =
                                        match serde_json::from_str::<Vec<WsUserEvent>>(&text)
                                            .or_else(|_| serde_json::from_str::<WsUserEvent>(&text).map(|e| vec![e]))
                                        {
                                            Ok(v) => v,
                                            Err(_) => continue, // unrecognised frame — skip
                                        };
                                    for ev in events {
                                        if ev.id.is_empty() || ev.status.is_empty() { continue; }
                                        let _ = tx.send(AppEvent::UserOrderUpdate(ev.id, ev.status));
                                    }
                                }
                                Some(Ok(Message::Ping(d))) => { let _ = write.send(Message::Pong(d)).await; }
                                Some(Ok(Message::Pong(_))) => {}
                                Some(Ok(_)) => {}
                                _ => break 'ws, // closed or error
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

        // Disconnected — notify and keep the task alive so the cancel watch stays valid.
        let _ = tx.send(AppEvent::UserWsDisconnected);
        tracing::info!("user WS disconnected, positions fall back to REST polling");

        // Sleep until cancelled (REST polling in the Tick handler is the fallback).
        loop {
            tokio::select! {
                _ = cancel.changed() => {
                    if *cancel.borrow() { return; }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
            }
        }
    });
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

pub(super) fn spawn_fetch_fee_rate(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    token_id: String,
) {
    tokio::spawn(async move {
        // Default to 0: most active markets return 404 (no fee), only special
        // markets return a non-zero base_fee. This matches py-clob-client behaviour.
        let bps = client.get_fee_rate(&token_id).await.unwrap_or(0);
        let _ = tx.send(AppEvent::FeeRateFetched(bps));
    });
}

pub(super) fn spawn_fetch_market_price(
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

/// Backfills `group_slug` on existing `resolutions` rows that pre-date the column.
///
/// One-shot task spawned at TUI startup. Reads up to `BACKFILL_CAP` resolutions
/// with empty `group_slug`, fetches each one from Gamma in parallel chunks
/// (re-using the client's request throttle), and writes any non-empty group_slug
/// values back. Idempotent: a second run picks up rows the first run didn't fill.
///
/// Sends `GroupSlugBackfillComplete(filled_count)` on completion (even if zero).
pub fn spawn_backfill_group_slugs(
    client: Arc<PolyClient>,
    tx: UnboundedSender<AppEvent>,
    db_path: std::path::PathBuf,
) {
    tokio::spawn(async move {
        use futures_util::future::join_all;
        const BACKFILL_CAP: usize = 1_000;
        const CONCURRENCY: usize = 16;

        // Pull pending condition_ids off the DB on a blocking thread.
        let db_p = db_path.clone();
        let pending: Vec<String> = match tokio::task::spawn_blocking(move || {
            let conn = crate::db::open(&db_p)?;
            let mut stmt = conn.prepare(
                "SELECT condition_id FROM resolutions
                 WHERE group_slug = ''
                 ORDER BY end_date DESC
                 LIMIT ?1",
            )?;
            let ids: Vec<String> = stmt
                .query_map(rusqlite::params![BACKFILL_CAP as i64], |r| {
                    r.get::<_, String>(0)
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok::<_, rusqlite::Error>(ids)
        })
        .await
        {
            Ok(Ok(v)) => v,
            _ => {
                let _ = tx.send(AppEvent::GroupSlugBackfillComplete(0));
                return;
            }
        };

        if pending.is_empty() {
            let _ = tx.send(AppEvent::GroupSlugBackfillComplete(0));
            return;
        }

        // Fan out Gamma lookups in chunks. The client's internal semaphore
        // throttles total concurrent in-flight requests.
        let mut updates: Vec<(String, String)> = Vec::new();
        for chunk in pending.chunks(CONCURRENCY) {
            let futs: Vec<_> = chunk
                .iter()
                .map(|cid| {
                    let c = Arc::clone(&client);
                    let cid = cid.clone();
                    async move {
                        let res = c.get_market_resolution(&cid).await;
                        (cid, res)
                    }
                })
                .collect();

            for (cid, res) in join_all(futs).await {
                if let Ok(Some(mr)) = res {
                    if !mr.group_slug.is_empty() {
                        updates.push((cid, mr.group_slug));
                    }
                }
            }
        }

        let filled = updates.len();
        if filled > 0 {
            let db_p = db_path.clone();
            let _ = tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
                let mut conn = crate::db::open(&db_p)?;
                let tx = conn.transaction()?;
                {
                    let mut stmt = tx.prepare_cached(
                        "UPDATE resolutions SET group_slug = ?1
                         WHERE condition_id = ?2 AND group_slug = ''",
                    )?;
                    for (cid, slug) in &updates {
                        let _ = stmt.execute(rusqlite::params![slug, cid]);
                    }
                }
                tx.commit()
            })
            .await;
        }

        let _ = tx.send(AppEvent::GroupSlugBackfillComplete(filled));
    });
}

/// Fetch an Open-Meteo forecast for the given airport + resolution date and
/// emit `ForecastLoaded`/`ForecastFailed` keyed by `condition_id`.
pub fn spawn_load_forecast(
    tx: UnboundedSender<AppEvent>,
    condition_id: String,
    airport: crate::weather::Airport,
    resolution_date: chrono::NaiveDate,
) {
    tokio::spawn(async move {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest client");
        match crate::forecast::fetch_forecast(&http, &airport, resolution_date).await {
            Ok(forecast) => {
                let _ = tx.send(AppEvent::ForecastLoaded {
                    condition_id,
                    forecast: Box::new(forecast),
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "forecast fetch failed");
                let _ = tx.send(AppEvent::ForecastFailed {
                    condition_id,
                    error: e.to_string(),
                });
            }
        }
    });
}
