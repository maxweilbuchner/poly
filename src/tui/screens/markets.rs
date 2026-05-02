use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

use super::util::{pad_left, pad_right, truncate};
use crate::tui::{market_category, theme, App};
use crate::types::Market;
use crate::weather::weather_location;

pub fn render(f: &mut Frame, area: Rect, app: &mut App) {
    let (list_area, search_area) = if app.search_mode || !app.search_query.is_empty() {
        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);
        (chunks[1], Some(chunks[0]))
    } else {
        (area, None)
    };

    if let Some(sa) = search_area {
        render_search_bar(f, sa, app);
    }

    render_market_list(f, list_area, app);
}

fn render_search_bar(f: &mut Frame, area: Rect, app: &App) {
    let cursor = if app.search_mode { "▏" } else { "" };
    let block = Block::bordered()
        .title(Span::styled(" Search ", Style::default().fg(theme::CYAN)))
        .border_style(Style::default().fg(if app.search_mode {
            theme::BORDER_ACTIVE
        } else {
            theme::BORDER
        }))
        .style(Style::default().bg(theme::PANEL_BG));

    let para = Paragraph::new(format!("{}{}", app.search_query, cursor))
        .style(Style::default().fg(theme::TEXT).bg(theme::PANEL_BG))
        .block(block);
    f.render_widget(para, area);
}

