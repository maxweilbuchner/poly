use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use crate::tui::{is_auth_error, theme, App};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(10),
        Constraint::Min(0),
    ])
    .split(area);

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
            Line::from(Span::styled("  No data. Press r to refresh.", Style::default().fg(theme::DIM))),
        ]
    } else {
        let balance_str = match app.balance {
            Some(b) => format!("${:.6} USDC", b),
            None => "—".to_string(),
        };

        let allowance_str = match app.allowance {
            Some(a) => format!("${:.6} USDC", a),
            None => "—".to_string(),
        };

        let low_allowance = app.allowance.map(|a| a < 10.0).unwrap_or(false);
        let allowance_color = if low_allowance {
            theme::BORDER_WARNING
        } else {
            theme::GREEN
        };

        let mut lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  USDC Balance  ", Style::default().fg(theme::DIM)),
                Span::styled(
                    balance_str.clone(),
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  CTF Allowance ", Style::default().fg(theme::DIM)),
                Span::styled(
                    allowance_str.clone(),
                    Style::default()
                        .fg(allowance_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ];

        if low_allowance {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "  ⚠ Low allowance — approve more USDC to place orders",
                Style::default().fg(theme::BORDER_WARNING),
            )]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "  r refresh",
            Style::default().fg(theme::VERY_DIM),
        )]));

        lines
    };

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}
