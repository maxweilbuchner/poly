use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Sparkline},
    Frame,
};

use crate::tui::widgets::order_book;
use crate::tui::{theme, App};
use ratatui::style::Color;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    // Split into header + sparklines + order books
    let condition_id = app
        .selected_market
        .as_ref()
        .map(|m| m.condition_id.as_str())
        .unwrap_or("");
    let history_key = format!("{}:{}", condition_id, app.sparkline_interval);
    let has_history = app.price_history.contains_key(&history_key);

    // Compute header height: base rows + description lines (capped at 20%
    // of available height unless 'e' was pressed to expand).
    let desc_lines = description_line_count(app, area.width, area.height);
    let header_height = 7 + desc_lines as u16;

    let chunks = if has_history {
        Layout::vertical([
            Constraint::Length(header_height),
            Constraint::Length(4),
            Constraint::Min(0),
        ])
        .split(area)
    } else {
        // No sparkline row — use a zero-height slot to keep the same 3-chunk indexing
        Layout::vertical([
            Constraint::Length(header_height),
            Constraint::Length(0),
            Constraint::Min(0),
        ])
        .split(area)
    };

    render_header(f, chunks[0], app);
    if has_history {
        render_sparklines(f, chunks[1], app);
    }
    render_books(f, chunks[2], app);
}