fn render_market_list(f: &mut Frame, area: Rect, app: &mut App) {
    // Build position badge lookup before the markets borrow.
    // Keyed by condition_id; value is a list of (badge_text, color) per outcome held.
    let pos_badges: std::collections::HashMap<String, Vec<(String, ratatui::style::Color)>> = {
        let mut map: std::collections::HashMap<String, Vec<(String, ratatui::style::Color)>> =
            std::collections::HashMap::new();
        for p in &app.positions {
            if p.size <= 0.0 {
                continue;
            }
            let cents = (p.avg_price * 100.0).round() as u64;
            let label = format!("↑ {} @{}¢", truncate(&p.outcome, 6), cents);
            let color = if p.current_price > p.avg_price + 0.005 {
                theme::GREEN
            } else if p.current_price < p.avg_price - 0.005 {
                theme::RED
            } else {
                theme::CYAN
            };
            map.entry(p.market_id.clone())
                .or_default()
                .push((label, color));
        }
        map
    };

    let filtered = app.filtered_markets();
    let count = filtered.len();

    let title = if app.watchlist_only {
        format!(" Markets — ★ watchlist ({}) ", count)
    } else if app.search_query.is_empty() {
        format!(" Markets ({}) ", count)
    } else {
        format!(" Markets — {} results ", count)
    };

    let cat_label = app.category_filter.as_deref().unwrap_or("all");
    let cat_active = app.category_filter.is_some();
    let prob_active = app.prob_filter != crate::tui::ProbFilter::All;
    let vol_active = app.volume_filter != crate::tui::VolumeFilter::All;
    let filters = Line::from(vec![
        Span::styled("sort:", Style::default().fg(theme::VERY_DIM)),
        Span::styled(
            format!("{}  ", app.sort_mode.label()),
            Style::default().fg(theme::DIM),
        ),
        Span::styled("date:", Style::default().fg(theme::VERY_DIM)),
        Span::styled(
            format!("{}  ", app.date_filter.label()),
            Style::default().fg(theme::DIM),
        ),
        Span::styled("prob:", Style::default().fg(theme::VERY_DIM)),
        Span::styled(
            format!("{}  ", app.prob_filter.label()),
            Style::default().fg(if prob_active { theme::CYAN } else { theme::DIM }),
        ),
        Span::styled("vol:", Style::default().fg(theme::VERY_DIM)),
        Span::styled(
            format!("{}  ", app.volume_filter.label()),
            Style::default().fg(if vol_active { theme::CYAN } else { theme::DIM }),
        ),
        Span::styled("cat:", Style::default().fg(theme::VERY_DIM)),
        Span::styled(
            format!("{}  ", cat_label),
            Style::default().fg(if cat_active { theme::CYAN } else { theme::DIM }),
        ),
        Span::styled("watch:", Style::default().fg(theme::VERY_DIM)),
        Span::styled(
            format!("{} ", if app.watchlist_only { "★" } else { "all" }),
            Style::default().fg(if app.watchlist_only {
                theme::YELLOW
            } else {
                theme::DIM
            }),
        ),
    ]);
    // .alignment(ratatui::layout::Alignment::Right);

    let block = Block::bordered()
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let content_chunks = if filtered.is_empty() {
        ratatui::layout::Layout::vertical([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Min(0),
        ])
        .split(inner_area)
    } else {
        ratatui::layout::Layout::vertical([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Min(0),
        ])
        .split(inner_area)
    };

    f.render_widget(
        ratatui::widgets::Paragraph::new(filters).alignment(ratatui::layout::Alignment::Right),
        content_chunks[0],
    );

    if filtered.is_empty() {
        let msg = if app.loading {
            "Loading markets…"
        } else if app.watchlist_only {
            "No starred markets. Press * to star a market, w to exit watchlist mode."
        } else if !app.search_query.is_empty() {
            "No markets match your search."
        } else if app.category_filter.is_some() || app.prob_filter != crate::tui::ProbFilter::All {
            "No markets match the active filters."
        } else {
            "No markets found. Press r to refresh."
        };
        let para =
            ratatui::widgets::Paragraph::new(Span::styled(msg, Style::default().fg(theme::DIM)))
                .style(Style::default().bg(theme::PANEL_BG));
        f.render_widget(para, content_chunks[1]);
        return;
    }

    // ── Pre-pass: format every row's columns and measure widths ──────────────
    let mrows: Vec<MRow> = filtered
        .iter()
        .map(|m| {
            let weather_suffix: Option<String> = market_category(m)
                .filter(|c| *c == "Weather")
                .and_then(|_| weather_location(m))
                .filter(|loc| !loc.icao.is_empty())
                .map(|loc| {
                    let local = crate::weather::lookup_airport(&loc.icao)
                        .and_then(crate::weather::local_time_now)
                        .map(|t| format!(" · {}", t))
                        .unwrap_or_default();
                    format!("  {}·{}{}", loc.icao, loc.country, local)
                });
            let (prices_str, prices_color) = format_prices(m);
            let vol_str = format_volume(m.volume);
            let (end_str, end_color) = format_end(m.end_date.as_deref())
                .unwrap_or_else(|| (String::new(), theme::VERY_DIM));
            let cat = market_category(m).unwrap_or("");
            MRow {
                question: m.question.clone(),
                weather_suffix,
                starred: app.watchlist.contains(&m.condition_id),
                prices_str,
                prices_color,
                vol_str,
                end_str,
                end_color,
                cat_str: cat.to_string(),
                cat_color: category_color(cat),
                badges: pos_badges.get(&m.condition_id).cloned().unwrap_or_default(),
            }
        })
        .collect();

    let max_prices = mrows
        .iter()
        .map(|r| r.prices_str.chars().count())
        .max()
        .unwrap_or(0)
        .max("Prices".len());
    let max_vol = mrows
        .iter()
        .map(|r| r.vol_str.chars().count())
        .max()
        .unwrap_or(0)
        .max("Vol".len());
    let max_ends = mrows
        .iter()
        .map(|r| r.end_str.chars().count())
        .max()
        .unwrap_or(0)
        .max("Ends".len());
    let max_cat = mrows
        .iter()
        .map(|r| r.cat_str.chars().count())
        .max()
        .unwrap_or(0)
        .max("Cat".len());

    // Row layout (rendered inside the bordered list):
    //   highlight "▸ "(2) + indent "  "(2) + q_width + 4× sep "  · "(4 each)
    //   + max_prices + max_vol + max_ends + max_cat
    // Subtract borders(2) from area.width to get the inner width.
    let fixed = 2 /* highlight */ + 2 /* indent */ + 4 * 4 /* separators */
        + max_prices + max_vol + max_ends + max_cat;
    let q_width = (area.width as usize)
        .saturating_sub(2) // borders
        .saturating_sub(fixed)
        .max(20);

    // ── Header row ────────────────────────────────────────────────────────────
    let header = Line::from(vec![
        Span::raw("    "),
        Span::styled(
            pad_right("Market".to_string(), q_width),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_right("Prices".to_string(), max_prices),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_left("Vol".to_string(), max_vol),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_left("Ends".to_string(), max_ends),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_right("Cat".to_string(), max_cat),
            Style::default().fg(theme::DIM),
        ),
    ]);

    f.render_widget(ratatui::widgets::Paragraph::new(header), content_chunks[1]);

    let items: Vec<ListItem> = mrows
        .into_iter()
        .map(|r| build_row(r, q_width, max_prices, max_vol, max_ends, max_cat))
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(ratatui::style::Color::Rgb(32, 38, 72))
                .fg(ratatui::style::Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, content_chunks[2], &mut app.market_list_state);
}

struct MRow {
    question: String,
    weather_suffix: Option<String>,
    starred: bool,
    prices_str: String,
    prices_color: Color,
    vol_str: String,
    end_str: String,
    end_color: Color,
    cat_str: String,
    cat_color: Color,
    badges: Vec<(String, Color)>,
}

