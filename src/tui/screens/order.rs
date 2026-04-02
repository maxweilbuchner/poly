use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

use crate::tui::{theme, ui::centered_rect, App};
use crate::types::OrderType;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let modal = centered_rect(55, 65, area);
    f.render_widget(Clear, modal);

    let side_label = app
        .order_form
        .side
        .as_ref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "—".to_string());

    let title = format!(" {} Order — {} ", side_label, app.order_form.outcome_name);
    let block = Block::bordered()
        .title(Span::styled(title, Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD)))
        .border_style(Style::default().fg(theme::CYAN))
        .style(Style::default().bg(theme::PANEL_BG));

    // Inner layout: rows
    let inner = block.inner(modal);
    f.render_widget(block, modal);

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
        Constraint::Length(1), // spacer
        Constraint::Min(0),    // footer
    ])
    .split(inner);

    // Side (read-only)
    let side_color = app.order_form.side.as_ref().map(|s| match s {
        crate::types::Side::Buy => theme::GREEN,
        crate::types::Side::Sell => theme::RED,
    }).unwrap_or(theme::DIM);
    render_field(f, rows[1], "Side", &side_label, false, side_color);

    // Size
    let size_focused = app.order_form.focused_field == 0;
    render_text_field(f, rows[3], "Size (shares)", &app.order_form.size_input, size_focused);
    if size_focused {
        let cx = rows[3].x + 18 + app.order_form.size_input.len() as u16;
        let cy = rows[3].y;
        if cx < rows[3].x + rows[3].width {
            f.set_cursor(cx, cy);
        }
    }

    // Price
    let price_focused = app.order_form.focused_field == 1;
    render_text_field(f, rows[4], "Price (0.01-0.99)", &app.order_form.price_input, price_focused);
    if price_focused {
        let cx = rows[4].x + 18 + app.order_form.price_input.len() as u16;
        let cy = rows[4].y;
        if cx < rows[4].x + rows[4].width {
            f.set_cursor(cx, cy);
        }
    }

    // Order type
    let ot_focused = app.order_form.focused_field == 2;
    let ot_label = match app.order_form.order_type {
        OrderType::Gtc => "GTC (Good-til-Cancelled)",
        OrderType::Fok => "FOK (Fill-or-Kill)",
        OrderType::Ioc => "IOC (Immediate-or-Cancel)",
    };
    let ot_hint = if ot_focused { " [Space to cycle]" } else { "" };
    render_field(
        f,
        rows[5],
        "Order Type",
        &format!("{}{}", ot_label, ot_hint),
        ot_focused,
        if ot_focused { theme::CYAN } else { theme::TEXT },
    );

    // Dry run toggle
    let dr_label = if app.order_form.dry_run { "ON  [d to toggle]" } else { "off [d to toggle]" };
    let dr_color = if app.order_form.dry_run { theme::YELLOW } else { theme::DIM };
    render_field(f, rows[7], "Dry Run", dr_label, false, dr_color);

    // Cost preview
    if let Some(cost) = app.order_form.cost() {
        render_field(
            f,
            rows[9],
            "Est. Cost",
            &format!("${:.4}", cost),
            false,
            theme::BLUE,
        );
    }

    // Footer hint
    let footer = Paragraph::new(Span::styled(
        "  Tab/Shift+Tab move fields   Enter submit   Esc cancel",
        Style::default().fg(theme::VERY_DIM),
    ));
    f.render_widget(footer, rows[11]);
}

fn render_text_field(f: &mut Frame, area: Rect, label: &str, value: &str, focused: bool) {
    let border_color = if focused { theme::BORDER_ACTIVE } else { theme::BORDER };
    let value_color = if focused { theme::TEXT } else { theme::DIM };

    let line = Line::from(vec![
        Span::styled(
            format!("  {:>16}: ", label),
            Style::default().fg(theme::DIM),
        ),
        Span::styled(
            format!("{}", value),
            Style::default().fg(value_color).add_modifier(if focused { Modifier::BOLD } else { Modifier::empty() }),
        ),
        if focused {
            Span::styled("▏", Style::default().fg(border_color))
        } else {
            Span::raw("")
        },
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_field(f: &mut Frame, area: Rect, label: &str, value: &str, focused: bool, color: ratatui::style::Color) {
    let line = Line::from(vec![
        Span::styled(
            format!("  {:>16}: ", label),
            Style::default().fg(theme::DIM),
        ),
        Span::styled(
            value,
            Style::default().fg(color).add_modifier(if focused { Modifier::BOLD } else { Modifier::empty() }),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
