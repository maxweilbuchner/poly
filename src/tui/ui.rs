use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

use super::{
    root_menu_items,
    screens::{analytics, balance, detail, markets, order, positions, setup},
    theme,
    widgets::{status_bar, tab_bar},
    App, Screen, Tab,
};

pub fn render(f: &mut Frame, app: &mut App) {
    // Fill the full terminal with the theme background so margins aren't black.
    f.render_widget(
        Block::default().style(Style::default().bg(theme::BG)),
        f.size(),
    );

    let area = f.size().inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    // Three-row layout: tab bar / content / status bar
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    tab_bar::render(f, chunks[0], app);
    status_bar::render(f, chunks[2], app);

    // Render content based on active tab.
    // MarketDetail can be pushed from any tab, so check the screen stack first.
    let content = chunks[1];
    match app.current_screen().cloned() {
        Some(Screen::MarketDetail) | Some(Screen::OrderEntry) => {
            detail::render(f, content, app);
        }
        _ => match &app.active_tab {
            Tab::Markets => render_markets_content(f, content, app),
            Tab::Positions => positions::render(f, content, app),
            Tab::Balance => balance::render(f, content, app),
            Tab::Analytics => analytics::render(f, content, app),
        },
    }

    // Render global modal overlays on top of everything.
    // OrderEntry and CloseConfirm are drawn here so they work from any tab.
    let full = f.size();
    match app.current_screen() {
        Some(Screen::Setup) => setup::render(f, full, &app.setup_form),
        Some(Screen::QuitConfirm) => render_root_menu(f, full, app),
        Some(Screen::Help) => render_help(f, full),
        Some(Screen::OrderEntry) => order::render(f, full, app),
        Some(Screen::CloseConfirm) => render_close_confirm(f, full, app),
        Some(Screen::CancelAllConfirm) => render_cancel_all_confirm(f, full, app),
        Some(Screen::RedeemConfirm) => render_redeem_confirm(f, full, app),
        Some(Screen::RedeemAllConfirm) => render_redeem_all_confirm(f, full, app),
        _ => {}
    }
}