/// Count how many lines the description will occupy in the header
/// (including the "..." indicator when truncated).
fn description_line_count(app: &App, area_width: u16, area_height: u16) -> usize {
    let desc = match &app.selected_market {
        Some(m) => m.description.as_deref().unwrap_or(""),
        None => "",
    };
    if desc.is_empty() {
        return 0;
    }
    let usable = (area_width as usize).saturating_sub(4).max(1);
    let total = wrap_text(desc, usable).len();
    if app.description_expanded {
        total
    } else {
        let fifth = (area_height as usize).saturating_sub(7) / 5;
        // The truncation indicator is now appended to the last visible line,
        // so it no longer needs its own row.
        total.min(fifth.max(2).saturating_sub(1))
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered()
        .title(Span::styled(
            " Market Detail ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    let lines = match &app.selected_market {
        None => vec![
            Line::from(""),
            Line::from(Span::styled("  Loading…", Style::default().fg(theme::DIM))),
        ],
        Some(m) => {
            let vol = format_volume(m.volume);
            let liq = format_volume(m.liquidity);
            let end = m.end_date.as_deref().unwrap_or("—");
            let cat = m.category.as_deref().unwrap_or("—");
            // Stats first
            let mut lines = vec![Line::from(vec![
                Span::styled("  Status: ", Style::default().fg(theme::DIM)),
                Span::styled(m.status.to_string(), Style::default().fg(theme::CYAN)),
                Span::styled("   Vol: ", Style::default().fg(theme::DIM)),
                Span::styled(vol, Style::default().fg(theme::YELLOW)),
                Span::styled("   Liq: ", Style::default().fg(theme::DIM)),
                Span::styled(liq, Style::default().fg(theme::YELLOW)),
            ])];
            {
                let mut ends_line = vec![
                    Span::styled("  Category: ", Style::default().fg(theme::DIM)),
                    Span::styled(cat, Style::default().fg(theme::TEXT)),
                    Span::styled("   Ends: ", Style::default().fg(theme::DIM)),
                    Span::styled(end, Style::default().fg(theme::TEXT)),
                ];
                if let Some((label, color)) = remaining_time(end) {
                    ends_line.push(Span::styled("  (", Style::default().fg(theme::DIM)));
                    ends_line.push(Span::styled(
                        label,
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ));
                    ends_line.push(Span::styled(")", Style::default().fg(theme::DIM)));
                }
                lines.push(Line::from(ends_line));
            }
            lines.push(Line::from(vec![
                Span::styled("  ID: ", Style::default().fg(theme::DIM)),
                Span::styled(&m.condition_id, Style::default().fg(theme::VERY_DIM)),
            ]));
            // Question
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    &m.question,
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            // Description, capped unless expanded
            let desc = m.description.as_deref().unwrap_or("");
            if !desc.is_empty() {
                let usable = (area.width as usize).saturating_sub(4).max(1);
                let wrapped = wrap_text(desc, usable);
                let total = wrapped.len();
                let max_desc = if app.description_expanded {
                    total
                } else {
                    let fifth = (area.height as usize).saturating_sub(7) / 5;
                    total.min(fifth.max(2).saturating_sub(1))
                };
                let truncated = !app.description_expanded && max_desc < total;
                let visible: Vec<String> = wrapped.into_iter().take(max_desc).collect();
                let last_idx = visible.len().saturating_sub(1);
                for (i, line) in visible.into_iter().enumerate() {
                    if truncated && i == last_idx {
                        // Append the truncation indicator to the last visible line so
                        // the reader sees a preview ending in "… [e expand]" rather
                        // than a bare "..." on its own line.
                        lines.push(Line::from(vec![
                            Span::styled("  ", Style::default()),
                            Span::styled(line, Style::default().fg(theme::DIM)),
                            Span::styled("…", Style::default().fg(theme::DIM)),
                            Span::styled("  [e expand]", Style::default().fg(theme::VERY_DIM)),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled("  ", Style::default()),
                            Span::styled(line, Style::default().fg(theme::DIM)),
                        ]));
                    }
                }
            }
            lines
        }
    };

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

fn render_sparklines(f: &mut Frame, area: Rect, app: &App) {
    let condition_id = match app.selected_market.as_ref() {
        Some(m) => m.condition_id.as_str(),
        None => return,
    };
    let history_key = format!("{}:{}", condition_id, app.sparkline_interval);
    let series = match app.price_history.get(&history_key) {
        Some(s) => s,
        None => return,
    };

    let n = series.len().clamp(1, 4) as u16;
    let constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();
    let cols = Layout::horizontal(constraints).split(area);

    let outcome_colors = [theme::CYAN, theme::GREEN, theme::YELLOW, theme::RED];

    for (i, (name, points)) in series.iter().enumerate() {
        if i >= cols.len() {
            break;
        }
        let color = outcome_colors[i % outcome_colors.len()];

        // Scale prices 0.0–1.0 → 0–100 as u64 for Sparkline
        let data: Vec<u64> = points
            .iter()
            .map(|&(_, p)| (p * 100.0).round() as u64)
            .collect();

        // Show only as many points as fit in the column width
        let width = cols[i].width.saturating_sub(2) as usize;
        let data_slice: &[u64] = if data.len() > width && width > 0 {
            &data[data.len() - width..]
        } else {
            &data
        };

        let label = if points.is_empty() {
            format!(" {} (no data)", name)
        } else {
            let last_price = points.last().map(|&(_, p)| p).unwrap_or(0.0);
            format!(
                " {} {:.0}%  {}",
                name,
                last_price * 100.0,
                app.sparkline_interval
            )
        };

        let block = Block::bordered()
            .title(Span::styled(label, Style::default().fg(color)))
            .border_style(Style::default().fg(theme::BORDER))
            .style(Style::default().bg(theme::PANEL_BG));

        let spark = Sparkline::default()
            .block(block)
            .data(data_slice)
            .max(100)
            .style(Style::default().fg(color));

        f.render_widget(spark, cols[i]);
    }
}

fn render_books(f: &mut Frame, area: Rect, app: &mut App) {
    // Check staleness: if the last update was more than 15s ago, show a warning.
    const STALE_SECS: u64 = 15;
    let stale_secs = app
        .order_book_updated_at
        .filter(|_| !app.order_books.is_empty())
        .map(|t| t.elapsed().as_secs())
        .filter(|&s| s >= STALE_SECS);

    // Reserve a warning row only when stale.
    let (warn_area, books_area) = if stale_secs.is_some() {
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
        (Some(rows[0]), rows[1])
    } else {
        (None, area)
    };

    if let (Some(warn), Some(secs)) = (warn_area, stale_secs) {
        let msg = format!(
            "  ⚠ order book data is {}s old — WS disconnected, polling every 10s  [r refresh]",
            secs
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                msg,
                Style::default().fg(Color::Rgb(200, 140, 40)),
            )),
            warn,
        );
    }

    if app.order_books.is_empty() {
        let block = Block::bordered()
            .title(Span::styled(
                " Order Books ",
                Style::default().fg(theme::CYAN),
            ))
            .border_style(Style::default().fg(theme::BORDER))
            .style(Style::default().bg(theme::PANEL_BG));
        let msg = if app.loading {
            "Loading…"
        } else {
            "No order book data."
        };
        let para = Paragraph::new(Span::styled(msg, Style::default().fg(theme::DIM))).block(block);
        f.render_widget(para, books_area);
        return;
    }

    // Split horizontally for each outcome
    let n = app.order_books.len().min(4) as u16;
    let constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();
    let cols = Layout::horizontal(constraints).split(books_area);

    for (i, (label, book)) in app.order_books.iter().enumerate() {
        if i >= cols.len() {
            break;
        }
        let selected = i == app.detail_outcome_index;
        order_book::render_with_selection(f, cols[i], Some(book), label, 10, selected);
    }
}

/// Returns `(label, color)` for the time remaining until `end_date`.
/// Returns `None` if the date cannot be parsed.
fn remaining_time(end_date: &str) -> Option<(String, Color)> {
    use chrono::{DateTime, NaiveDate, Utc};

    let end: DateTime<Utc> = DateTime::parse_from_rfc3339(end_date)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            NaiveDate::parse_from_str(&end_date[..end_date.len().min(10)], "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|ndt| ndt.and_utc())
        })?;

    let secs = end.signed_duration_since(Utc::now()).num_seconds();

    if secs <= 0 {
        return Some(("ended".to_string(), theme::DIM));
    }

    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;

    let label = if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    };

    let color = if secs < 3_600 {
        theme::RED
    } else if secs < 86_400 {
        theme::YELLOW
    } else {
        theme::DIM
    };

    Some((label, color))
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

/// Word-wrap `text` into lines of at most `max_width` characters.
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            if current.is_empty() {
                current = word.to_string();
            } else if current.len() + 1 + word.len() <= max_width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}
