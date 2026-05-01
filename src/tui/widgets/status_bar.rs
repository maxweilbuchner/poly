use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{theme, App, Screen, Tab};

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    // Check flash expiry and clear if needed.
    if let Some((_, t, is_err)) = &app.flash {
        let ttl = if *is_err { 5 } else { 3 };
        if t.elapsed() >= std::time::Duration::from_secs(ttl) {
            app.flash = None;
        }
    }

    let spinner_chars = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let spin = spinner_chars[(app.tick / 2) as usize % spinner_chars.len()];

    // Flash: give the full row (minus version) to the message so it's never clipped.
    if let Some((msg, _, is_err)) = &app.flash {
        let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(16)]).split(area);

        let color = if *is_err { theme::ERROR } else { theme::YELLOW };
        let left = Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(msg.clone(), Style::default().fg(color).bg(theme::BG)),
        ]))
        .style(Style::default().bg(theme::BG));
        f.render_widget(left, chunks[0]);

        render_version(f, chunks[1], app.user_ws_connected);
        return;
    }

    // Build spinner label (if any). Tab-specific labels tell users what's actually
    // being fetched instead of a generic "fetching…" that lingers without context.
    let spinner_label: Option<String> = if app.loading {
        let what = match app.active_tab {
            Tab::Markets => "markets",
            Tab::Positions => "positions",
            Tab::Balance => "balance",
            Tab::Analytics => "analytics",
            Tab::Viewer => "portfolio",
        };
        Some(format!(" {} loading {}…  ", spin, what))
    } else if app.markets_loading_more {
        Some(format!(" {} more markets…  ", spin))
    } else {
        None
    };

    let spin_w = spinner_label
        .as_ref()
        .map(|s| s.chars().count() as u16)
        .unwrap_or(0);

    // Auth warning column — shown persistently whenever credentials were probed and rejected.
    const AUTH_WARN_W: u16 = 20; // " ⚠ creds invalid  " — fixed width keeps layout stable
    let auth_warn = app.auth_warning.as_ref().map(|_| " ⚠ creds invalid  ");

    match (spin_w > 0, auth_warn) {
        (true, Some(aw)) => {
            // Four-column: key hints | spinner | auth warn | version
            let chunks = Layout::horizontal([
                Constraint::Min(0),
                Constraint::Length(spin_w),
                Constraint::Length(AUTH_WARN_W),
                Constraint::Length(16),
            ])
            .split(area);
            f.render_widget(
                Paragraph::new(Line::from(key_hint_spans(app)))
                    .style(Style::default().bg(theme::BG)),
                chunks[0],
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    spinner_label.clone().unwrap_or_default(),
                    Style::default().fg(theme::VERY_DIM).bg(theme::BG),
                )]))
                .style(Style::default().bg(theme::BG)),
                chunks[1],
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    aw,
                    Style::default().fg(theme::ERROR).bg(theme::BG),
                )]))
                .style(Style::default().bg(theme::BG)),
                chunks[2],
            );
            render_version(f, chunks[3], app.user_ws_connected);
        }
        (true, None) => {
            // Three-column: key hints | spinner | version
            let chunks = Layout::horizontal([
                Constraint::Min(0),
                Constraint::Length(spin_w),
                Constraint::Length(16),
            ])
            .split(area);
            f.render_widget(
                Paragraph::new(Line::from(key_hint_spans(app)))
                    .style(Style::default().bg(theme::BG)),
                chunks[0],
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    spinner_label.unwrap_or_default(),
                    Style::default().fg(theme::VERY_DIM).bg(theme::BG),
                )]))
                .style(Style::default().bg(theme::BG)),
                chunks[1],
            );
            render_version(f, chunks[2], app.user_ws_connected);
        }
        (false, Some(aw)) => {
            // Three-column: key hints | auth warn | version
            let chunks = Layout::horizontal([
                Constraint::Min(0),
                Constraint::Length(AUTH_WARN_W),
                Constraint::Length(16),
            ])
            .split(area);
            f.render_widget(
                Paragraph::new(Line::from(key_hint_spans(app)))
                    .style(Style::default().bg(theme::BG)),
                chunks[0],
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    aw,
                    Style::default().fg(theme::ERROR).bg(theme::BG),
                )]))
                .style(Style::default().bg(theme::BG)),
                chunks[1],
            );
            render_version(f, chunks[2], app.user_ws_connected);
        }
        (false, None) => {
            // Two-column: key hints | version
            let chunks =
                Layout::horizontal([Constraint::Min(0), Constraint::Length(16)]).split(area);
            f.render_widget(
                Paragraph::new(Line::from(key_hint_spans(app)))
                    .style(Style::default().bg(theme::BG)),
                chunks[0],
            );
            render_version(f, chunks[1], app.user_ws_connected);
        }
    }
}

fn render_version(f: &mut Frame, area: Rect, _ws_connected: bool) {
    // WS state is now shown in the tab bar; this row keeps just the version.
    let spans = vec![Span::styled(
        concat!("poly v", env!("CARGO_PKG_VERSION"), " "),
        Style::default().fg(theme::VERY_DIM).bg(theme::BG),
    )];
    let v = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(theme::BG))
        .alignment(ratatui::layout::Alignment::Right);
    f.render_widget(v, area);
}

/// Push a styled key+action pair onto a span vec.
/// key is rendered in CYAN; action is rendered in VERY_DIM followed by two spaces.
fn push_hint(spans: &mut Vec<Span<'static>>, key: &'static str, action: &'static str) {
    spans.push(Span::styled(key, Style::default().fg(theme::CYAN)));
    spans.push(Span::styled(
        format!(" {}  ", action),
        Style::default().fg(theme::VERY_DIM),
    ));
}

