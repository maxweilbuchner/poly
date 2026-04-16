use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

use crate::tui::widgets::order_book;
use crate::tui::{theme, App};
use crate::types::{OrderType, Side};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Clear, area);

    let halves =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    render_book(f, halves[0], app);
    render_form(f, halves[1], app);
}

fn render_book(f: &mut Frame, area: Rect, app: &App) {
    let book_entry = app
        .order_books
        .iter()
        .find(|(_, b)| b.token_id == app.order_form.token_id);

    let (label, book_ref) = match book_entry {
        Some((l, b)) => (l.as_str(), Some(b as &crate::types::OrderBook)),
        None => (app.order_form.outcome_name.as_str(), None),
    };

    // Highlight the side the user is trading: BUY hits the ask, SELL hits the bid.
    let side_note = match app.order_form.side {
        Some(Side::Buy) => " [ask]",
        Some(Side::Sell) => " [bid]",
        None => "",
    };
    let full_label = format!("{}{}", label, side_note);

    let levels = area.height.saturating_sub(4) as usize;
    order_book::render_with_selection(f, area, book_ref, &full_label, levels, false);
}

fn render_form(f: &mut Frame, area: Rect, app: &App) {
    let side_label = app
        .order_form
        .side
        .as_ref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "—".to_string());

    let title = if app.order_form.close_position {
        format!(" CLOSE POSITION — {} ", app.order_form.outcome_name)
    } else {
        format!(" {} Order — {} ", side_label, app.order_form.outcome_name)
    };
    let block = Block::bordered()
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // spacer
        Constraint::Length(1), // side (read-only)
        Constraint::Length(1), // spacer
        Constraint::Length(1), // size field
        Constraint::Length(1), // price field
        Constraint::Length(1), // order type
        Constraint::Length(1), // spacer
        Constraint::Length(1), // dry run
        Constraint::Length(1), // spacer
        Constraint::Length(1), // cost display
        Constraint::Length(1), // fee display
        Constraint::Length(1), // spacer
        Constraint::Min(0),    // footer
    ])
    .split(inner);

    // Side (read-only)
    let side_color = app
        .order_form
        .side
        .as_ref()
        .map(|s| match s {
            Side::Buy => theme::GREEN,
            Side::Sell => theme::RED,
        })
        .unwrap_or(theme::DIM);
    render_field(f, rows[1], "Side", &side_label, false, side_color);

    // Inline validation — only flag parseable-but-invalid values, not mid-input (trailing dot).
    let size_parsed: Option<f64> = app.order_form.size_input.parse().ok();
    let size_below_min = !app.order_form.size_input.is_empty()
        && !app.order_form.size_input.ends_with('.')
        && size_parsed.is_some_and(|v| v > 0.0 && v < 5.0);
    let size_exceeds_held = !app.order_form.size_input.is_empty()
        && !app.order_form.size_input.ends_with('.')
        && app.order_form.max_size.is_some()
        && size_parsed.is_some_and(|v| v > app.order_form.max_size.unwrap_or(f64::MAX));
    let size_error = size_below_min || size_exceeds_held;

    let price_parsed: Option<f64> = app.order_form.price_input.parse().ok();
    let price_error = !app.order_form.market_order
        && !app.order_form.price_input.is_empty()
        && !app.order_form.price_input.ends_with('.')
        && price_parsed.is_some_and(|v| v > 0.0 && !(0.01..=0.99).contains(&v));

    // Size
    let size_focused = app.order_form.focused_field == 0;
    let size_label = if size_exceeds_held {
        format!("Size (max {:.2})", app.order_form.max_size.unwrap_or(0.0))
    } else if size_below_min {
        "Size (min 5 shares)".to_string()
    } else {
        "Size (shares)".to_string()
    };
    render_text_field(
        f,
        rows[3],
        &size_label,
        &app.order_form.size_input,
        size_focused,
        size_error,
    );
    if size_focused {
        let cx = rows[3].x + 18 + app.order_form.size_input.len() as u16;
        let cy = rows[3].y;
        if cx < rows[3].x + rows[3].width {
            f.set_cursor(cx, cy);
        }
    }

    // Price
    let price_focused = app.order_form.focused_field == 1;
    if app.order_form.market_order {
        let (price_display, price_color) = match app.order_form.market_price {
            Some(p) => (format!("{:.4}  [r refresh]", p), theme::YELLOW),
            None if app.order_form.market_price_failed => {
                ("fetch failed  [r retry]".to_string(), theme::RED)
            }
            None => ("fetching…".to_string(), theme::DIM),
        };
        render_field(
            f,
            rows[4],
            "Price (market)",
            &price_display,
            false,
            price_color,
        );
    } else {
        render_text_field(
            f,
            rows[4],
            "Price (0.01–0.99)",
            &app.order_form.price_input,
            price_focused,
            price_error,
        );
        if price_focused {
            let cx = rows[4].x + 18 + app.order_form.price_input.len() as u16;
            let cy = rows[4].y;
            if cx < rows[4].x + rows[4].width {
                f.set_cursor(cx, cy);
            }
        }
    }

    // Order type
    let ot_focused = app.order_form.focused_field == 2;
    let ot_label = if app.order_form.market_order {
        let dir = if matches!(app.order_form.side, Some(Side::Buy)) {
            "ask"
        } else {
            "bid"
        };
        format!("Market (FOK @ best {})", dir)
    } else {
        match app.order_form.order_type {
            OrderType::Gtc => "GTC (Good-til-Cancelled)".to_string(),
            OrderType::Fok => "FOK (Fill-or-Kill)".to_string(),
            OrderType::Ioc => "IOC (Immediate-or-Cancel)".to_string(),
        }
    };
    let ot_hint = if ot_focused { " [Space to cycle]" } else { "" };
    let ot_color = if app.order_form.market_order {
        theme::YELLOW
    } else if ot_focused {
        theme::CYAN
    } else {
        theme::TEXT
    };
    render_field(
        f,
        rows[5],
        "Order Type",
        &format!("{}{}", ot_label, ot_hint),
        ot_focused,
        ot_color,
    );

    // Dry run toggle
    let dr_label = if app.order_form.dry_run {
        "ON  [d to toggle]"
    } else {
        "off [d to toggle]"
    };
    let dr_color = if app.order_form.dry_run {
        theme::YELLOW
    } else {
        theme::DIM
    };
    render_field(f, rows[7], "Dry Run", dr_label, false, dr_color);

    // Cost preview
    if let Some(cost) = app.order_form.cost() {
        let (cost_str, cost_color) = if cost < 1.0 {
            (format!("${:.4}  (min $1.00)", cost), theme::RED)
        } else {
            (format!("${:.4}", cost), theme::BLUE)
        };
        render_field(f, rows[9], "Est. Cost", &cost_str, false, cost_color);
    }

    // Fee display
    let (fee_label, fee_color) = match app.order_form.fee_rate_bps {
        None => ("fetching…".to_string(), theme::DIM),
        Some(0) => ("0 bps  (no fee)".to_string(), theme::DIM),
        Some(bps) => {
            let pct = bps as f64 / 100.0;
            let price: Option<f64> = if app.order_form.market_order {
                app.order_form.market_price
            } else {
                app.order_form.price_input.parse().ok()
            };
            let size: Option<f64> = app.order_form.size_input.parse().ok();
            match (size, price) {
                (Some(s), Some(p)) if p > 0.0 && p < 1.0 => {
                    let fee = crate::client::PolyClient::calculate_fee(s, p, bps);
                    (
                        format!("{} bps  ({:.2}%)  ≈ ${:.5}", bps, pct, fee),
                        theme::YELLOW,
                    )
                }
                _ => (format!("{} bps  ({:.2}%)", bps, pct), theme::YELLOW),
            }
        }
    };
    render_field(f, rows[10], "Fee", &fee_label, false, fee_color);

    // Footer hint
    let footer = Paragraph::new(Span::styled(
        "  Tab/Shift+Tab move fields   Enter submit   Esc cancel",
        Style::default().fg(theme::VERY_DIM),
    ));
    f.render_widget(footer, rows[12]);
}

fn render_text_field(
    f: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    is_error: bool,
) {
    let border_color = if focused {
        theme::BORDER_ACTIVE
    } else {
        theme::BORDER
    };
    let value_color = if is_error {
        theme::RED
    } else if focused {
        theme::TEXT
    } else {
        theme::DIM
    };

    let line = Line::from(vec![
        Span::styled(
            format!("  {:>16}: ", label),
            Style::default().fg(theme::DIM),
        ),
        Span::styled(
            value.to_string(),
            Style::default().fg(value_color).add_modifier(if focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ),
        if focused {
            Span::styled("▏", Style::default().fg(border_color))
        } else {
            Span::raw("")
        },
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_field(
    f: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    color: ratatui::style::Color,
) {
    let line = Line::from(vec![
        Span::styled(
            format!("  {:>16}: ", label),
            Style::default().fg(theme::DIM),
        ),
        Span::styled(
            value.to_string(),
            Style::default().fg(color).add_modifier(if focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