fn build_row(
    r: MRow,
    q_width: usize,
    max_prices: usize,
    max_vol: usize,
    max_ends: usize,
    max_cat: usize,
) -> ListItem<'static> {
    let MRow {
        question,
        weather_suffix,
        starred,
        prices_str,
        prices_color,
        vol_str,
        end_str,
        end_color,
        cat_str,
        cat_color,
        badges,
    } = r;

    // Reserve space for star prefix and weather suffix inside the question column.
    let star_w = if starred { 2 } else { 0 };
    let suffix_w = weather_suffix
        .as_deref()
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let q_avail = q_width.saturating_sub(star_w).saturating_sub(suffix_w);
    let q_truncated = truncate(&question, q_avail);
    let q_used = star_w + q_truncated.chars().count() + suffix_w;
    let q_pad = q_width.saturating_sub(q_used);

    let mut spans: Vec<Span> = vec![Span::raw("  ")];
    if starred {
        spans.push(Span::styled("★ ", Style::default().fg(theme::YELLOW)));
    }
    spans.push(Span::styled(
        q_truncated,
        Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
    ));
    if let Some(s) = weather_suffix {
        spans.push(Span::styled(s, Style::default().fg(theme::VERY_DIM)));
    }
    if q_pad > 0 {
        spans.push(Span::raw(" ".repeat(q_pad)));
    }

    spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
    spans.push(Span::styled(
        pad_right(prices_str, max_prices),
        Style::default().fg(prices_color),
    ));

    spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
    spans.push(Span::styled(
        pad_left(vol_str, max_vol),
        Style::default().fg(theme::DIM),
    ));

    spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
    spans.push(Span::styled(
        pad_left(end_str, max_ends),
        Style::default().fg(end_color),
    ));

    spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
    spans.push(Span::styled(
        pad_right(cat_str, max_cat),
        Style::default().fg(cat_color),
    ));

    if !badges.is_empty() {
        spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
        for (i, (badge, badge_color)) in badges.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ", Style::default().fg(theme::VERY_DIM)));
            }
            spans.push(Span::styled(badge, Style::default().fg(badge_color)));
        }
    }

    ListItem::new(Line::from(spans))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn format_volume(v: f64) -> String {
    if v >= 1_000_000.0 {
        format!("${:.1}M", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("${:.1}K", v / 1_000.0)
    } else {
        format!("${:.2}", v)
    }
}

fn format_prices(m: &Market) -> (String, Color) {
    match m.outcomes.len() {
        0 => (String::new(), theme::CYAN),
        1 => {
            let p = m.outcomes[0].price;
            (format!("{:.0}%", p * 100.0), prob_color(p))
        }
        _ => {
            let primary_price = m.outcomes[0].price;
            let pairs: Vec<String> = m
                .outcomes
                .iter()
                .take(4)
                .map(|o| format!("{}:{:.0}%", truncate(&o.name, 4), o.price * 100.0))
                .collect();
            (pairs.join("  "), prob_color(primary_price))
        }
    }
}

/// Color based on probability of the primary outcome.
fn prob_color(p: f64) -> Color {
    if p >= 0.70 {
        theme::GREEN
    } else if p <= 0.30 {
        theme::RED
    } else {
        theme::CYAN
    }
}

/// Color for a category label.
fn category_color(cat: &str) -> Color {
    match cat {
        "Sports" => theme::GREEN,
        "Crypto" => theme::YELLOW,
        "Politics" => theme::PURPLE,
        "Finance" => theme::BLUE,
        "Weather" => theme::CYAN,
        _ => theme::HINT,
    }
}

fn format_end(end_date: Option<&str>) -> Option<(String, Color)> {
    use chrono::{DateTime, NaiveDate, Utc};

    let s = end_date?;
    let end: DateTime<Utc> = DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            NaiveDate::parse_from_str(&s[..s.len().min(10)], "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|ndt| ndt.and_utc())
        })?;

    let secs = end.signed_duration_since(Utc::now()).num_seconds();

    if secs <= 0 {
        return Some(("ended".to_string(), theme::VERY_DIM));
    }

    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;

    let label = if days >= 30 {
        format!("{}mo", days / 30)
    } else if days > 0 {
        format!("{}d", days)
    } else if hours > 0 {
        format!("{}h", hours)
    } else {
        "< 1h".to_string()
    };

    let color = if secs < 86_400 {
        theme::RED // < 1 day — urgent
    } else if days < 7 {
        theme::YELLOW // < 1 week — soon
    } else {
        theme::HINT // distant
    };

    Some((label, color))
}
