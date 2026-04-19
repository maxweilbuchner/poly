use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use crate::tui::{is_auth_error, theme, App};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(22), Constraint::Min(0)]).split(area);

    render_balance_panel(f, chunks[0], app);
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