fn render_markets_content(f: &mut Frame, area: Rect, app: &mut App) {
    match app.current_screen().cloned() {
        // Detail is always shown as the background; overlays are drawn globally above.
        Some(Screen::MarketDetail) | Some(Screen::OrderEntry) => {
            detail::render(f, area, app);
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

fn render_root_menu(f: &mut Frame, area: Rect, app: &App) {
    let modal = centered_rect(38, 14, area);
    f.render_widget(Clear, modal);

    let block = Block::bordered()
        .title(Span::styled(
            " Menu ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(modal);
    f.render_widget(block, modal);

    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let items = root_menu_items(app);
    let mut lines = vec![Line::from("")];
    for (i, (label, key_hint, color)) in items.iter().enumerate() {
        if i == app.menu_index {
            lines.push(Line::from(vec![
                Span::styled(
                    " ▸ ",
                    Style::default().fg(*color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:<18}", label),
                    Style::default().fg(*color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {}", key_hint), Style::default().fg(theme::HINT)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled(format!("{:<18}", label), Style::default().fg(theme::DIM)),
                Span::styled(
                    format!(" {}", key_hint),
                    Style::default().fg(theme::VERY_DIM),
                ),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), sections[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("↑↓", Style::default().fg(theme::CYAN)),
        Span::styled(" navigate   ", Style::default().fg(theme::HINT)),
        Span::styled("↵", Style::default().fg(theme::CYAN)),
        Span::styled(" select   ", Style::default().fg(theme::HINT)),
        Span::styled("Esc", Style::default().fg(theme::CYAN)),
        Span::styled(" cancel", Style::default().fg(theme::HINT)),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(footer, sections[2]);
}

fn render_close_confirm(f: &mut Frame, area: Rect, app: &App) {
    let modal = centered_rect(46, 40, area);
    f.render_widget(Clear, modal);

    let block = Block::bordered()
        .title(Span::styled(
            " Close Position ",
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Rgb(140, 60, 60)))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(modal);
    f.render_widget(block, modal);

    let sections = Layout::vertical([
        Constraint::Length(1), // top spacer
        Constraint::Min(1),    // body
        Constraint::Length(1), // footer
    ])
    .split(inner);

    let pos = app.close_confirm_pos_idx.and_then(|i| app.positions.get(i));
    let size: f64 = app.order_form.size_input.parse().unwrap_or(0.0);
    let outcome = &app.order_form.outcome_name;

    let price_str = match app.order_form.market_price {
        Some(p) => format!("{:.4}", p),
        None if app.order_form.market_price_failed => "fetch failed  [r retry]".to_string(),
        None => "loading…".to_string(),
    };

    let proceeds = app.order_form.market_price.map(|p| size * p);
    let avg_price = pos.map(|p| p.avg_price).unwrap_or(0.0);
    let cost_basis = avg_price * size;
    let pnl = proceeds.map(|pr| pr - cost_basis);

    let outcome_display: String = if outcome.chars().count() > 38 {
        outcome.chars().take(37).collect::<String>() + "…"
    } else {
        outcome.clone()
    };

    let mut lines = vec![Line::from("")];

    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            outcome_display,
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Sell      ", Style::default().fg(theme::VERY_DIM)),
        Span::styled(
            format!("{:.2} shares", size),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  At market  ", Style::default().fg(theme::VERY_DIM)),
        Span::styled(price_str, Style::default().fg(theme::TEXT)),
    ]));
    if let Some(avg) = pos.map(|p| p.avg_price) {
        lines.push(Line::from(vec![
            Span::styled("  Avg price  ", Style::default().fg(theme::VERY_DIM)),
            Span::styled(format!("{:.4}", avg), Style::default().fg(theme::DIM)),
        ]));
    }
    if let Some(pr) = proceeds {
        lines.push(Line::from(vec![
            Span::styled("  Proceeds   ", Style::default().fg(theme::VERY_DIM)),
            Span::styled(format!("${:.4}", pr), Style::default().fg(theme::TEXT)),
        ]));
    }
    if let Some(p) = pnl {
        let sign = if p >= 0.0 { "+" } else { "" };
        let color = if p >= 0.0 { theme::GREEN } else { theme::RED };
        lines.push(Line::from(vec![
            Span::styled("  P&L        ", Style::default().fg(theme::VERY_DIM)),
            Span::styled(
                format!("{}{:.4}", sign, p),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    f.render_widget(Paragraph::new(lines), sections[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "y/Enter",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" confirm   ", Style::default().fg(theme::HINT)),
        Span::styled("Esc/n", Style::default().fg(theme::DIM)),
        Span::styled(" cancel", Style::default().fg(theme::HINT)),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(footer, sections[2]);
}

fn render_help(f: &mut Frame, area: Rect) {
    let modal = centered_rect(70, 90, area);
    f.render_widget(Clear, modal);

    let block = Block::bordered()
        .title(Span::styled(
            " Key Bindings ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(modal);
    f.render_widget(block, modal);

    // Reserve the last row for the close hint.
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);

    // Two content columns separated by a 1-char vertical divider.
    let cols = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .split(layout[0]);

    // Vertical divider.
    let div_h = layout[0].height as usize;
    let div_lines: Vec<Line<'static>> = (0..div_h)
        .map(|_| Line::from(Span::styled("│", Style::default().fg(theme::BORDER))))
        .collect();
    f.render_widget(Paragraph::new(div_lines), cols[1]);

    let k = Style::default().fg(theme::TEXT);
    let d = Style::default().fg(theme::DIM);
    let h = Style::default()
        .fg(theme::CYAN)
        .add_modifier(Modifier::BOLD);

    // ── Left column: Navigation · Market Detail · Order Entry ─────────────

    let mut left: Vec<Line<'static>> = vec![Line::from("")];

    left.push(Line::from(Span::styled("  Navigation", h)));
    for (key, desc) in [
        ("1 / 2 / 3 / 4", "Switch tabs"),
        ("Tab", "Cycle tabs / switch panels"),
        ("↑ ↓  /  j k", "Navigate lists"),
        ("Esc", "Go back / close"),
        ("q", "Quit menu"),
        ("?", "Help screen"),
        ("Ctrl+C", "Force quit"),
    ] {
        left.push(Line::from(vec![
            Span::styled(format!("  {:>12}  ", key), k),
            Span::styled(desc, d),
        ]));
    }

    left.push(Line::from(""));
    left.push(Line::from(Span::styled("  Market Detail", h)));
    for (key, desc) in [
        ("← →  /  Tab", "Cycle outcomes"),
        ("t", "Sparkline: 1d ↔ 1w"),
        ("b / s", "Buy / sell"),
        ("c", "Copy condition ID"),
        ("r", "Refresh"),
    ] {
        left.push(Line::from(vec![
            Span::styled(format!("  {:>12}  ", key), k),
            Span::styled(desc, d),
        ]));
    }

    left.push(Line::from(""));
    left.push(Line::from(Span::styled("  Order Entry", h)));
    for (key, desc) in [
        ("Tab", "Next field"),
        ("Space", "Cycle order type"),
        ("d", "Toggle dry-run"),
        ("r", "Refresh market price"),
        ("Enter", "Submit"),
        ("Esc", "Cancel"),
    ] {
        left.push(Line::from(vec![
            Span::styled(format!("  {:>12}  ", key), k),
            Span::styled(desc, d),
        ]));
    }

    // ── Right column: Markets · Positions ─────────────────────────────────

    let mut right: Vec<Line<'static>> = vec![Line::from("")];

    right.push(Line::from(Span::styled("  Markets", h)));
    for (key, desc) in [
        ("/", "Search"),
        ("s", "Sort: vol → date → prob"),
        ("d", "Date: all → today → 7d → 30d"),
        ("p", "Prob: all ↔ 80–98%"),
        ("v", "Vol: all → >1K → >10K → >100K"),
        ("*", "Star / unstar market"),
        ("w", "Toggle watchlist-only"),
        ("e", "Export starred to JSON"),
        ("Enter", "Open market detail"),
        ("r", "Refresh"),
    ] {
        right.push(Line::from(vec![
            Span::styled(format!("  {:>12}  ", key), k),
            Span::styled(desc, d),
        ]));
    }

    right.push(Line::from(""));
    right.push(Line::from(Span::styled("  Positions", h)));
    for (key, desc) in [
        ("b / s", "Buy more / sell"),
        ("x", "Close at market price"),
        ("c", "Cancel highlighted order"),
        ("C", "Cancel all orders"),
        ("R", "Redeem won position on-chain"),
        ("A", "Redeem all redeemable"),
        ("r", "Refresh"),
    ] {
        right.push(Line::from(vec![
            Span::styled(format!("  {:>12}  ", key), k),
            Span::styled(desc, d),
        ]));
    }

    right.push(Line::from(""));
    right.push(Line::from(Span::styled("  Balance", h)));
    right.push(Line::from(vec![
        Span::styled(format!("  {:>12}  ", "r"), k),
        Span::styled("Refresh balance", d),
    ]));

    right.push(Line::from(""));
    right.push(Line::from(Span::styled("  Analytics", h)));
    for (key, desc) in [
        ("p", "Pull market snapshot"),
        ("r", "Recompute analytics"),
        ("s", "Collapse / expand panel"),
        ("t", "Calibration window (3–12h)"),
        ("w", "Regression: WLS ↔ OLS"),
        ("c", "Copy DB path"),
        ("o", "Open data folder"),
    ] {
        right.push(Line::from(vec![
            Span::styled(format!("  {:>12}  ", key), k),
            Span::styled(desc, d),
        ]));
    }

    f.render_widget(Paragraph::new(left), cols[0]);
    f.render_widget(Paragraph::new(right), cols[2]);

    // Footer close hint.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  Esc", Style::default().fg(theme::CYAN)),
            Span::styled("  close", Style::default().fg(theme::VERY_DIM)),
        ])),
        layout[1],
    );
}

fn render_cancel_all_confirm(f: &mut Frame, area: Rect, app: &App) {
    let modal = centered_rect(44, 30, area);
    f.render_widget(Clear, modal);

    let block = Block::bordered()
        .title(Span::styled(
            " Cancel All Orders ",
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Rgb(140, 60, 60)))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(modal);
    f.render_widget(block, modal);

    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let count = app.orders.len();
    let mut lines = vec![Line::from("")];
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!(
                "Cancel all {} open order{}?",
                count,
                if count == 1 { "" } else { "s" }
            ),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "This action cannot be undone.",
            Style::default().fg(theme::DIM),
        ),
    ]));
    f.render_widget(Paragraph::new(lines), sections[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "y/Enter",
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" confirm   ", Style::default().fg(theme::HINT)),
        Span::styled("Esc/n", Style::default().fg(theme::DIM)),
        Span::styled(" cancel", Style::default().fg(theme::HINT)),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(footer, sections[2]);
}

fn render_redeem_confirm(f: &mut Frame, area: Rect, app: &App) {
    let modal = centered_rect(46, 40, area);
    f.render_widget(Clear, modal);

    let block = Block::bordered()
        .title(Span::styled(
            " Redeem Position ",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Rgb(40, 120, 60)))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(modal);
    f.render_widget(block, modal);

    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let pos = app
        .redeem_confirm_pos_idx
        .and_then(|i| app.positions.get(i));

    let mut lines = vec![Line::from("")];
    if let Some(p) = pos {
        let outcome_display: String = if p.outcome.chars().count() > 38 {
            p.outcome.chars().take(37).collect::<String>() + "…"
        } else {
            p.outcome.clone()
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                outcome_display,
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Shares     ", Style::default().fg(theme::VERY_DIM)),
            Span::styled(
                format!("{:.2}", p.size),
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Proceeds   ", Style::default().fg(theme::VERY_DIM)),
            Span::styled(
                format!("≈ ${:.4} USDC", p.size),
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Sends an on-chain tx — requires RPC + private key.",
                Style::default().fg(theme::VERY_DIM),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "  No position selected.",
            Style::default().fg(theme::DIM),
        )));
    }
    f.render_widget(Paragraph::new(lines), sections[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "y/Enter",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" confirm   ", Style::default().fg(theme::HINT)),
        Span::styled("Esc/n", Style::default().fg(theme::DIM)),
        Span::styled(" cancel", Style::default().fg(theme::HINT)),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(footer, sections[2]);
}

fn render_redeem_all_confirm(f: &mut Frame, area: Rect, app: &App) {
    let modal = centered_rect(44, 32, area);
    f.render_widget(Clear, modal);

    let redeemable: Vec<&crate::types::Position> =
        app.positions.iter().filter(|p| p.redeemable).collect();
    let count = redeemable.len();
    let total_proceeds: f64 = redeemable.iter().map(|p| p.size).sum();

    let block = Block::bordered()
        .title(Span::styled(
            " Redeem All Positions ",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Rgb(40, 120, 60)))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(modal);
    f.render_widget(block, modal);

    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let mut lines = vec![Line::from("")];
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!(
                "Redeem {} redeemable position{}?",
                count,
                if count == 1 { "" } else { "s" }
            ),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Total proceeds  ", Style::default().fg(theme::VERY_DIM)),
        Span::styled(
            format!("≈ ${:.4} USDC", total_proceeds),
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "Sends one tx per position sequentially.",
            Style::default().fg(theme::VERY_DIM),
        ),
    ]));
    f.render_widget(Paragraph::new(lines), sections[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "y/Enter",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" confirm   ", Style::default().fg(theme::HINT)),
        Span::styled("Esc/n", Style::default().fg(theme::DIM)),
        Span::styled(" cancel", Style::default().fg(theme::HINT)),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(footer, sections[2]);
}
