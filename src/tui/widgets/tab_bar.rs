use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{theme, App, Tab};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let tabs = [
        (Tab::Markets, "1 Markets"),
        (Tab::Positions, "2 Positions"),
        (Tab::Balance, "3 Balance"),
        (Tab::Analytics, "4 Analytics"),
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

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(theme::BG));
    f.render_widget(paragraph, area);
}
