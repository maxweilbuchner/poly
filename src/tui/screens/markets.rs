use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

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

    let content_chunks = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Min(0),
    ])
    .split(inner_area);

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

    // borders(2) + highlight_symbol "▸ "(2) + content indent "  "(2)
    let q_width = area.width.saturating_sub(6) as usize;

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|m| {
            let badges = pos_badges.get(&m.condition_id).cloned().unwrap_or_default();
            build_item(m, q_width, app.watchlist.contains(&m.condition_id), badges)
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(ratatui::style::Color::Rgb(32, 38, 72))
                .fg(ratatui::style::Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, content_chunks[1], &mut app.market_list_state);
}

fn build_item(
    m: &Market,
    q_width: usize,
    starred: bool,
    positions: Vec<(String, ratatui::style::Color)>,
) -> ListItem<'static> {
    let q_width = if starred {
        q_width.saturating_sub(2)
    } else {
        q_width
    };

    // For weather markets, surface the resolution station next to the question
    // so the user can see *where* the market resolves at a glance (the question
    // names the city in plain English, but the ICAO code is the canonical
    // station identifier and disambiguates e.g. multiple "Springfield"s).
    let weather_suffix: Option<String> = market_category(m)
        .filter(|c| *c == "Weather")
        .and_then(|_| weather_location(m))
        .filter(|loc| !loc.icao.is_empty())
        .map(|loc| format!("  {}·{}", loc.icao, loc.country));

    let suffix_w = weather_suffix
        .as_deref()
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let question = truncate(&m.question, q_width.saturating_sub(suffix_w));

    // Line 1: question (with optional star prefix)
    let mut line1_spans = vec![Span::raw("  ")];
    if starred {
        line1_spans.push(Span::styled("★ ", Style::default().fg(theme::YELLOW)));
    }
    line1_spans.push(Span::styled(
        question,
        Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
    ));
    if let Some(s) = weather_suffix {
        line1_spans.push(Span::styled(s, Style::default().fg(theme::VERY_DIM)));
    }
    let line1 = Line::from(line1_spans);

    // Line 2: metadata
    let vol = format_volume(m.volume);
    let (prices_str, prices_color) = format_prices(m);
    let end_info = format_end(m.end_date.as_deref());
    let cat = market_category(m);

    let mut spans: Vec<Span> = vec![
        Span::raw("  "),
        Span::styled(vol, Style::default().fg(theme::DIM)),
    ];
    if !prices_str.is_empty() {
        spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
        spans.push(Span::styled(prices_str, Style::default().fg(prices_color)));
    }
    if let Some((end_str, end_color)) = end_info {
        spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
        spans.push(Span::styled(end_str, Style::default().fg(end_color)));
    }
    if let Some(cat_str) = cat {
        spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
        spans.push(Span::styled(
            cat_str,
            Style::default().fg(category_color(cat_str)),
        ));
    }
    if !positions.is_empty() {
        spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
        for (i, (badge, badge_color)) in positions.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  ", Style::default().fg(theme::VERY_DIM)));
            }
            spans.push(Span::styled(badge, Style::default().fg(badge_color)));
        }
    }

    let line2 = Line::from(spans);

    ListItem::new(vec![line1, line2])
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}
