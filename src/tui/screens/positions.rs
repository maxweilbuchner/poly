use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::{is_auth_error, theme, App};
use crate::types::{Order, OrderStatus, Position, Side};

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(0),
        Constraint::Percentage(25),
    ])
    .split(area);

    render_summary(f, chunks[0], app);
    render_positions(f, chunks[1], app);
    render_orders(f, chunks[2], app);
}

// ── Portfolio summary panel ─────────────────────────────────────────────────

fn render_summary(f: &mut Frame, area: Rect, app: &App) {
    let empty_block = Block::bordered()
        .title(Span::styled(
            " Summary ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    if app.positions.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  No positions loaded.",
                Style::default().fg(theme::DIM),
            ))
            .block(empty_block),
            area,
        );
        return;
    }

    let total_unreal: f64 = app.positions.iter().map(|p| p.unrealized_pnl).sum();
    let total_real: f64 = app.positions.iter().map(|p| p.realized_pnl).sum();
    let cost_basis: f64 = app.positions.iter().map(|p| p.size * p.avg_price).sum();
    let portfolio_value: f64 = app.positions.iter().map(|p| p.size * p.current_price).sum();
    let return_pct = if cost_basis > 0.0 {
        (portfolio_value - cost_basis) / cost_basis * 100.0
    } else {
        0.0
    };
    let count = app.positions.len();
    let total_shares: f64 = app.positions.iter().map(|p| p.size).sum();

    let pnl_color = |v: f64| if v >= 0.0 { theme::GREEN } else { theme::RED };
    let sign = |v: f64| if v >= 0.0 { "+" } else { "" };

    let title = Span::styled(
        " Summary ",
        Style::default()
            .fg(theme::CYAN)
            .add_modifier(Modifier::BOLD),
    );

    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Build columns: label on top, value below
    let mut labels: Vec<(&str, Color)> = Vec::new();
    let mut values: Vec<(String, Color, bool)> = Vec::new();

    labels.push(("Shares", theme::DIM));
    values.push((
        format!("{:.2} ({})", total_shares, count),
        theme::TEXT,
        true,
    ));

    labels.push(("Value", theme::DIM));
    values.push((format!("${:.2}", portfolio_value), theme::TEXT, true));

    labels.push(("Cost", theme::DIM));
    values.push((format!("${:.2}", cost_basis), theme::TEXT, false));

    labels.push(("Unrealized", theme::DIM));
    values.push((
        format!("{}{:.4}", sign(total_unreal), total_unreal),
        pnl_color(total_unreal),
        true,
    ));

    if total_real != 0.0 {
        labels.push(("Realized", theme::DIM));
        values.push((
            format!("{}{:.4}", sign(total_real), total_real),
            pnl_color(total_real),
            true,
        ));
    }

    labels.push(("Return", theme::DIM));
    values.push((
        format!("{}{:.1}%", sign(return_pct), return_pct),
        pnl_color(return_pct),
        true,
    ));

    let col_w = (inner.width as usize).saturating_sub(2) / labels.len();

    let mut label_spans: Vec<Span<'static>> = vec![Span::raw("  ")];
    for (lbl, color) in &labels {
        label_spans.push(Span::styled(
            pad_right(lbl.to_string(), col_w),
            Style::default().fg(*color),
        ));
    }

    let mut value_spans: Vec<Span<'static>> = vec![Span::raw("  ")];
    for (val, color, bold) in &values {
        let mut style = Style::default().fg(*color);
        if *bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        value_spans.push(Span::styled(pad_right(val.clone(), col_w), style));
    }

    f.render_widget(
        Paragraph::new(vec![Line::from(label_spans), Line::from(value_spans)]),
        inner,
    );
}

// ── Positions list ──────────────────────────────────────────────────────────

