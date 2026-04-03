use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::{is_auth_error, theme, App};

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ])
    .split(area);

    render_positions(f, chunks[0], app);
    render_orders(f, chunks[1], app);
}

fn render_positions(f: &mut Frame, area: Rect, app: &mut App) {
    let focused = !app.positions_focus_orders;
    let border_color = if focused { theme::BORDER_ACTIVE } else { theme::BORDER };

    let block = Block::bordered()
        .title(Span::styled(
            " Positions ",
            Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme::PANEL_BG));

    if app.positions.is_empty() {
        let para = if app.loading {
            Paragraph::new(Span::styled("Loading…", Style::default().fg(theme::DIM))).block(block)
        } else if let Some(err) = &app.last_error {
            if is_auth_error(err) {
                auth_error_paragraph(err, block)
            } else {
                Paragraph::new(Span::styled("No open positions.", Style::default().fg(theme::DIM))).block(block)
            }
        } else {
            Paragraph::new(Span::styled("No open positions.", Style::default().fg(theme::DIM))).block(block)
        };
        f.render_widget(para, area);
        return;
    }

    let items: Vec<ListItem> = app
        .positions
        .iter()
        .map(|p| {
            let pnl_color = if p.unrealized_pnl >= 0.0 { theme::GREEN } else { theme::RED };
            let pnl_sign = if p.unrealized_pnl >= 0.0 { "+" } else { "" };
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<45}", truncate(&p.market_question, 44)),
                    Style::default().fg(theme::TEXT),
                ),
                Span::styled(
                    format!(" {:>8}", p.outcome),
                    Style::default().fg(theme::CYAN),
                ),
                Span::styled(
                    format!(" {:>7.2}", p.size),
                    Style::default().fg(theme::TEXT),
                ),
                Span::styled(
                    format!(" @{:.4}", p.avg_price),
                    Style::default().fg(theme::DIM),
                ),
                Span::styled(
                    format!(" {}{:.4}", pnl_sign, p.unrealized_pnl),
                    Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(theme::BORDER_ACTIVE)
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut app.positions_list_state);
}

fn render_orders(f: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.positions_focus_orders;
    let border_color = if focused { theme::BORDER_ACTIVE } else { theme::BORDER };

    let title = if focused {
        " Open Orders [c cancel  C cancel-all] "
    } else {
        " Open Orders "
    };

    let block = Block::bordered()
        .title(Span::styled(
            title,
            Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme::PANEL_BG));

    if app.orders.is_empty() {
        let para = if app.loading {
            Paragraph::new(Span::styled("Loading…", Style::default().fg(theme::DIM))).block(block)
        } else if let Some(err) = &app.last_error {
            if is_auth_error(err) {
                auth_error_paragraph(err, block)
            } else {
                Paragraph::new(Span::styled("No open orders.", Style::default().fg(theme::DIM))).block(block)
            }
        } else {
            Paragraph::new(Span::styled("No open orders.", Style::default().fg(theme::DIM))).block(block)
        };
        f.render_widget(para, area);
        return;
    }

    let items: Vec<ListItem> = app
        .orders
        .iter()
        .map(|o| {
            let side_color = match o.side {
                crate::types::Side::Buy => theme::GREEN,
                crate::types::Side::Sell => theme::RED,
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<20}", truncate(&o.id, 19)),
                    Style::default().fg(theme::VERY_DIM),
                ),
                Span::styled(
                    format!(" {:>4}", o.side),
                    Style::default().fg(side_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" @{:.4}", o.price),
                    Style::default().fg(theme::TEXT),
                ),
                Span::styled(
                    format!(" {:>8.2}", o.original_size),
                    Style::default().fg(theme::TEXT),
                ),
                Span::styled(
                    format!(" filled: {:.2}", o.size_matched),
                    Style::default().fg(theme::DIM),
                ),
                Span::styled(
                    format!(" [{}]", o.status),
                    Style::default().fg(theme::BLUE),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(theme::BORDER_ACTIVE)
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut app.orders_list_state);
}

/// Build a Paragraph that shows an auth/credentials error persistently inside a panel.
fn auth_error_paragraph<'a>(err: &'a str, block: Block<'a>) -> Paragraph<'a> {
    // Split on the hint line so we can colour them differently.
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
