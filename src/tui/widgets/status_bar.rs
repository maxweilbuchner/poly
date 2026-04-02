use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{theme, App, Screen};

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    // Check flash expiry and clear if needed.
    if let Some((_, t)) = &app.flash {
        if t.elapsed() >= std::time::Duration::from_secs(3) {
            app.flash = None;
        }
    }

    let chunks = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(20),
        Constraint::Percentage(40),
    ])
    .split(area);

    // Left: context-sensitive key hints
    let hints = key_hints(app);
    let left = Paragraph::new(Line::from(vec![Span::styled(
        hints,
        Style::default().fg(theme::VERY_DIM).bg(theme::BG),
    )]))
    .style(Style::default().bg(theme::BG));
    f.render_widget(left, chunks[0]);

    // Center: flash message or loading indicator
    let center_text = if let Some((msg, _)) = &app.flash {
        Span::styled(msg.clone(), Style::default().fg(theme::YELLOW).bg(theme::BG))
    } else if app.loading {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame = (app.tick / 2) as usize % spinner.len();
        Span::styled(
            format!("{} loading…", spinner[frame]),
            Style::default().fg(theme::DIM).bg(theme::BG),
        )
    } else {
        Span::styled("", Style::default().bg(theme::BG))
    };
    let center = Paragraph::new(Line::from(vec![center_text]))
        .style(Style::default().bg(theme::BG))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(center, chunks[1]);

    // Right: version hint
    let right = Paragraph::new(Line::from(vec![Span::styled(
        " poly v0.1 ",
        Style::default().fg(theme::VERY_DIM).bg(theme::BG),
    )]))
    .style(Style::default().bg(theme::BG))
    .alignment(ratatui::layout::Alignment::Right);
    f.render_widget(right, chunks[2]);
}

fn key_hints(app: &App) -> String {
    match app.current_screen() {
        Some(Screen::MarketList) => {
            if app.search_mode {
                " Esc cancel search  Enter confirm".to_string()
            } else {
                " / search  ↑↓ navigate  Enter select  r refresh  ? help  q quit".to_string()
            }
        }
        Some(Screen::MarketDetail) => {
            " b buy  s sell  r refresh  Esc back  ? help".to_string()
        }
        Some(Screen::OrderEntry) => {
            " Tab next field  Enter submit  d dry-run  Esc cancel".to_string()
        }
        Some(Screen::QuitConfirm) => " y/Enter quit  n/Esc cancel".to_string(),
        Some(Screen::Help) => " Esc close".to_string(),
        None => String::new(),
    }
}
