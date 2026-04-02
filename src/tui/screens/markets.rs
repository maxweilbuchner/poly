use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::{theme, App};
use crate::types::MarketStatus;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    // Optionally show a search bar at the top when in search mode or query is non-empty.
    let (list_area, search_area) = if app.search_mode || !app.search_query.is_empty() {
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);
        (chunks[1], Some(chunks[0]))
    } else {
        (area, None)
    };

    if let Some(sa) = search_area {
        render_search_bar(f, sa, app);
    }

    render_market_list(f, list_area, app);
}

fn render_search_bar(f: &mut Frame, area: Rect, app: &App) {
    let cursor = if app.search_mode { "▏" } else { "" };
    let block = Block::bordered()
        .title(Span::styled(" Search ", Style::default().fg(theme::CYAN)))
        .border_style(Style::default().fg(if app.search_mode {
            theme::BORDER_ACTIVE
        } else {
            theme::BORDER
        }))
        .style(Style::default().bg(theme::PANEL_BG));

    let text = format!("{}{}", app.search_query, cursor);
    let para = Paragraph::new(text)
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL_BG))
        .block(block);
    f.render_widget(para, area);
}

fn render_market_list(f: &mut Frame, area: Rect, app: &mut App) {
    let filtered = app.filtered_markets();

    let title = if app.search_query.is_empty() {
        " Markets ".to_string()
    } else {
        format!(" Markets ({} results) ", filtered.len())
    };

    let block = Block::bordered()
        .title(Span::styled(title, Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD)))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    if filtered.is_empty() {
        let msg = if app.loading {
            "Loading markets…"
        } else if !app.search_query.is_empty() {
            "No markets match your search."
        } else {
            "No markets found. Press r to refresh."
        };
        let para = Paragraph::new(Span::styled(msg, Style::default().fg(theme::DIM)))
            .block(block)
            .style(Style::default().bg(theme::PANEL_BG));
        f.render_widget(para, area);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|m| {
            let status_style = match m.status {
                MarketStatus::Active => Style::default().fg(theme::GREEN),
                MarketStatus::Closed => Style::default().fg(theme::RED),
                MarketStatus::Unknown => Style::default().fg(theme::DIM),
            };
            let vol = format_volume(m.volume);
            let question = truncate(&m.question, 60);
            // Best Yes/No prices from first two outcomes
            let prices = if m.outcomes.len() >= 2 {
                format!(
                    " Y:{:.2} N:{:.2}",
                    m.outcomes[0].price,
                    m.outcomes[1].price
                )
            } else if m.outcomes.len() == 1 {
                format!(" {:.2}", m.outcomes[0].price)
            } else {
                String::new()
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("{:<60}", question),
                    Style::default().fg(theme::TEXT),
                ),
                Span::styled(
                    format!(" {:>8}", vol),
                    Style::default().fg(theme::YELLOW),
                ),
                Span::styled(prices, Style::default().fg(theme::CYAN)),
                Span::styled(
                    format!(" [{:}]", m.status),
                    status_style,
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

    f.render_stateful_widget(list, area, &mut app.market_list_state);
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}