/// Like `push_hint` but takes an owned action string (e.g. when the label includes runtime state).
fn push_hint_dyn(spans: &mut Vec<Span<'static>>, key: &'static str, action: String) {
    spans.push(Span::styled(key, Style::default().fg(theme::CYAN)));
    spans.push(Span::styled(
        format!(" {}  ", action),
        Style::default().fg(theme::VERY_DIM),
    ));
}

fn key_hint_spans(app: &App) -> Vec<Span<'static>> {
    let mut s: Vec<Span<'static>> = vec![Span::raw(" ")];

    // Modal overlays take priority regardless of active tab.
    match app.current_screen() {
        Some(Screen::OrderEntry) => {
            push_hint(&mut s, "Tab", "next field");
            push_hint(&mut s, "Enter", "submit");
            push_hint(&mut s, "d", "dry-run");
            push_hint(&mut s, "Esc", "cancel");
            return s;
        }
        Some(Screen::CloseConfirm) => {
            push_hint(&mut s, "y", "confirm");
            push_hint(&mut s, "r", "retry price");
            push_hint(&mut s, "Esc", "cancel");
            return s;
        }
        Some(Screen::CancelAllConfirm) => {
            push_hint(&mut s, "y", "cancel all");
            push_hint(&mut s, "Esc", "abort");
            return s;
        }
        Some(Screen::RedeemConfirm) | Some(Screen::RedeemAllConfirm) => {
            push_hint(&mut s, "y", "confirm redeem");
            push_hint(&mut s, "Esc", "cancel");
            return s;
        }
        Some(Screen::QuitConfirm) => {
            push_hint(&mut s, "y", "quit");
            push_hint(&mut s, "Esc", "cancel");
            return s;
        }
        Some(Screen::Setup) => {
            push_hint(&mut s, "Enter", "next");
            push_hint(&mut s, "Esc", "cancel");
            return s;
        }
        Some(Screen::Help) => {
            push_hint(&mut s, "Esc", "close");
            return s;
        }
        _ => {}
    }

    match app.active_tab {
        Tab::Markets => match app.current_screen() {
            Some(Screen::MarketDetail) => {
                push_hint(&mut s, "←→", "outcome");
                push_hint(&mut s, "t", "interval");
                push_hint(&mut s, "b", "buy");
                push_hint(&mut s, "s", "sell");
                push_hint(&mut s, "c", "copy");
                push_hint(&mut s, "r", "refresh");
                push_hint(&mut s, "Esc", "back");
                push_hint(&mut s, "?", "help");
            }
            _ => {
                if app.search_mode {
                    push_hint(&mut s, "Esc", "cancel");
                    push_hint(&mut s, "Enter", "confirm");
                } else {
                    push_hint(&mut s, "/", "search");
                    push_hint(&mut s, "↑↓", "navigate");
                    push_hint(&mut s, "Enter", "open");
                    push_hint(&mut s, "s", "sort");
                    push_hint(&mut s, "d", "date");
                    push_hint(&mut s, "p", "prob");
                    push_hint(&mut s, "v", "vol");
                    let cat_label = match app.category_filter.as_deref() {
                        Some(cat) => format!("cat:{}", cat),
                        None => "cat".to_string(),
                    };
                    push_hint_dyn(&mut s, "c", cat_label);
                    push_hint(&mut s, "e", "export ★");
                    push_hint(&mut s, "r", "refresh");
                    push_hint(&mut s, "?", "help");
                    push_hint(&mut s, "q", "menu");
                }
            }
        },
        Tab::Positions => {
            push_hint(&mut s, "↑↓", "navigate");
            if app.positions_focus_orders {
                push_hint(&mut s, "c", "cancel");
                push_hint(&mut s, "C", "cancel all");
                push_hint(&mut s, "Tab", "positions");
            } else {
                push_hint(&mut s, "b", "buy");
                push_hint(&mut s, "s", "sell");
                push_hint(&mut s, "x", "close");
                push_hint(&mut s, "R", "redeem");
                push_hint(&mut s, "A", "redeem all");
                push_hint(&mut s, "Tab", "orders");
            }
            push_hint(&mut s, "r", "refresh");
            push_hint(&mut s, "q", "menu");
        }
        Tab::Balance => {
            push_hint(&mut s, "r", "refresh");
            push_hint(&mut s, "q", "menu");
            push_hint(&mut s, "?", "help");
        }
        Tab::Analytics => {
            push_hint(&mut s, "s", "collapse");
            push_hint(&mut s, "r", "recompute");
            push_hint(&mut s, "p", "snapshot");
            push_hint_dyn(&mut s, "t", format!("−{}h (C)", app.calibration_hours));
            push_hint_dyn(
                &mut s,
                "w",
                format!(
                    "fit:{}",
                    if app.regression_weighted {
                        "WLS"
                    } else {
                        "OLS"
                    }
                ),
            );
            push_hint(&mut s, "q", "menu");
        }
        Tab::Viewer => {
            if app.viewer_address_editing {
                push_hint(&mut s, "Enter", "submit");
                push_hint(&mut s, "Esc", "cancel");
            } else {
                push_hint(&mut s, "/", "address");
                if app.viewer_address.is_some() {
                    push_hint(&mut s, "↑↓", "navigate");
                    push_hint(&mut s, "Enter", "open");
                    push_hint(&mut s, "r", "refresh");
                }
                push_hint(&mut s, "q", "menu");
            }
        }
    }

    s
}
