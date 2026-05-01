use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{theme, App, Screen, Tab};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let tabs = [
        (Tab::Markets, "1 Markets"),
        (Tab::Positions, "2 Positions"),
        (Tab::Balance, "3 Balance"),
        (Tab::Analytics, "4 Analytics"),
        (Tab::Viewer, "5 Viewer"),
    ];

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(" ", Style::default().bg(theme::BG)));

    for (i, (tab, label)) in tabs.iter().enumerate() {
        let is_active = &app.active_tab == tab;
        let style = if is_active {
            Style::default()
                .fg(theme::CYAN)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::VERY_DIM).bg(theme::BG)
        };
        spans.push(Span::styled(format!(" {} ", label), style));
        if i < tabs.len() - 1 {
            spans.push(Span::styled(
                "│",
                Style::default().fg(theme::BORDER).bg(theme::BG),
            ));
        }
    }

    // Breadcrumb: when a detail/order screen is open, append "  › <market question>"
    // (and " › Order" for the order entry overlay).
    if let Some(market) = &app.selected_market {
        let on_detail = matches!(
            app.current_screen(),
            Some(Screen::MarketDetail) | Some(Screen::OrderEntry)
        );
        if on_detail {
            spans.push(Span::styled(
                "  ›  ",
                Style::default().fg(theme::VERY_DIM).bg(theme::BG),
            ));
            // Cap breadcrumb to leave room for the right-side WS indicator.
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let right_w = ws_indicator_width(app);
            let avail = (area.width as usize)
                .saturating_sub(used)
                .saturating_sub(right_w + 2)
                .max(8);
            spans.push(Span::styled(
                truncate(&market.question, avail),
                Style::default().fg(theme::TEXT).bg(theme::BG),
            ));
            if matches!(app.current_screen(), Some(Screen::OrderEntry)) {
                spans.push(Span::styled(
                    "  ›  ",
                    Style::default().fg(theme::VERY_DIM).bg(theme::BG),
                ));
                spans.push(Span::styled(
                    "Order",
                    Style::default().fg(theme::CYAN).bg(theme::BG),
                ));
            }
        }
    }

    let right_spans = ws_indicator_spans(app);
    let right_w = right_spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum::<usize>() as u16;

    let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(right_w)]).split(area);

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::BG)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(right_spans))
            .style(Style::default().bg(theme::BG))
            .alignment(ratatui::layout::Alignment::Right),
        chunks[1],
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}

/// Per-channel WS state: only show what's actually connected/active.
/// - book WS is meaningful only when an order book is loaded (a detail is open
///   or we just left one). Green when fresh, yellow when stale (falling back
///   to HTTP polling), red when never received an update.
/// - user WS feeds order/balance updates and is meaningful whenever creds exist.
fn ws_indicator_spans(app: &App) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();

    let book_state = book_ws_state(app);
    if let Some((label, color)) = book_state {
        spans.push(Span::styled(
            format!(" ● book {} ", label),
            Style::default().fg(color).bg(theme::BG),
        ));
    }

    if app.user_ws_connected {
        spans.push(Span::styled(
            " ● user live ",
            Style::default().fg(theme::GREEN).bg(theme::BG),
        ));
    }

    if !spans.is_empty() {
        spans.insert(0, Span::styled(" ", Style::default().bg(theme::BG)));
    }
    spans
}

fn ws_indicator_width(app: &App) -> usize {
    ws_indicator_spans(app)
        .iter()
        .map(|s| s.content.chars().count())
        .sum()
}

fn book_ws_state(app: &App) -> Option<(&'static str, ratatui::style::Color)> {
    // Only meaningful while the book WS task is active.
    app.ws_cancel.as_ref()?;
    match app.order_book_updated_at {
        None => Some(("…", theme::YELLOW)),
        Some(t) => {
            let secs = t.elapsed().as_secs();
            if secs < 15 {
                Some(("live", theme::GREEN))
            } else {
                Some(("polling", theme::YELLOW))
            }
        }
    }
}
