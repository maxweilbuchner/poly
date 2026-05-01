use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::{theme, App};
use crate::types::Position;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    // When no address is set and the input isn't open, show a single centered
    // prompt instead of two stacked empty panels.
    if app.viewer_address.is_none() && !app.viewer_address_editing {
        render_empty_prompt(f, area, app);
        return;
    }

    let show_recent = app.viewer_address_editing && !app.viewer_recent.is_empty();
    let recent_h = if show_recent {
        (app.viewer_recent.len().min(8) as u16) + 2
    } else {
        0
    };

    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(recent_h),
        Constraint::Min(0),
    ])
    .split(area);

    render_address_bar(f, chunks[0], app);
    if show_recent {
        render_recent_list(f, chunks[1], app);
    }
    render_viewer_positions(f, chunks[2], app);
}

fn render_recent_list(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered()
        .title(Span::styled(
            " Recent ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines: Vec<Line<'static>> = app
        .viewer_recent
        .iter()
        .take(8)
        .enumerate()
        .map(|(i, addr)| {
            let selected = app.viewer_recent_selected == Some(i);
            let (marker, style) = if selected {
                (
                    "▸ ",
                    Style::default()
                        .fg(theme::CYAN)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("  ", Style::default().fg(theme::TEXT))
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(theme::CYAN)),
                Span::styled(abbreviate_address(addr), style),
                Span::raw("  "),
                Span::styled(addr.clone(), Style::default().fg(theme::VERY_DIM)),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_empty_prompt(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered()
        .title(Span::styled(
            " Viewer ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Centered prompt — pad the top so the message sits roughly mid-panel.
    let top_pad = (inner.height as usize / 2).saturating_sub(1);
    let mut lines: Vec<Line<'static>> = (0..top_pad).map(|_| Line::from("")).collect();
    lines.push(Line::from(vec![Span::styled(
        "View any Polymarket wallet's portfolio",
        Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(theme::DIM)),
        Span::styled("/", Style::default().fg(theme::CYAN)),
        Span::styled(" to enter an address", Style::default().fg(theme::DIM)),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "Example: 0x123... or vitalik.eth",
        Style::default().fg(theme::VERY_DIM),
    )]));

    if !app.viewer_recent.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Recent",
            Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
        )]));
        for addr in app.viewer_recent.iter().take(5) {
            lines.push(Line::from(vec![Span::styled(
                abbreviate_address(addr),
                Style::default().fg(theme::TEXT),
            )]));
        }
    }

    f.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        inner,
    );
}

// ── Address input bar ────────────────────────────────────────────────────────

fn render_address_bar(f: &mut Frame, area: Rect, app: &App) {
    let border_color = if app.viewer_address_editing {
        theme::CYAN
    } else {
        theme::BORDER
    };

    let block = Block::bordered()
        .title(Span::styled(
            " Viewer ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.viewer_address_editing {
        let cursor = if app.tick % 20 < 10 { "▎" } else { " " };
        let line = Line::from(vec![
            Span::styled("  Address: ", Style::default().fg(theme::DIM)),
            Span::styled(
                app.viewer_address_input.clone(),
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(cursor, Style::default().fg(theme::CYAN)),
            Span::raw("  "),
            Span::styled("Enter", Style::default().fg(theme::CYAN)),
            Span::styled(" submit  ", Style::default().fg(theme::HINT)),
            Span::styled("↑↓", Style::default().fg(theme::CYAN)),
            Span::styled(" recent  ", Style::default().fg(theme::HINT)),
            Span::styled("Esc", Style::default().fg(theme::CYAN)),
            Span::styled(" cancel", Style::default().fg(theme::HINT)),
        ]);
        f.render_widget(Paragraph::new(line), inner);
    } else if let Some(addr) = &app.viewer_address {
        let display = abbreviate_address(addr);
        let line = Line::from(vec![
            Span::styled("  Address: ", Style::default().fg(theme::DIM)),
            Span::styled(
                display,
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("/", Style::default().fg(theme::CYAN)),
            Span::styled(" new address  ", Style::default().fg(theme::HINT)),
            Span::styled("r", Style::default().fg(theme::CYAN)),
            Span::styled(" refresh", Style::default().fg(theme::HINT)),
        ]);
        f.render_widget(Paragraph::new(line), inner);
    } else {
        let line = Line::from(vec![
            Span::styled(
                "  Enter a wallet address to view their portfolio",
                Style::default().fg(theme::DIM),
            ),
            Span::raw("  "),
            Span::styled("/", Style::default().fg(theme::CYAN)),
            Span::styled(" start", Style::default().fg(theme::HINT)),
        ]);
        f.render_widget(Paragraph::new(line), inner);
    }
}

// ── Positions list ───────────────────────────────────────────────────────────

fn render_viewer_positions(f: &mut Frame, area: Rect, app: &mut App) {
    if app.viewer_address.is_none() {
        let block = Block::bordered()
            .title(Span::styled(
                " Positions ",
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(theme::BORDER))
            .style(Style::default().bg(theme::PANEL_BG));
        f.render_widget(
            Paragraph::new(Span::styled(
                "  Press / to enter an address",
                Style::default().fg(theme::VERY_DIM),
            ))
            .block(block),
            area,
        );
        return;
    }

    let portfolio_value: f64 = app
        .viewer_positions
        .iter()
        .map(|p| p.size * p.current_price)
        .sum();
    let count = app.viewer_positions.len();

    let title = Line::from(vec![
        Span::styled(
            format!(" Positions ({}) ", count),
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("Portfolio: ${:.2} ", portfolio_value),
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    if app.viewer_positions.is_empty() {
        let msg = if app.loading {
            "Loading…"
        } else {
            "No positions found for this address."
        };
        f.render_widget(
            Paragraph::new(Span::styled(msg, Style::default().fg(theme::DIM))).block(block),
            area,
        );
        return;
    }

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Summary line
    let inner_chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    let cost_basis: f64 = app
        .viewer_positions
        .iter()
        .map(|p| p.size * p.avg_price)
        .sum();
    let total_unreal: f64 = app.viewer_positions.iter().map(|p| p.unrealized_pnl).sum();
    let return_pct = if cost_basis > 0.0 {
        (portfolio_value - cost_basis) / cost_basis * 100.0
    } else {
        0.0
    };

    let pnl_color = |v: f64| if v >= 0.0 { theme::GREEN } else { theme::RED };
    let sign = |v: f64| if v >= 0.0 { "+" } else { "" };

    let line1 = Line::from(vec![
        Span::styled("  P&L ", Style::default().fg(theme::DIM)),
        Span::styled(
            format!("{}{:.4}", sign(total_unreal), total_unreal),
            Style::default()
                .fg(pnl_color(total_unreal))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" unreal", Style::default().fg(theme::VERY_DIM)),
        Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
        Span::styled(
            format!("{}{:.1}%", sign(return_pct), return_pct),
            Style::default()
                .fg(pnl_color(return_pct))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" return", Style::default().fg(theme::VERY_DIM)),
    ]);
    let line2 = Line::from(vec![
        Span::styled(
            format!("  {} position{}", count, if count == 1 { "" } else { "s" }),
            Style::default().fg(theme::DIM),
        ),
        Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
        Span::styled("value ", Style::default().fg(theme::DIM)),
        Span::styled(
            format!("${:.2}", portfolio_value),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
        Span::styled("cost ", Style::default().fg(theme::DIM)),
        Span::styled(
            format!("${:.2}", cost_basis),
            Style::default().fg(theme::TEXT),
        ),
    ]);

    f.render_widget(Paragraph::new(vec![line1, line2]), inner_chunks[0]);

    let (header, items) = build_viewer_items(&app.viewer_positions, area.width as usize);
    f.render_widget(Paragraph::new(header), inner_chunks[1]);

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(32, 38, 72))
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, inner_chunks[2], &mut app.viewer_list_state);
}

fn build_viewer_items(
    positions: &[Position],
    area_width: usize,
) -> (Line<'static>, Vec<ListItem<'static>>) {
    // borders(2) + highlight(2) + indent(2) = 6 overhead
    let q_width = area_width.saturating_sub(6).max(20);

    struct VRow {
        question: String,
        outcome: String,
        size_str: String,
        price_str: String,
        value_str: String,
        pnl_str: String,
        pnl_color: Color,
        cur_color: Color,
    }

    let rows: Vec<VRow> = positions
        .iter()
        .map(|p| {
            let value = p.size * p.current_price;
            let pnl_sign = if p.unrealized_pnl >= 0.0 { "+" } else { "" };
            VRow {
                question: truncate(&p.market_question, q_width),
                outcome: p.outcome.clone(),
                size_str: format!("{:.2}", p.size),
                price_str: format!("@{:.4}", p.current_price),
                value_str: format!("${:.2}", value),
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
            }
        })
        .collect();

    let max_size = rows.iter().map(|r| r.size_str.len()).max().unwrap_or(4);
    let max_price = rows.iter().map(|r| r.price_str.len()).max().unwrap_or(7);
    let max_value = rows.iter().map(|r| r.value_str.len()).max().unwrap_or(6);
    let max_pnl = rows.iter().map(|r| r.pnl_str.len()).max().unwrap_or(8);

    // Fixed: indent(2) + seps(4*4=16) + " shares"(7) + size + price + value + pnl
    let fixed = 2 + 16 + max_size + 7 + max_price + max_value + max_pnl;
    let outcome_width = area_width.saturating_sub(4).saturating_sub(fixed).max(4);

    // Column header — 4 = highlight_symbol(2) + item indent(2)
    let size_hdr_w = max_size + 7;
    let header = Line::from(vec![
        Span::raw("    "),
        Span::styled(
            pad_right("Outcome".to_string(), outcome_width),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_right("Shares".to_string(), size_hdr_w),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_right("Price".to_string(), max_price),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_right("Value".to_string(), max_value),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_right("P&L".to_string(), max_pnl),
            Style::default().fg(theme::DIM),
        ),
    ]);

    let items = rows
        .into_iter()
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

            let outcome_cell = pad_right(truncate(&r.outcome, outcome_width), outcome_width);
            let size_cell = format!("{:>width$} shares", r.size_str, width = max_size);
            let price_cell = pad_right(r.price_str, max_price);
            let value_cell = pad_right(r.value_str, max_value);
            let pnl_cell = pad_right(r.pnl_str, max_pnl);

            let line2 = Line::from(vec![
                Span::raw("  "),
                Span::styled(outcome_cell, Style::default().fg(theme::DIM)),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(size_cell, Style::default().fg(theme::DIM)),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(price_cell, Style::default().fg(r.cur_color)),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(
                    value_cell,
                    Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
                Span::styled(
                    pnl_cell,
                    Style::default()
                        .fg(r.pnl_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);

            ListItem::new(vec![line1, line2])
        })
        .collect();

    (header, items)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn abbreviate_address(addr: &str) -> String {
    if addr.len() > 12 {
        format!("{}…{}", &addr[..6], &addr[addr.len() - 4..])
    } else {
        addr.to_string()
    }
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