fn render_positions(f: &mut Frame, area: Rect, app: &mut App) {
    let focused = !app.positions_focus_orders;
    let border_color = if focused {
        theme::BORDER_ACTIVE
    } else {
        theme::BORDER
    };

    let age_str = match app.positions_refreshed_at {
        Some(t) => {
            let secs = t.elapsed().as_secs();
            if secs < 5 {
                " [just now] ".to_string()
            } else if secs < 60 {
                format!(" [{}s ago] ", secs)
            } else if secs < 3600 {
                format!(" [{}m ago] ", secs / 60)
            } else {
                format!(" [{}h ago] ", secs / 3600)
            }
        }
        None => String::new(),
    };

    let title = Line::from(vec![
        Span::styled(
            " Positions",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(age_str, Style::default().fg(theme::VERY_DIM)),
    ]);

    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme::PANEL_BG));

    if app.positions.is_empty() {
        if app.loading {
            f.render_widget(
                Paragraph::new(Span::styled("Loading…", Style::default().fg(theme::DIM)))
                    .block(block),
                area,
            );
        } else if let Some(err) = &app.last_error {
            let err_str = err.to_string();
            if is_auth_error(err) {
                f.render_widget(auth_error_paragraph(&err_str, block), area);
            } else {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "No open positions.",
                        Style::default().fg(theme::DIM),
                    ))
                    .block(block),
                    area,
                );
            }
        } else {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "No open positions.",
                    Style::default().fg(theme::DIM),
                ))
                .block(block),
                area,
            );
        }
        return;
    }

    // Render block shell first, then split the inner area.
    let inner = block.inner(area);
    f.render_widget(block, area);

    let inner_chunks = Layout::vertical([
        Constraint::Length(1), // column header
        Constraint::Min(0),    // list
    ])
    .split(inner);

    let end_dates: Vec<Option<String>> = app
        .positions
        .iter()
        .map(|p| {
            // Prefer the end_date fetched with the position (covers expired/low-volume markets).
            // Fall back to the in-memory market list for markets still actively loaded.
            p.end_date.clone().or_else(|| {
                app.markets
                    .iter()
                    .find(|m| m.condition_id == p.market_id)
                    .and_then(|m| m.end_date.clone())
            })
        })
        .collect();

    let (header, items) = build_position_items(&app.positions, area.width as usize, &end_dates);
    f.render_widget(Paragraph::new(header), inner_chunks[0]);

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(32, 38, 72))
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, inner_chunks[1], &mut app.positions_list_state);
}

