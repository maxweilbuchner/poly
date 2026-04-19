use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Chart, Dataset, GraphType, Paragraph},
    Frame,
};

use crate::tui::{is_auth_error, theme, App};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(22), Constraint::Min(0)]).split(area);

    render_balance_panel(f, chunks[0], app);
    render_net_worth_chart(f, chunks[1], app);
}

fn render_balance_panel(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered()
        .title(Span::styled(
            " Balance ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    let lines = if app.loading && app.balance.is_none() {
        vec![
            Line::from(""),
            Line::from(Span::styled("  Loading…", Style::default().fg(theme::DIM))),
        ]
    } else if app.balance.is_none() {
        if let Some(err) = &app.last_error {
            if is_auth_error(err) {
                let err_str = err.to_string();
                let mut lines = vec![Line::from("")];
                for raw in err_str.lines() {
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
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled(
                    "  r to retry after adding credentials",
                    Style::default().fg(theme::VERY_DIM),
                )]));
                // render immediately with this lines vec
                let para = Paragraph::new(lines).block(block);
                f.render_widget(para, area);
                return;
            }
        }
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No data. Press r to refresh.",
                Style::default().fg(theme::DIM),
            )),
        ]
    } else {
        let bal = app.balance.unwrap_or(0.0);

        // Max uint256 / 1e6 ≈ 1.15e71 — treat anything above 1e18 as "unlimited".
        let allowance_str = match app.allowance {
            Some(a) if a > 1e18 => "Unlimited".to_string(),
            Some(a) => format!("${:.2}", a),
            None => "—".to_string(),
        };

        let low_allowance = app.allowance.map(|a| a < 10.0).unwrap_or(false);
        let allowance_color = if low_allowance {
            theme::BORDER_WARNING
        } else {
            theme::GREEN
        };

        // ── Portfolio calculations ───────────────────────────────────
        let positions_value: f64 = app
            .positions
            .iter()
            .map(|p| p.size * p.current_price)
            .sum();
        let total_shares: f64 = app.positions.iter().map(|p| p.size).sum();
        let net_worth = bal + positions_value;
        let max_payout = bal + total_shares;

        let label = Style::default().fg(theme::DIM);
        let val = Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD);

        let mut lines = vec![
            Line::from(""),
            // ── Wallet section ───────────────────────────────────────
            Line::from(vec![
                Span::styled("   Wallet ", Style::default().fg(theme::CYAN)),
                Span::styled(
                    "──────────────────────────────",
                    Style::default().fg(theme::BORDER),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("   Cash              ", label),
                Span::styled(format!("${:.2}", bal), val),
            ]),
            Line::from(vec![
                Span::styled("   CTF Allowance     ", label),
                Span::styled(allowance_str.clone(), Style::default().fg(allowance_color)),
            ]),
        ];

        if low_allowance {
            lines.push(Line::from(vec![Span::styled(
                "   ⚠ Low — approve more USDC to place orders",
                Style::default().fg(theme::BORDER_WARNING),
            )]));
        }

        // ── Portfolio section ────────────────────────────────────────
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("   Portfolio ", Style::default().fg(theme::CYAN)),
            Span::styled(
                "───────────────────────────",
                Style::default().fg(theme::BORDER),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("   Positions         ", label),
            Span::styled(format!("${:.2}", positions_value), val),
            Span::styled(
                format!("  ({} open)", app.positions.len()),
                Style::default().fg(theme::VERY_DIM),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("   Shares held       ", label),
            Span::styled(format!("{:.2}", total_shares), Style::default().fg(theme::TEXT)),
        ]));

        // ── Totals section ───────────────────────────────────────────
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("   Totals ", Style::default().fg(theme::CYAN)),
            Span::styled(
                "────────────────────────────",
                Style::default().fg(theme::BORDER),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("   Net Worth         ", label),
            Span::styled(
                format!("${:.2}", net_worth),
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  cash + positions at market",
                Style::default().fg(theme::VERY_DIM),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("   Max Payout        ", label),
            Span::styled(
                format!("${:.2}", max_payout),
                Style::default()
                    .fg(theme::BLUE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  if all shares resolve to $1",
                Style::default().fg(theme::VERY_DIM),
            ),
        ]));

        lines
    };

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

// ── Net worth time-series chart ──────────────────────────────────────────────

fn render_net_worth_chart(f: &mut Frame, area: Rect, app: &App) {
    if area.height < 4 {
        return;
    }

    let block = Block::bordered()
        .title(Span::styled(
            " Net Worth ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    if app.net_worth_history.len() < 3 {
        let msg = if app.net_worth_history.is_empty() {
            "Collecting data… first log in ~30s".to_string()
        } else {
            format!(
                "Collecting data… {}/3 points (logs every 10m)",
                app.net_worth_history.len()
            )
        };
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(msg, Style::default().fg(theme::VERY_DIM)),
            ])),
            inner,
        );
        return;
    }

    let data = &app.net_worth_history;

    // Compute axis bounds.
    let x_min = data.first().map(|d| d.0).unwrap_or(0.0);
    let x_max = data.last().map(|d| d.0).unwrap_or(1.0);
    let y_min = data.iter().map(|d| d.1).fold(f64::INFINITY, f64::min);
    let y_max = data.iter().map(|d| d.1).fold(f64::NEG_INFINITY, f64::max);

    // Add 5% padding to Y axis.
    let y_range = (y_max - y_min).max(1.0);
    let y_lo = (y_min - y_range * 0.05).max(0.0);
    let y_hi = y_max + y_range * 0.05;

    let x_labels = make_time_labels(x_min, x_max);
    let y_labels = make_value_labels(y_lo, y_hi);

    let datasets = vec![Dataset::default()
        .data(data)
        .graph_type(GraphType::Line)
        .marker(symbols::Marker::Braille)
        .style(Style::default().fg(theme::GREEN))];

    let chart = Chart::new(datasets)
        .block(block)
        .style(Style::default().bg(theme::PANEL_BG))
        .x_axis(
            Axis::default()
                .bounds([x_min, x_max])
                .labels(x_labels)
                .style(Style::default().fg(theme::VERY_DIM)),
        )
        .y_axis(
            Axis::default()
                .title(Span::styled("$", Style::default().fg(theme::DIM)))
                .bounds([y_lo, y_hi])
                .labels(y_labels)
                .style(Style::default().fg(theme::VERY_DIM)),
        );

    f.render_widget(chart, area);
}

fn make_time_labels(x_min: f64, x_max: f64) -> Vec<Span<'static>> {
    use chrono::{Local, TimeZone};
    let fmt = if (x_max - x_min) > 86400.0 {
        "%b %d"
    } else {
        "%H:%M"
    };
    let mid = (x_min + x_max) / 2.0;
    [x_min, mid, x_max]
        .iter()
        .map(|&ts| {
            let label = Local
                .timestamp_opt(ts as i64, 0)
                .single()
                .map(|dt| dt.format(fmt).to_string())
                .unwrap_or_default();
            Span::styled(label, Style::default().fg(theme::VERY_DIM))
        })
        .collect()
}

fn make_value_labels(y_lo: f64, y_hi: f64) -> Vec<Span<'static>> {
    let mid = (y_lo + y_hi) / 2.0;
    [y_lo, mid, y_hi]
        .iter()
        .map(|&v| Span::styled(format!("${:.0}", v), Style::default().fg(theme::VERY_DIM)))
        .collect()
}
