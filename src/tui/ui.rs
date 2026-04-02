use ratatui::{
    layout::{Constraint, Layout, Rect},
    Frame,
};

use super::{
    screens::{balance, detail, markets, order, positions},
    widgets::{status_bar, tab_bar},
    App, Screen, Tab,
};

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.size();

    // Three-row layout: tab bar / content / status bar
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    tab_bar::render(f, chunks[0], app);
    status_bar::render(f, chunks[2], app);

    // Render content based on active tab
    let content = chunks[1];
    match &app.active_tab {
        Tab::Markets => render_markets_content(f, content, app),
        Tab::Positions => positions::render(f, content, app),
        Tab::Balance => balance::render(f, content, app),
    }

    // Render global modal overlays on top of everything
    let full = f.size();
    match app.current_screen() {
        Some(Screen::QuitConfirm) => render_quit_confirm(f, full, app),
        Some(Screen::Help) => render_help(f, full),
        _ => {}
    }
}

fn render_markets_content(f: &mut Frame, area: Rect, app: &mut App) {
    match app.current_screen().cloned() {
        Some(Screen::MarketDetail) | Some(Screen::OrderEntry) => {
            detail::render(f, area, app);
            if matches!(app.current_screen(), Some(Screen::OrderEntry)) {
                order::render(f, f.size(), app);
            }
        }
        _ => {
            markets::render(f, area, app);
        }
    }
}

// ── Modal helpers ─────────────────────────────────────────────────────────────

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

fn render_quit_confirm(f: &mut Frame, area: Rect, _app: &App) {
    use ratatui::{
        style::{Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Clear, Paragraph},
    };
    use super::theme;

    let modal = centered_rect(40, 20, area);
    f.render_widget(Clear, modal);

    let block = Block::bordered()
        .title(Span::styled(
            " Quit poly? ",
            Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL_BG));

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Press ", Style::default().fg(theme::DIM)),
            Span::styled("y", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
            Span::styled(" or ", Style::default().fg(theme::DIM)),
            Span::styled("Enter", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
            Span::styled(" to quit", Style::default().fg(theme::DIM)),
        ]),
        Line::from(vec![
            Span::styled("  Press ", Style::default().fg(theme::DIM)),
            Span::styled("n", Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)),
            Span::styled(" or ", Style::default().fg(theme::DIM)),
            Span::styled("Esc", Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)),
            Span::styled(" to cancel", Style::default().fg(theme::DIM)),
        ]),
    ];

    let para = Paragraph::new(text).block(block);
    f.render_widget(para, modal);
}

fn render_help(f: &mut Frame, area: Rect) {
    use ratatui::{
        style::{Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Clear, Paragraph},
    };
    use super::theme;

    let modal = centered_rect(60, 70, area);
    f.render_widget(Clear, modal);

    let block = Block::bordered()
        .title(Span::styled(
            " Key Bindings ",
            Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL_BG));

    let entries: &[(&str, &str)] = &[
        ("1 / 2 / 3", "Switch to Markets / Positions / Balance"),
        ("Tab", "Cycle tabs or switch panels"),
        ("↑ ↓ / j k", "Navigate lists"),
        ("/", "Enter search mode (Markets)"),
        ("Enter", "Select / confirm"),
        ("Esc / h", "Go back / close modal"),
        ("r", "Refresh current screen"),
        ("b", "Place buy order (Market Detail)"),
        ("s", "Place sell order (Market Detail)"),
        ("c", "Cancel highlighted order (Positions)"),
        ("C", "Cancel all orders (Positions)"),
        ("d", "Toggle dry-run (Order Entry)"),
        ("Space", "Cycle order type (Order Entry)"),
        ("q", "Quit menu"),
        ("?", "This help screen"),
        ("Ctrl+C", "Force quit"),
    ];

    let mut lines = vec![Line::from("")];
    for (key, desc) in entries {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:>12}  ", key),
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(*desc, Style::default().fg(theme::TEXT)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  Esc to close",
        Style::default().fg(theme::VERY_DIM),
    )]));

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, modal);
}
