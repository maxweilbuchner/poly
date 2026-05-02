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
    // Build held-position lookup: condition_id → color (worst direction wins so a
    // losing leg of a multi-outcome hold isn't masked by a winning one).
    let held_color: std::collections::HashMap<String, Color> = {
        let mut map: std::collections::HashMap<String, Color> = std::collections::HashMap::new();
        for p in &app.positions {
            if p.size <= 0.0 {
                continue;
            }
            let color = if p.current_price > p.avg_price + 0.005 {
                theme::GREEN
            } else if p.current_price < p.avg_price - 0.005 {
                theme::RED
            } else {
                theme::CYAN
            };
            map.entry(p.market_id.clone())
                .and_modify(|c| {
                    // Prefer red > cyan > green so a losing leg surfaces.
                    if *c != theme::RED && (color == theme::RED || *c == theme::GREEN) {
                        *c = color;
                    }
                })
                .or_insert(color);
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
    // Skip markets whose close time has already passed — they're awaiting
    // resolution and don't belong in a "live markets" list.
    let mrows: Vec<MRow> = filtered
        .iter()
        .filter_map(|m| {
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
            let lead = lead_cell(m);
            let vol_str = format_volume(m.volume);
            let vol_color = volume_color(m.volume);
            let end = format_end(m.end_date.as_deref());
            if matches!(end.as_ref(), Some((s, _)) if s == "ended") {
                return None;
            }
            let (end_str, end_color) = end.unwrap_or_else(|| ("—".to_string(), theme::VERY_DIM));
            let cat_raw = market_category(m).unwrap_or("");
            let (cat_str, cat_color) = if cat_raw.is_empty() {
                ("—".to_string(), theme::VERY_DIM)
            } else {
                (cat_raw.to_string(), category_color(cat_raw))
            };
            Some(MRow {
                question: m.question.clone(),
                weather_suffix,
                starred: app.watchlist.contains(&m.condition_id),
                lead,
                vol_str,
                vol_color,
                end_str,
                end_color,
                cat_str,
                cat_color,
                held: held_color.get(&m.condition_id).copied(),
            })
        })
        .collect();

    const LABEL_CAP: usize = 10;
    let max_label = mrows
        .iter()
        .filter_map(|r| r.lead.as_ref().map(|l| l.label.chars().count()))
        .max()
        .unwrap_or(3)
        .min(LABEL_CAP)
        .max("Yes".len());
    let max_pct = 4; // "100%" / " 95%"
                     // label + 2-space gap + bar + 2-space gap + pct
    let prices_col_w = (max_label + 2 + BAR_W + 2 + max_pct).max("Prices".len());
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
        .max("Category".len());

    // Row layout (rendered inside the bordered list):
    //   highlight "▸ "(2) + gutter "★●"(2) + " "(1) + q_width
    //   + "    "(4, no bullet) + max_prices
    //   + "  · "(4) + max_vol + "  · "(4) + max_ends + "  · "(4) + max_cat
    let fixed = 2 /* highlight */ + 2 /* gutter */ + 1 /* gutter→q gap */
        + 4 /* q→prices spacer */ + 3 * 4 /* bullet separators */
        + prices_col_w + max_vol + max_ends + max_cat;
    let q_avail = (area.width as usize)
        .saturating_sub(2) // borders
        .saturating_sub(fixed);
    // Cap q_width at the longest actual question+suffix (+2 slack) so short
    // questions don't leave a wide dead gap before the Prices column.
    let max_q_content = mrows
        .iter()
        .map(|r| {
            r.question.chars().count()
                + r.weather_suffix
                    .as_deref()
                    .map(|s| s.chars().count())
                    .unwrap_or(0)
        })
        .max()
        .unwrap_or(20);
    let q_width = q_avail.min(max_q_content + 2).max(20);

    // ── Header row ────────────────────────────────────────────────────────────
    // Indent = highlight(2) + gutter(2) + gap(1) = 5 chars to align with question.
    let header = Line::from(vec![
        Span::raw("     "),
        Span::styled(
            pad_right("Market".to_string(), q_width),
            Style::default().fg(theme::DIM),
        ),
        Span::raw("    "),
        Span::styled(
            pad_right("Prices".to_string(), prices_col_w),
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
            pad_right("Category".to_string(), max_cat),
            Style::default().fg(theme::DIM),
        ),
    ]);

    f.render_widget(ratatui::widgets::Paragraph::new(header), content_chunks[1]);

    let items: Vec<ListItem> = mrows
        .into_iter()
        .map(|r| build_row(r, q_width, max_label, max_vol, max_ends, max_cat))
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

struct LeadCell {
    label: String,
    pct: String,
    prob: f64,
    color: Color,
}

const BAR_W: usize = 12;

struct MRow {
    question: String,
    weather_suffix: Option<String>,
    starred: bool,
    lead: Option<LeadCell>,
    vol_str: String,
    vol_color: Color,
    end_str: String,
    end_color: Color,
    cat_str: String,
    cat_color: Color,
    held: Option<Color>,
}

fn build_row(
    r: MRow,
    q_width: usize,
    max_label: usize,
    max_vol: usize,
    max_ends: usize,
    max_cat: usize,
) -> ListItem<'static> {
    let MRow {
        question,
        weather_suffix,
        starred,
        lead,
        vol_str,
        vol_color,
        end_str,
        end_color,
        cat_str,
        cat_color,
        held,
    } = r;

    let suffix_w = weather_suffix
        .as_deref()
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let q_avail = q_width.saturating_sub(suffix_w);
    let q_truncated = truncate(&question, q_avail);
    let q_used = q_truncated.chars().count() + suffix_w;
    let q_pad = q_width.saturating_sub(q_used);

    // ── Gutter: ★ for starred, ● colored by held P&L direction ──────────────
    let star_span = if starred {
        Span::styled("★", Style::default().fg(theme::YELLOW))
    } else {
        Span::raw(" ")
    };
    let held_span = match held {
        Some(c) => Span::styled("●", Style::default().fg(c)),
        None => Span::raw(" "),
    };

    let mut spans: Vec<Span> = vec![star_span, held_span, Span::raw(" ")];

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

    // Plain spacer (no orphan bullet) between Market and Prices.
    spans.push(Span::raw("    "));
    let prices_col_w = max_label + 2 + BAR_W + 2 + 4;
    match lead {
        Some(l) => {
            let label = pad_right(truncate(&l.label, max_label), max_label);
            let bar = render_bar(l.prob, BAR_W);
            let pct = pad_left(l.pct.clone(), 4);
            spans.push(Span::styled(label, Style::default().fg(l.color)));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(bar, Style::default().fg(l.color)));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                pct,
                Style::default().fg(l.color).add_modifier(Modifier::BOLD),
            ));
        }
        None => {
            spans.push(Span::raw(" ".repeat(prices_col_w)));
        }
    }

    spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
    spans.push(Span::styled(
        pad_left(vol_str, max_vol),
        Style::default().fg(vol_color),
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

fn lead_cell(m: &Market) -> Option<LeadCell> {
    let leader = m.outcomes.iter().max_by(|a, b| {
        a.price
            .partial_cmp(&b.price)
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    Some(LeadCell {
        label: leader.name.clone(),
        pct: format!("{:.0}%", leader.price * 100.0),
        prob: leader.price,
        color: prob_color(leader.price),
    })
}

/// Horizontal probability bar with eighth-cell precision.
/// `width` is the total cell width; output is exactly `width` columns wide.
fn render_bar(p: f64, width: usize) -> String {
    let p = p.clamp(0.0, 1.0);
    let total_eighths = (p * width as f64 * 8.0).round() as usize;
    let full = total_eighths / 8;
    let partial = total_eighths % 8;
    let partial_char = match partial {
        1 => "▏",
        2 => "▎",
        3 => "▍",
        4 => "▌",
        5 => "▋",
        6 => "▊",
        7 => "▉",
        _ => "",
    };
    let mut s = String::new();
    for _ in 0..full {
        s.push('█');
    }
    s.push_str(partial_char);
    let used = full + if partial > 0 { 1 } else { 0 };
    for _ in used..width {
        s.push(' ');
    }
    s
}

/// Brighten the volume cell as the market gets bigger so high-volume rows
/// jump out at a glance. Log-step thresholds, not a true gradient.
fn volume_color(v: f64) -> Color {
    if v >= 10_000.0 {
        theme::TEXT
    } else if v >= 100.0 {
        theme::DIM
    } else {
        theme::VERY_DIM
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
