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
    let chunks = Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(area);

    render_summary_panel(f, chunks[0], app);
    render_net_worth_chart(f, chunks[1], app);
}

fn render_summary_panel(f: &mut Frame, area: Rect, app: &App) {
    let empty_block = Block::bordered()
        .title(Span::styled(
            " Summary ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    if app.loading && app.balance.is_none() {
        f.render_widget(
            Paragraph::new(Span::styled("  Loading…", Style::default().fg(theme::DIM)))
                .block(empty_block),
            area,
        );
        return;
    }

    if app.balance.is_none() {
        if let Some(err) = &app.last_error {
            if is_auth_error(err) {
                let err_str = err.to_string();
                let mut lines = vec![];
                for raw in err_str.lines() {
                    let line = raw.trim_start_matches("  ");
                    if line.starts_with("Hint:") {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(line.to_string(), Style::default().fg(theme::YELLOW)),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(
                                line.to_string(),
                                Style::default()
                                    .fg(theme::ERROR)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]));
                    }
                }
                f.render_widget(Paragraph::new(lines).block(empty_block), area);
                return;
            }
        }
        f.render_widget(
            Paragraph::new(Span::styled(
                "  No data. Press r to refresh.",
                Style::default().fg(theme::DIM),
            ))
            .block(empty_block),
            area,
        );
        return;
    }

    let bal = app.balance.unwrap_or(0.0);

    let low_allowance = app.allowance.map(|a| a < 10.0).unwrap_or(false);

    let positions_value: f64 = app.positions.iter().map(|p| p.size * p.current_price).sum();
    let total_shares: f64 = app.positions.iter().map(|p| p.size).sum();
    let net_worth = bal + positions_value;
    let max_payout = bal + total_shares;

    let pos_count = app.positions.len();

    // Return vs first net-worth log entry; annualized over the elapsed window.
    let (pd_str, pd_color, pm_str, pm_color, pa_str, pa_color) = match app.net_worth_history.first()
    {
        Some(&(t0, nw0)) if nw0 > 0.0 && app.net_worth_history.len() >= 2 => {
            let t1 = app.net_worth_history.last().map(|&(t, _)| t).unwrap_or(t0);
            let r = (net_worth - nw0) / nw0;
            let elapsed = (t1 - t0).max(1.0);

            let days = elapsed / 86400.0;
            let months = elapsed / (30.4375 * 86400.0);
            let years = elapsed / (365.25 * 86400.0);

            let calc_ret = |t: f64| {
                if t > 0.0 && (1.0 + r) > 0.0 {
                    (1.0 + r).powf(1.0 / t) - 1.0
                } else {
                    0.0
                }
            };

            let pd = calc_ret(days);
            let pm = calc_ret(months);
            let pa = calc_ret(years);

            let color = |v: f64| if v >= 0.0 { theme::GREEN } else { theme::RED };
            let sign = |v: f64| if v >= 0.0 { "+" } else { "" };
            (
                format!("{}{:.2}%", sign(pd), pd * 100.0),
                color(pd),
                format!("{}{:.2}%", sign(pm), pm * 100.0),
                color(pm),
                format!("{}{:.2}%", sign(pa), pa * 100.0),
                color(pa),
            )
        }
        _ => (
            "—".to_string(),
            theme::DIM,
            "—".to_string(),
            theme::DIM,
            "—".to_string(),
            theme::DIM,
        ),
    };

    // Title with metadata
    let mut title_spans = vec![Span::styled(
        " Summary ",
        Style::default()
            .fg(theme::CYAN)
            .add_modifier(Modifier::BOLD),
    )];
    if low_allowance {
        title_spans.push(Span::styled(
            "· ⚠ low allowance ",
            Style::default().fg(theme::BORDER_WARNING),
        ));
    }

    let block = Block::bordered()
        .title(Line::from(title_spans))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let labels = [
        "Net Worth",
        "Cash",
        "Positions",
        "Max Payout",
        "Return p.d.",
        "Return p.m.",
        "Return p.a.",
    ];
    let value_strs = [
        format!("${:.2}", net_worth),
        format!("${:.2}", bal),
        format!("${:.2} ({})", positions_value, pos_count),
        format!("${:.2}", max_payout),
        pd_str,
        pm_str,
        pa_str,
    ];
    let value_colors = [
        theme::GREEN,
        theme::TEXT,
        theme::TEXT,
        theme::BLUE,
        pd_color,
        pm_color,
        pa_color,
    ];
    let value_bold = [true, true, true, true, true, true, true];

    let col_w = (inner.width as usize).saturating_sub(2) / labels.len();

    let mut label_spans: Vec<Span<'static>> = vec![Span::raw("  ")];
    for lbl in &labels {
        label_spans.push(Span::styled(
            pad_right(lbl.to_string(), col_w),
            Style::default().fg(theme::DIM),
        ));
    }

    let mut val_spans: Vec<Span<'static>> = vec![Span::raw("  ")];
    for (i, val) in value_strs.iter().enumerate() {
        let mut style = Style::default().fg(value_colors[i]);
        if value_bold[i] {
            style = style.add_modifier(Modifier::BOLD);
        }
        val_spans.push(Span::styled(pad_right(val.clone(), col_w), style));
    }

    f.render_widget(
        Paragraph::new(vec![Line::from(label_spans), Line::from(val_spans)]),
        inner,
    );
}

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

    let data: Vec<(f64, f64)> = app
        .net_worth_history
        .iter()
        .filter(|&&(_, y)| y > 0.0)
        .copied()
        .collect();

    if data.len() < 3 {
        let msg = if data.is_empty() {
            "Collecting data… first log in ~30s".to_string()
        } else {
            format!("Collecting data… {}/3 points (logs every 10m)", data.len())
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
        .data(&data)
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