fn build_position_items(
    positions: &[Position],
    area_width: usize,
    end_dates: &[Option<String>],
) -> (Line<'static>, Vec<ListItem<'static>>) {
    // ── Pre-pass: compute max expiry label width so we can reserve space on line1 ─

    struct PRow {
        question: String,
        outcome: String,
        size_num: String,
        avg_str: String,
        cur_str: String,
        pnl_str: String,
        pnl_color: Color,
        cur_color: Color,
        redeemable: bool,
        /// Single unified status: "won"/"lost"/"pending"/"exp Xd"/...
        status: String,
        status_color: Color,
    }

    let status_width: usize = positions
        .iter()
        .zip(end_dates.iter())
        .map(|(p, ed)| compute_status(p, ed.as_deref()).0.len())
        .max()
        .unwrap_or(7);

    // Reserve space for the [R] badge if any position is redeemable.
    let has_redeemable = positions.iter().any(|p| p.redeemable);
    let badge_width = if has_redeemable { 5 } else { 0 };

    // ── Pass 1: format every field, measure column widths ─────────────────────

    let rows: Vec<PRow> = positions
        .iter()
        .zip(end_dates.iter())
        .map(|(p, ed)| {
            let pnl_sign = if p.unrealized_pnl >= 0.0 { "+" } else { "" };
            let (status, status_color) = compute_status(p, ed.as_deref());
            PRow {
                question: p.market_question.clone(),
                outcome: p.outcome.clone(),
                size_num: format!("{:.2}", p.size),
                avg_str: format!("{:.4}", p.avg_price),
                cur_str: format!("{:.4}", p.current_price),
                pnl_str: format!("{}{:.4}", pnl_sign, p.unrealized_pnl),
                pnl_color: if p.unrealized_pnl >= 0.0 {
                    theme::GREEN
                } else {
                    theme::RED
                },
                cur_color: if p.current_price > p.avg_price + 0.01 {
                    theme::GREEN
                } else if p.current_price < p.avg_price - 0.01 {
                    theme::RED
                } else {
                    theme::TEXT
                },
                redeemable: p.redeemable,
                status,
                status_color,
            }
        })
        .collect();

    // Max width of each variable column across all rows.
    let max_outcome = rows
        .iter()
        .map(|r| r.outcome.chars().count())
        .max()
        .unwrap_or(3);
    let max_size = rows.iter().map(|r| r.size_num.len()).max().unwrap_or(4);
    let max_avg = rows.iter().map(|r| r.avg_str.len()).max().unwrap_or(10);
    let max_cur = rows.iter().map(|r| r.cur_str.len()).max().unwrap_or(10);
    let max_pnl = rows.iter().map(|r| r.pnl_str.len()).max().unwrap_or(11);

    // Single-line layout:
    //   indent(2) + outcome + sep(4) + question + sep(4) + status + sep(4)
    //   + size + " shares"(7) + sep(4) + avg + sep(4) + cur + sep(4) + pnl + badge
    // Subtract 4 extra for borders(2) + highlight symbol "▸ "(2) not in line content.
    let size_hdr_w = max_size + 7;
    let fixed = 2
        + max_outcome
        + 4
        + 4
        + status_width
        + 4
        + size_hdr_w
        + 4
        + max_avg
        + 4
        + max_cur
        + 4
        + max_pnl
        + badge_width;
    let q_width = area_width.saturating_sub(4).saturating_sub(fixed).max(20);

    // ── Column header ─────────────────────────────────────────────────────────
    let header = Line::from(vec![
        Span::raw("    "),
        Span::styled(
            pad_right("Outcome".to_string(), max_outcome),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_right("Market".to_string(), q_width),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_right("Status".to_string(), status_width),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_left("Shares".to_string(), size_hdr_w),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_left("Avg".to_string(), max_avg),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_left("Cur".to_string(), max_cur),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_left("P&L".to_string(), max_pnl),
            Style::default().fg(theme::DIM),
        ),
    ]);

    // ── Pass 2: build ListItems with padded columns ───────────────────────────

    let items = rows
        .into_iter()
        .map(|r| {
            let outcome_cell = pad_right(truncate(&r.outcome, max_outcome), max_outcome);
            let question_cell = pad_right(truncate(&r.question, q_width), q_width);
            let status_cell = pad_right(r.status, status_width);
            let size_cell = format!("{:>width$} shares", r.size_num, width = max_size);
            let avg_cell = pad_left(r.avg_str, max_avg);
            let cur_cell = pad_left(r.cur_str, max_cur);
            let pnl_cell = pad_left(r.pnl_str, max_pnl);

            let mut spans = vec![
                Span::raw("  "),
                Span::styled(outcome_cell, Style::default().fg(theme::CYAN)),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(question_cell, Style::default().fg(theme::TEXT)),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(
                    status_cell,
                    Style::default()
                        .fg(r.status_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(size_cell, Style::default().fg(theme::TEXT)),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(avg_cell, Style::default().fg(theme::DIM)),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(cur_cell, Style::default().fg(r.cur_color)),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(
                    pnl_cell,
                    Style::default()
                        .fg(r.pnl_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            if r.redeemable {
                spans.push(Span::styled(
                    "  [R]",
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    (header, items)
}

/// Unified status for a position: "won"/"lost" once resolved, "pending" once past
/// close but unresolved, otherwise the time-to-expiry label ("exp 3h" etc.).
fn compute_status(p: &Position, end: Option<&str>) -> (String, Color) {
    if p.market_closed {
        return if p.current_price > 0.95 {
            ("won".to_string(), theme::GREEN)
        } else {
            ("lost".to_string(), theme::RED)
        };
    }
    match end {
        Some(s) => {
            let (label, color) = format_expiry(s);
            // Past end_date but market not yet flagged closed → pending resolution.
            if label == "expired" {
                ("pending".to_string(), theme::YELLOW)
            } else {
                (label, color)
            }
        }
        None => ("pending".to_string(), theme::YELLOW),
    }
}

/// Parse an end-date string and return a human-readable "exp Xd/Xh/Xm" label plus its colour.
fn format_expiry(end: &str) -> (String, Color) {
    let dt: Option<DateTime<Local>> = DateTime::parse_from_rfc3339(end)
        .map(|dt| dt.with_timezone(&Local))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(end.get(..10).unwrap_or(""), "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|ndt| Utc.from_utc_datetime(&ndt).with_timezone(&Local))
        });

    match dt {
        None => (String::new(), theme::VERY_DIM),
        Some(dt) => {
            let now = Local::now();
            if dt <= now {
                return ("expired".to_string(), theme::RED);
            }
            let dur = dt.signed_duration_since(now);
            let days = dur.num_days();
            let hours = dur.num_hours();
            let mins = dur.num_minutes();
            let (label, color) = if days >= 7 {
                (format!("exp {}d", days), theme::DIM)
            } else if days >= 2 {
                (format!("exp {}d", days), theme::TEXT)
            } else if hours >= 2 {
                (format!("exp {}h", hours), theme::YELLOW)
            } else if mins >= 1 {
                (format!("exp {}m", mins), theme::RED)
            } else {
                ("exp <1m".to_string(), theme::RED)
            };
            (label, color)
        }
    }
}

fn render_orders(f: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.positions_focus_orders;
    let border_color = if focused {
        theme::BORDER_ACTIVE
    } else {
        theme::BORDER
    };

    let block = Block::bordered()
        .title(Span::styled(
            " Open Orders ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme::PANEL_BG));

    if app.orders.is_empty() {
        if app.loading {
            f.render_widget(
                Paragraph::new(Span::styled("Loading…", Style::default().fg(theme::DIM)))
                    .block(block),
                area,
            );
        } else if let Some(err) = &app.last_error {
            let err_str = err.to_string();
            if is_auth_error(err) {
                f.render_widget(auth_error_paragraph(&err_str, block), area);
            } else {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "No open orders.",
                        Style::default().fg(theme::DIM),
                    ))
                    .block(block),
                    area,
                );
            }
        } else {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "No open orders.",
                    Style::default().fg(theme::DIM),
                ))
                .block(block),
                area,
            );
        }
        return;
    }

    let items = build_order_items(&app.orders, area.width as usize);
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(32, 38, 72))
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut app.orders_list_state);
}

fn build_order_items(orders: &[Order], area_width: usize) -> Vec<ListItem<'static>> {
    // ── Pass 1: format every field, measure column widths ─────────────────────

    struct ORow {
        question: String,
        side_str: String,
        outcome: String,
        price_str: String,
        size_num: String,
        filled_str: String, // empty if not partially filled
        status_str: String,
        side_color: Color,
        status_color: Color,
    }

    let q_width = area_width.saturating_sub(6);

    let rows: Vec<ORow> = orders
        .iter()
        .map(|o| {
            let label = if o.market.is_empty() {
                &o.id
            } else {
                &o.market
            };
            let remaining = o.original_size - o.size_matched;
            ORow {
                question: truncate(label, q_width),
                side_str: format!("{}", o.side),
                outcome: o.outcome.clone(),
                price_str: format!("@{:.4}", o.price),
                size_num: format!("{:.2}", o.original_size),
                filled_str: if o.size_matched > 0.0 {
                    format!("filled {:.2}  rem {:.2}", o.size_matched, remaining)
                } else {
                    String::new()
                },
                status_str: format!("[{}]", o.status),
                side_color: match o.side {
                    Side::Buy => theme::GREEN,
                    Side::Sell => theme::RED,
                },
                status_color: match o.status {
                    OrderStatus::Live => theme::BLUE,
                    OrderStatus::Filled => theme::GREEN,
                    OrderStatus::Cancelled => theme::RED,
                    OrderStatus::PartiallyFilled => theme::YELLOW,
                    OrderStatus::Unknown => theme::VERY_DIM,
                },
            }
        })
        .collect();

    let max_side = rows.iter().map(|r| r.side_str.len()).max().unwrap_or(4);
    let max_size = rows.iter().map(|r| r.size_num.len()).max().unwrap_or(4);
    let max_price = rows.iter().map(|r| r.price_str.len()).max().unwrap_or(7);
    let max_filled = rows.iter().map(|r| r.filled_str.len()).max().unwrap_or(0);
    let max_status = rows.iter().map(|r| r.status_str.len()).max().unwrap_or(6);
    let has_outcome = rows.iter().any(|r| !r.outcome.is_empty());
    let has_filled = rows.iter().any(|r| !r.filled_str.is_empty());

    // Fixed overhead: indent(2) + side + sep(4) + price + sep(4) + " shares"(7) + size + sep(4) + status
    //   + if outcome column exists: sep(4)
    //   + if filled column exists:  sep(4) + filled
    // Subtract 4 extra for borders(2) + highlight symbol "▸ "(2) not in line content.
    let fixed = 2
        + max_side
        + 4
        + max_price
        + 4
        + max_size
        + 7
        + 4
        + max_status
        + if has_outcome { 4 } else { 0 }
        + if has_filled { 4 + max_filled } else { 0 };
    let area_width = area_width.saturating_sub(4);
    let outcome_width = if has_outcome {
        area_width.saturating_sub(fixed).max(4)
    } else {
        0
    };

    // ── Pass 2: build ListItems with padded columns ───────────────────────────

    rows.into_iter()
        .map(|r| {
            let line1 = Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    r.question,
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);

            let side_cell = pad_right(r.side_str, max_side);
            let price_cell = pad_right(r.price_str, max_price);
            let size_cell = format!("{:>width$} shares", r.size_num, width = max_size);
            let status_cell = pad_right(r.status_str, max_status);

            let mut spans = vec![
                Span::raw("  "),
                Span::styled(
                    side_cell,
                    Style::default()
                        .fg(r.side_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            if has_outcome {
                let outcome_cell = pad_right(truncate(&r.outcome, outcome_width), outcome_width);
                spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
                spans.push(Span::styled(outcome_cell, Style::default().fg(theme::DIM)));
            }
            spans.extend([
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(price_cell, Style::default().fg(theme::DIM)),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(size_cell, Style::default().fg(theme::DIM)),
            ]);
            if has_filled {
                let filled_cell = pad_right(r.filled_str, max_filled);
                spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
                spans.push(Span::styled(filled_cell, Style::default().fg(theme::DIM)));
            }
            spans.extend([
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(status_cell, Style::default().fg(r.status_color)),
            ]);

            let line2 = Line::from(spans);
            ListItem::new(vec![line1, line2])
        })
        .collect()
}

/// Right-pad `s` with spaces (so `s` sits on the left).
fn pad_right(s: String, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s
    } else {
        let mut out = s;
        out.extend(std::iter::repeat_n(' ', width - len));
        out
    }
}

/// Left-pad `s` with spaces (so `s` sits on the right — for right-aligned numerics).
fn pad_left(s: String, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s
    } else {
        let pad: String = std::iter::repeat_n(' ', width - len).collect();
        pad + &s
    }
}

/// Build a Paragraph that shows an auth/credentials error persistently inside a panel.
fn auth_error_paragraph<'a>(err: &'a str, block: Block<'a>) -> Paragraph<'a> {
    let mut lines = vec![Line::from("")];
    for raw in err.lines() {
        let line = raw.trim_start_matches("  ");
        if line.starts_with("Hint:") {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(line.to_string(), Style::default().fg(theme::YELLOW)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(theme::ERROR)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }
    Paragraph::new(lines).block(block)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}
