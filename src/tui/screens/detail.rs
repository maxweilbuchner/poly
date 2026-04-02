use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use crate::tui::{theme, App};
use crate::tui::widgets::order_book;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    // Split into header + order books
    let chunks = Layout::vertical([
        Constraint::Length(8),
        Constraint::Min(0),
    ])
    .split(area);

    render_header(f, chunks[0], app);
    render_books(f, chunks[1], app);
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered()
        .title(Span::styled(" Market Detail ", Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD)))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    let lines = match &app.selected_market {
        None => vec![
            Line::from(""),
            Line::from(Span::styled("  Loading…", Style::default().fg(theme::DIM))),
        ],
        Some(m) => {
            let vol = format_volume(m.volume);
            let liq = format_volume(m.liquidity);
            let end = m.end_date.as_deref().unwrap_or("—");
            let cat = m.category.as_deref().unwrap_or("—");
            vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(&m.question, Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Status: ", Style::default().fg(theme::DIM)),
                    Span::styled(m.status.to_string(), Style::default().fg(theme::CYAN)),
                    Span::styled("   Vol: ", Style::default().fg(theme::DIM)),
                    Span::styled(vol, Style::default().fg(theme::YELLOW)),
                    Span::styled("   Liq: ", Style::default().fg(theme::DIM)),
                    Span::styled(liq, Style::default().fg(theme::YELLOW)),
                ]),
                Line::from(vec![
                    Span::styled("  Category: ", Style::default().fg(theme::DIM)),
                    Span::styled(cat, Style::default().fg(theme::TEXT)),
                    Span::styled("   Ends: ", Style::default().fg(theme::DIM)),
                    Span::styled(end, Style::default().fg(theme::TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  ID: ", Style::default().fg(theme::DIM)),
                    Span::styled(&m.condition_id, Style::default().fg(theme::VERY_DIM)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  b buy  s sell  r refresh  Esc back",
                        Style::default().fg(theme::VERY_DIM),
                    ),
                ]),
            ]
        }
    };

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

fn render_books(f: &mut Frame, area: Rect, app: &mut App) {
    if app.order_books.is_empty() {
        let block = Block::bordered()
            .title(Span::styled(" Order Books ", Style::default().fg(theme::CYAN)))
            .border_style(Style::default().fg(theme::BORDER))
            .style(Style::default().bg(theme::PANEL_BG));
        let msg = if app.loading { "Loading…" } else { "No order book data." };
        let para = Paragraph::new(Span::styled(msg, Style::default().fg(theme::DIM))).block(block);
        f.render_widget(para, area);
        return;
    }

    // Split horizontally for each outcome
    let n = app.order_books.len().min(4) as u16;
    let constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();
    let cols = Layout::horizontal(constraints).split(area);

    for (i, (label, book)) in app.order_books.iter().enumerate() {
        if i >= cols.len() {
            break;
        }
        order_book::render(f, cols[i], Some(book), label, 10);
    }
}

fn format_volume(v: f64) -> String {
    if v >= 1_000_000.0 {
        format!("${:.1}M", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("${:.1}K", v / 1_000.0)
    } else {
        format!("${:.2}", v)
    }
}

