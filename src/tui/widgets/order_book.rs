use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Block, Row, Table},
    Frame,
};

use crate::tui::theme;
use crate::types::OrderBook;

/// Render an order book (bid/ask ladder) into `area`.
///
/// Shows up to `levels` price levels, bids green, asks red.
pub fn render_with_selection(
    f: &mut Frame,
    area: Rect,
    book: Option<&OrderBook>,
    label: &str,
    levels: usize,
    selected: bool,
) {
    let title = if selected {
        format!(" ▸ {} ", label)
    } else {
        format!(" {} ", label)
    };
    let border_color = if selected { theme::CYAN } else { theme::BORDER };
    let block = Block::bordered()
        .title(ratatui::text::Span::styled(
            title,
            Style::default().fg(border_color).add_modifier(if selected {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ))
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme::PANEL_BG));

    // Header row — group labels (Asks / Bids) over price+size.
    let header = Row::new(vec![
        ratatui::text::Span::styled(
            "Ask",
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        ),
        ratatui::text::Span::styled(
            "Size",
            Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
        ),
        ratatui::text::Span::styled("│", Style::default().fg(theme::BORDER)),
        ratatui::text::Span::styled(
            "Bid",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        ratatui::text::Span::styled(
            "Size",
            Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
        ),
    ]);

    let rows: Vec<Row> = match book {
        None => vec![Row::new(vec!["—", "", "", "", ""])],
        Some(book) => {
            let max = levels.max(1);
            let ask_levels: Vec<_> = book.asks.iter().take(max).collect();
            let bid_levels: Vec<_> = book.bids.iter().take(max).collect();
            let count = ask_levels.len().max(bid_levels.len());

            (0..count)
                .map(|i| {
                    let ask_price = ask_levels
                        .get(i)
                        .map(|l| format!("{:.4}", l.price))
                        .unwrap_or_default();
                    let ask_size = ask_levels
                        .get(i)
                        .map(|l| format!("{:.2}", l.size))
                        .unwrap_or_default();
                    let bid_price = bid_levels
                        .get(i)
                        .map(|l| format!("{:.4}", l.price))
                        .unwrap_or_default();
                    let bid_size = bid_levels
                        .get(i)
                        .map(|l| format!("{:.2}", l.size))
                        .unwrap_or_default();

                    let ask_style = Style::default().fg(theme::RED);
                    let bid_style = Style::default().fg(theme::GREEN);
                    let sep_style = Style::default().fg(theme::BORDER);

                    Row::new(vec![
                        ratatui::text::Span::styled(ask_price, ask_style),
                        ratatui::text::Span::styled(ask_size, ask_style),
                        ratatui::text::Span::styled("│", sep_style),
                        ratatui::text::Span::styled(bid_price, bid_style),
                        ratatui::text::Span::styled(bid_size, bid_style),
                    ])
                })
                .collect()
        }
    };

    let widths = [
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(1),
        Constraint::Length(8),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1);

    f.render_widget(table, area);
}
