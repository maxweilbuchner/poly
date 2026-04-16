use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Bar, BarChart, BarGroup, Block, Chart, Dataset, GraphType, Paragraph},
    Frame,
};

/// Regression result: `(slope, intercept, fitted_points)`.
type Regression = (f64, f64, Vec<(f64, f64)>);

fn fmt_vol(v: f64) -> String {
    if v >= 1_000_000_000.0 {
        format!("${:.1}B", v / 1_000_000_000.0)
    } else if v >= 1_000_000.0 {
        format!("${:.0}M", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("${:.0}K", v / 1_000.0)
    } else {
        format!("${:.0}", v)
    }
}

use crate::tui::{theme, App, CalibCell, CALIB_CATEGORIES};

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    // ── Top panel: collapsed (1-line strip) or expanded (7-line panel + summary) ─
    let charts_area = if app.analytics_panel_collapsed {
        let rows = Layout::vertical([
            Constraint::Length(1), // collapsed status strip
            Constraint::Min(0),
        ])
        .split(area);
        render_collapsed_strip(f, rows[0], app);
        rows[1]
    } else {
        let rows = Layout::vertical([
            Constraint::Length(7), // snapshot status panel
            Constraint::Min(0),
        ])
        .split(area);
        render_snapshot_panel(f, rows[0], app);
        rows[1]
    };

    if app.analytics_stats.is_none() {
        let msg = if app.analytics_loading {
            let sp = SPINNER[(app.tick / 4) as usize % SPINNER.len()];
            format!("{} Computing analytics…", sp)
        } else {
            "No analytics data — press p to pull markets, then r to recompute.".to_string()
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(msg, Style::default().fg(theme::DIM)),
            ])),
            charts_area,
        );
        return;
    }

    // ── Chart grid ────────────────────────────────────────────────────────────
    // Row 0 (50%): [A: Prob Distribution] | [B: Resolution Bias]
    // Row 1 (50%): [C: Calibration      ] | [D: Volume by Cat  ]
    let chart_rows = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(charts_area);

    let top_cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chart_rows[0]);

    let bot_cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chart_rows[1]);

    render_prob_distribution(f, top_cols[0], app, false);
    render_resolution_bias(f, top_cols[1], app, false);
    render_high_confidence(f, bot_cols[0], app, false);
    render_calibration_matrix(f, bot_cols[1], app, false);
}

// ── Collapsed 1-line status strip ────────────────────────────────────────────

fn render_collapsed_strip(f: &mut Frame, area: Rect, app: &App) {
    let status_span = if app.snapshot_in_progress {
        let sp = SPINNER[(app.tick / 4) as usize % SPINNER.len()];
        Span::styled(
            format!("{} Fetching {} mkts…", sp, app.snapshot_fetched_so_far),
            Style::default().fg(theme::YELLOW),
        )
    } else if let Some(t) = app.snapshot_last_at {
        let secs = (chrono::Utc::now() - t).num_seconds().max(0) as u64;
        let ago = if secs < 60 {
            format!("{}s ago", secs)
        } else if secs < 3600 {
            format!("{}m {}s ago", secs / 60, secs % 60)
        } else {
            format!("{}h ago", secs / 3600)
        };
        let remaining = 3600u64.saturating_sub(secs);
        let next = if remaining == 0 {
            "soon".to_string()
        } else if remaining < 60 {
            format!("in {}s", remaining)
        } else {
            format!("in {}m", remaining / 60)
        };
        Span::styled(
            format!("Last {}  ·  Next {}", ago, next),
            Style::default().fg(theme::DIM),
        )
    } else {
        Span::styled(
            "No snapshot yet — press p",
            Style::default().fg(theme::VERY_DIM),
        )
    };

    let spans = vec![
        Span::styled(" ▸ ", Style::default().fg(theme::VERY_DIM)),
        status_span,
        Span::styled("  s expand", Style::default().fg(theme::VERY_DIM)),
    ];
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::BG)),
        area,
    );
}

// ── Compact snapshot status panel ─────────────────────────────────────────────

fn render_snapshot_panel(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered()
        .title(Span::styled(
            " Market Snapshot ",
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::PANEL_BG));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // 5 inner lines: blank, status+next, blank, resolutions, error
    let chunks = Layout::vertical([
        Constraint::Length(1), // 0: blank
        Constraint::Length(1), // 1: status / last  ·  next (combined)
        Constraint::Length(1), // 2: blank
        Constraint::Length(1), // 3: resolutions count
        Constraint::Length(1), // 4: error (or blank)
    ])
    .split(inner);

    // ── Combined status + next line ───────────────────────────────────────────
    // Split row 1 horizontally: [left: status/last] [right: next]
    let status_cols = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[1]);

    let status_line = if app.snapshot_in_progress {
        let spinner = SPINNER[(app.tick / 4) as usize % SPINNER.len()];
        Line::from(vec![
            Span::styled("  Status  ", Style::default().fg(theme::DIM)),
            Span::styled(
                format!("{} Fetching…", spinner),
                Style::default()
                    .fg(theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} markets", app.snapshot_fetched_so_far),
                Style::default().fg(theme::TEXT),
            ),
        ])
    } else if let Some(t) = app.snapshot_last_at {
        let secs = (chrono::Utc::now() - t).num_seconds().max(0) as u64;
        let ago = if secs < 60 {
            format!("{}s ago", secs)
        } else if secs < 3600 {
            format!("{}m {}s ago", secs / 60, secs % 60)
        } else {
            format!("{}h ago", secs / 3600)
        };
        Line::from(vec![
            Span::styled("  Last    ", Style::default().fg(theme::DIM)),
            Span::styled(ago, Style::default().fg(theme::TEXT)),
            Span::styled("  · ", Style::default().fg(theme::VERY_DIM)),
            Span::styled(
                format!("{} mkts", app.snapshot_last_count),
                Style::default().fg(theme::DIM),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("  Last    ", Style::default().fg(theme::DIM)),
            Span::styled("never run", Style::default().fg(theme::VERY_DIM)),
        ])
    };
    f.render_widget(Paragraph::new(status_line), status_cols[0]);

    let next_line = if app.snapshot_in_progress {
        Line::from(Span::styled(
            "next  after this pull",
            Style::default().fg(theme::VERY_DIM),
        ))
    } else {
        match app.snapshot_last_at {
            None => {
                let ticks_left = 600u64.saturating_sub(app.tick);
                let secs_left = ticks_left / 20;
                Line::from(vec![
                    Span::styled("next  ", Style::default().fg(theme::DIM)),
                    Span::styled(
                        if secs_left == 0 {
                            "soon".to_string()
                        } else {
                            format!("in ~{}s (auto)", secs_left)
                        },
                        Style::default().fg(theme::CYAN),
                    ),
                ])
            }
            Some(t) => {
                let elapsed = (chrono::Utc::now() - t).num_seconds().max(0) as u64;
                let remaining = 3600u64.saturating_sub(elapsed);
                let label = if remaining == 0 {
                    "soon".to_string()
                } else if remaining < 60 {
                    format!("in {}s", remaining)
                } else {
                    format!("in {}m {}s", remaining / 60, remaining % 60)
                };
                Line::from(vec![
                    Span::styled("next  ", Style::default().fg(theme::DIM)),
                    Span::styled(label, Style::default().fg(theme::CYAN)),
                ])
            }
        }
    };
    f.render_widget(Paragraph::new(next_line), status_cols[1]);

    // ── Summary stats (analytics) or fallback resolved count ─────────────────
    if let Some(stats) = &app.analytics_stats {
        let active = stats.prob_buckets.iter().sum::<u64>();
        let resolved = stats.res_yes + stats.res_no + stats.res_other;
        let vol_str = fmt_vol(stats.total_volume);
        let sep = Span::styled("  ·  ", Style::default().fg(theme::VERY_DIM));
        let mut spans: Vec<Span> = vec![
            Span::raw("  "),
            Span::styled("Total ", Style::default().fg(theme::DIM)),
            Span::styled(
                format!("{}", stats.total_markets),
                Style::default().fg(theme::TEXT),
            ),
            sep.clone(),
            Span::styled("Volume ", Style::default().fg(theme::DIM)),
            Span::styled(vol_str, Style::default().fg(theme::CYAN)),
            sep.clone(),
            Span::styled("Priced ", Style::default().fg(theme::DIM)),
            Span::styled(format!("{}", active), Style::default().fg(theme::TEXT)),
            sep.clone(),
            Span::styled("Resolved ", Style::default().fg(theme::DIM)),
            Span::styled(format!("{}", resolved), Style::default().fg(theme::TEXT)),
        ];
        let binary_resolved = stats.res_yes + stats.res_no;
        if binary_resolved > 0 {
            let yes_pct = stats.res_yes as f64 / binary_resolved as f64 * 100.0;
            spans.push(sep);
            spans.push(Span::styled("YES rate ", Style::default().fg(theme::DIM)));
            spans.push(Span::styled(
                format!("{:.0}%", yes_pct),
                Style::default().fg(theme::GREEN),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), chunks[3]);
    } else {
        let total_resolved = app.known_resolved_ids.len();
        let mut res_spans: Vec<Span> = vec![
            Span::styled("  Resolved    ", Style::default().fg(theme::DIM)),
            Span::styled(
                format!("{} stored", total_resolved),
                Style::default().fg(theme::TEXT),
            ),
        ];
        if app.resolutions_new_last_run > 0 {
            res_spans.push(Span::styled("  · ", Style::default().fg(theme::VERY_DIM)));
            res_spans.push(Span::styled(
                format!("+{} new", app.resolutions_new_last_run),
                Style::default().fg(theme::GREEN),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(res_spans)), chunks[3]);
    }

    // ── Error (or blank) ──────────────────────────────────────────────────────
    if let Some(err) = &app.snapshot_error {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  Error       ", Style::default().fg(theme::DIM)),
                Span::styled(err.clone(), Style::default().fg(theme::ERROR)),
            ])),
            chunks[4],
        );
    }
}

// ── Shared block constructor ───────────────────────────────────────────────────

fn make_block(title: &str, focused: bool) -> Block<'_> {
    let border_color = if focused {
        theme::BORDER_ACTIVE
    } else {
        theme::BORDER
    };
    Block::bordered()
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme::PANEL_BG))
}

fn empty_state<'a>(msg: &'a str) -> Paragraph<'a> {
    Paragraph::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(msg, Style::default().fg(theme::VERY_DIM)),
    ]))
}

// ── B: Active market probability distribution ─────────────────────────────────

fn render_prob_distribution(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let Some(stats) = &app.analytics_stats else {
        f.render_widget(
            empty_state("loading…").block(make_block(" A: Yes Probability Distribution ", focused)),
            area,
        );
        return;
    };

    let total: u64 = stats.prob_buckets.iter().sum();
    let median_pct_label = {
        let half = total / 2;
        let mut cum = 0u64;
        let mut med_pct = 0usize;
        for (i, &v) in stats.prob_buckets.iter().enumerate() {
            cum += v;
            if cum >= half {
                med_pct = i * 5;
                break;
            }
        }
        med_pct
    };
    let snap_suffix = if app.snapshot_in_progress {
        let sp = SPINNER[(app.tick / 3) as usize % SPINNER.len()];
        format!(" {} ", sp)
    } else {
        " ".to_string()
    };
    let title = if total > 0 {
        format!(
            " A: Yes Probability Distribution  ({} markets, median ~{}%){}",
            total, median_pct_label, snap_suffix
        )
    } else {
        format!(
            " A: Yes Probability Distribution  ({} markets){}",
            total, snap_suffix
        )
    };

    // Draw the border first so we can measure the real inner rect.
    let block = make_block(&title, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Dynamic bar width based on actual inner width (20 bars + 19 gaps).
    let inner_w = inner.width as usize;
    let bar_width = ((inner_w.saturating_sub(19)) / 20).clamp(1, 10) as u16;

    // Center the bars: compute how much space the bars actually need,
    // then push the chart rect right by half the leftover.
    let needed_w = 20 * bar_width + 19; // 19 one-column gaps
    let left_pad = inner.width.saturating_sub(needed_w) / 2;
    let chart_area = Rect {
        x: inner.x + left_pad,
        y: inner.y,
        width: needed_w.min(inner.width),
        height: inner.height,
    };

    // Shorter labels when bars are too narrow to show "X%".
    let labels: Vec<String> = (0..20usize)
        .map(|i| {
            if bar_width >= 3 {
                format!("{}%", i * 5)
            } else {
                format!("{}", i * 5)
            }
        })
        .collect();

    // Median bucket: walk from left until cumulative count >= 50% of total.
    let median_bucket = if total > 0 {
        let half = total / 2;
        let mut cum = 0u64;
        let mut med = 0usize;
        for (i, &v) in stats.prob_buckets.iter().enumerate() {
            cum += v;
            if cum >= half {
                med = i;
                break;
            }
        }
        Some(med)
    } else {
        None
    };

    let bars: Vec<Bar> = stats
        .prob_buckets
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let is_median = median_bucket == Some(i);
            let color = if is_median {
                theme::YELLOW
            } else {
                theme::CYAN
            };
            Bar::default()
                .value(v)
                .label(Line::from(labels[i].as_str()))
                .style(Style::default().fg(color))
        })
        .collect();

    // No .block() — border already rendered above.
    let chart = BarChart::default()
        .data(BarGroup::default().bars(&bars))
        .bar_width(bar_width)
        .bar_gap(1)
        .value_style(
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )
        .label_style(Style::default().fg(theme::DIM));

    f.render_widget(chart, chart_area);
}

// ── D: Calibration-fit matrix (categories × volume tiers) ─────────────────────
//
// Same WLS/OLS regression as panel C, but computed per cell. Each cell shows
// the two endpoints of the fit — the predicted probability at which actual
// resolution is forecast to be 0% and 100%. For perfectly-calibrated markets
// those would be 0 and 100; realistic markets collapse toward the mean, e.g.
// 16/86 means the regression line hits 0% actual at predicted=16% and 100%
// actual at predicted=86%.

const TIER_LABELS: [&str; 5] = ["<1K", "1-10K", "10-100K", "100K-1M", ">1M"];

fn fit_endpoints(buckets: &[(u32, u32); 10], weighted: bool) -> Option<(f64, f64)> {
    let pts: Vec<(f64, f64, f64)> = (0..10usize)
        .filter_map(|b| {
            let (yes, total) = buckets[b];
            if total == 0 {
                return None;
            }
            let x = b as f64 * 10.0 + 5.0;
            let y = yes as f64 / total as f64 * 100.0;
            let w = if weighted { total as f64 } else { 1.0 };
            Some((x, y, w))
        })
        .collect();
    if pts.len() < 2 {
        return None;
    }
    let sum_w: f64 = pts.iter().map(|(_, _, w)| *w).sum();
    let sum_wx: f64 = pts.iter().map(|(x, _, w)| w * x).sum();
    let sum_wy: f64 = pts.iter().map(|(_, y, w)| w * y).sum();
    let sum_wxx: f64 = pts.iter().map(|(x, _, w)| w * x * x).sum();
    let sum_wxy: f64 = pts.iter().map(|(x, y, w)| w * x * y).sum();
    let denom = sum_w * sum_wxx - sum_wx * sum_wx;
    if denom.abs() < 1e-10 {
        return None;
    }
    let m = (sum_w * sum_wxy - sum_wx * sum_wy) / denom;
    let b = (sum_wy - m * sum_wx) / sum_w;
    if m.abs() < 1e-10 {
        return None;
    }
    Some((-b / m, (100.0 - b) / m))
}

fn aggregate_cells(cells: &[CalibCell]) -> CalibCell {
    let mut out = CalibCell::default();
    for c in cells {
        for b in 0..10 {
            out.buckets[b].0 += c.buckets[b].0;
            out.buckets[b].1 += c.buckets[b].1;
        }
        out.n += c.n;
    }
    out
}

/// Interpolate green → yellow → red along `t ∈ [0, 1]`.
fn spread_color(t: f64) -> ratatui::style::Color {
    use ratatui::style::Color;
    let t = t.clamp(0.0, 1.0);
    // Stops: green (62,224,126) at 0.0, yellow (238,172,50) at 0.5, red (240,80,80) at 1.0.
    let (r, g, b) = if t < 0.5 {
        let u = t / 0.5;
        (
            lerp(62.0, 238.0, u),
            lerp(224.0, 172.0, u),
            lerp(126.0, 50.0, u),
        )
    } else {
        let u = (t - 0.5) / 0.5;
        (
            lerp(238.0, 240.0, u),
            lerp(172.0, 80.0, u),
            lerp(50.0, 80.0, u),
        )
    };
    Color::Rgb(r as u8, g as u8, b as u8)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn cell_span(
    cell: &CalibCell,
    weighted: bool,
    w: usize,
    bold: bool,
    spread_min: Option<f64>,
    spread_max: Option<f64>,
) -> Span<'static> {
    let dash = format!("{:>w$}", "·", w = w);
    if cell.n < 10 {
        return Span::styled(dash, Style::default().fg(theme::VERY_DIM));
    }
    let Some((xz, xh)) = fit_endpoints(&cell.buckets, weighted) else {
        return Span::styled(dash, Style::default().fg(theme::VERY_DIM));
    };
    let spread = xh - xz;
    let color = match (spread_min, spread_max) {
        (Some(lo), Some(hi)) if hi > lo + 1e-9 => spread_color((spread - lo) / (hi - lo)),
        _ => theme::YELLOW,
    };
    let xz_i = xz.round().clamp(-99.0, 999.0) as i32;
    let xh_i = xh.round().clamp(-99.0, 999.0) as i32;
    let label = format!("{}/{}", xz_i, xh_i);
    let text = format!("{:>w$}", label, w = w);
    let mut style = Style::default().fg(color);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Span::styled(text, style)
}

fn render_calibration_matrix(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let cal_suffix = if app.analytics_loading {
        let sp = SPINNER[(app.tick / 3) as usize % SPINNER.len()];
        if app.calibration_fetch_total > 0 {
            format!(
                " {} {}/{} ",
                sp, app.calibration_fetch_done, app.calibration_fetch_total
            )
        } else {
            format!(" {} ", sp)
        }
    } else {
        " ".to_string()
    };
    let fit_label = if app.regression_weighted {
        "WLS"
    } else {
        "OLS"
    };
    let title = format!(
        " D: Calibration Fit · category × volume  (−{}h, {}){}",
        app.calibration_hours, fit_label, cal_suffix
    );
    let block = make_block(&title, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(stats) = &app.analytics_stats else {
        f.render_widget(empty_state("loading…"), inner);
        return;
    };

    let total: u32 = stats.calibration_matrix.iter().flatten().map(|c| c.n).sum();
    if total == 0 {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No calibration data yet.",
                Style::default().fg(theme::VERY_DIM),
            )),
            Line::from(Span::styled(
                "  Press r to recompute after a market snapshot.",
                Style::default().fg(theme::VERY_DIM),
            )),
        ];
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // Aggregates for the "All" row/column.
    let all_per_tier: Vec<CalibCell> = (0..5)
        .map(|t| {
            let col: Vec<CalibCell> = (0..6).map(|c| stats.calibration_matrix[c][t]).collect();
            aggregate_cells(&col)
        })
        .collect();
    let all_per_cat: Vec<CalibCell> = (0..6)
        .map(|c| aggregate_cells(&stats.calibration_matrix[c]))
        .collect();
    let grand = aggregate_cells(&all_per_tier);

    // Compute spread (xh − xz) for every visible cell with a valid fit, then
    // use the observed range to rank colors: smallest spread = green, largest
    // = red. Aggregates are included so "All" cells share the same scale.
    let mut spreads: Vec<f64> = Vec::new();
    let mut push_spread = |cell: &CalibCell| {
        if cell.n < 10 {
            return;
        }
        if let Some((xz, xh)) = fit_endpoints(&cell.buckets, app.regression_weighted) {
            spreads.push(xh - xz);
        }
    };
    for row in stats.calibration_matrix.iter() {
        for c in row.iter() {
            push_spread(c);
        }
    }
    for c in all_per_tier.iter() {
        push_spread(c);
    }
    for c in all_per_cat.iter() {
        push_spread(c);
    }
    push_spread(&grand);
    let spread_min = spreads.iter().cloned().fold(f64::INFINITY, f64::min);
    let spread_max = spreads.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let (spread_min, spread_max) = if spreads.is_empty() {
        (None, None)
    } else {
        (Some(spread_min), Some(spread_max))
    };

    // Layout: dynamic column width so things still fit in narrow panels.
    let label_w: usize = CALIB_CATEGORIES
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(8);
    let inner_w = inner.width as usize;
    let cell_w: usize = {
        // Need: 2 (left pad) + label_w + 1 + 6 cells × cell_w
        let avail = inner_w.saturating_sub(2 + label_w + 1);
        (avail / 6).clamp(6, 10)
    };

    // Header row: blank label cell + tier labels + "All".
    let mut header_spans: Vec<Span<'static>> =
        vec![Span::raw(format!("  {:<w$} ", "", w = label_w))];
    for tl in TIER_LABELS.iter() {
        header_spans.push(Span::styled(
            format!("{:>w$}", tl, w = cell_w),
            Style::default().fg(theme::VERY_DIM),
        ));
    }
    header_spans.push(Span::styled(
        format!("{:>w$}", "All", w = cell_w),
        Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
    ));

    let mut lines: Vec<Line<'static>> = vec![Line::from(header_spans), Line::from("")];

    // Category rows.
    for (ci, cat) in CALIB_CATEGORIES.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            format!("  {:<w$} ", cat, w = label_w),
            Style::default().fg(theme::TEXT),
        )];
        for ti in 0..5 {
            spans.push(cell_span(
                &stats.calibration_matrix[ci][ti],
                app.regression_weighted,
                cell_w,
                false,
                spread_min,
                spread_max,
            ));
        }
        spans.push(cell_span(
            &all_per_cat[ci],
            app.regression_weighted,
            cell_w,
            true,
            spread_min,
            spread_max,
        ));
        lines.push(Line::from(spans));
    }

    // All row.
    lines.push(Line::from(""));
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        format!("  {:<w$} ", "All", w = label_w),
        Style::default()
            .fg(theme::CYAN)
            .add_modifier(Modifier::BOLD),
    )];
    for tier in all_per_tier.iter().take(5) {
        spans.push(cell_span(
            tier,
            app.regression_weighted,
            cell_w,
            true,
            spread_min,
            spread_max,
        ));
    }
    spans.push(cell_span(
        &grand,
        app.regression_weighted,
        cell_w,
        true,
        spread_min,
        spread_max,
    ));
    lines.push(Line::from(spans));

    f.render_widget(Paragraph::new(lines), inner);
}

// ── B: Resolution bias (paragraph with Unicode bars) ─────────────────────────

fn render_resolution_bias(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let snap_suffix = if app.snapshot_in_progress {
        let sp = SPINNER[(app.tick / 3) as usize % SPINNER.len()];
        format!(" {} ", sp)
    } else {
        " ".to_string()
    };
    let b_title = format!(" B: Resolution Bias{}", snap_suffix);

    let Some(stats) = &app.analytics_stats else {
        f.render_widget(
            empty_state("loading…").block(make_block(&b_title, focused)),
            area,
        );
        return;
    };

    let total = (stats.res_yes + stats.res_no + stats.res_other) as f64;

    if total == 0.0 {
        let block = make_block(&b_title, focused);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No resolutions recorded yet.",
                Style::default().fg(theme::VERY_DIM),
            )),
            Line::from(Span::styled(
                "  Markets resolve over time — check back after a snapshot.",
                Style::default().fg(theme::VERY_DIM),
            )),
        ];
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let yes_pct = stats.res_yes as f64 / total;
    let no_pct = stats.res_no as f64 / total;
    let oth_pct = stats.res_other as f64 / total;

    // Each outcome gets two bar rows (fat bars) for visual weight, plus a blank
    // separator. Build the content first, then vertically center it.
    let block = make_block(&b_title, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Dynamic bar width: label(7) + pct(6) + count(~9) + right margin(2) = 24.
    let bar_w = (inner.width as usize).saturating_sub(24).clamp(6, 60);
    let make_bar = |pct: f64| -> (String, String) {
        let filled = (pct * bar_w as f64).round() as usize;
        let empty = bar_w.saturating_sub(filled);
        ("█".repeat(filled), "░".repeat(empty))
    };

    let (yes_f, yes_e) = make_bar(yes_pct);
    let (no_f, no_e) = make_bar(no_pct);
    let (oth_f, oth_e) = make_bar(oth_pct);

    let content: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled("  Total   ", Style::default().fg(theme::DIM)),
            Span::styled(
                format!("{} resolved markets", total as usize),
                Style::default().fg(theme::TEXT),
            ),
        ]),
        Line::from(""),
        // YES — row 1: label + bar + stats
        Line::from(vec![
            Span::styled(
                "  YES  ",
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(yes_f.clone(), Style::default().fg(theme::GREEN)),
            Span::styled(yes_e.clone(), Style::default().fg(theme::VERY_DIM)),
            Span::styled(
                format!("  {:>3.0}%", yes_pct * 100.0),
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  n={}", stats.res_yes),
                Style::default().fg(theme::DIM),
            ),
        ]),
        // YES — row 2: bar repeated for height
        Line::from(vec![
            Span::raw("       "),
            Span::styled(yes_f, Style::default().fg(theme::GREEN)),
            Span::styled(yes_e, Style::default().fg(theme::VERY_DIM)),
        ]),
        Line::from(""),
        // NO — row 1
        Line::from(vec![
            Span::styled(
                "  NO   ",
                Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
            ),
            Span::styled(no_f.clone(), Style::default().fg(theme::RED)),
            Span::styled(no_e.clone(), Style::default().fg(theme::VERY_DIM)),
            Span::styled(
                format!("  {:>3.0}%", no_pct * 100.0),
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  n={}", stats.res_no),
                Style::default().fg(theme::DIM),
            ),
        ]),
        // NO — row 2
        Line::from(vec![
            Span::raw("       "),
            Span::styled(no_f, Style::default().fg(theme::RED)),
            Span::styled(no_e, Style::default().fg(theme::VERY_DIM)),
        ]),
        Line::from(""),
        // OTHER — row 1
        Line::from(vec![
            Span::styled(
                "  OTHER",
                Style::default()
                    .fg(theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(oth_f.clone(), Style::default().fg(theme::YELLOW)),
            Span::styled(oth_e.clone(), Style::default().fg(theme::VERY_DIM)),
            Span::styled(
                format!("  {:>3.0}%", oth_pct * 100.0),
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  n={}", stats.res_other),
                Style::default().fg(theme::DIM),
            ),
        ]),
        // OTHER — row 2
        Line::from(vec![
            Span::raw("       "),
            Span::styled(oth_f, Style::default().fg(theme::YELLOW)),
            Span::styled(oth_e, Style::default().fg(theme::VERY_DIM)),
        ]),
    ];

    // Vertically center: prepend (available - content) / 2 blank lines.
    let top_pad = inner.height.saturating_sub(content.len() as u16) / 2;
    let mut lines: Vec<Line<'static>> = (0..top_pad).map(|_| Line::from("")).collect();
    lines.extend(content);

    f.render_widget(Paragraph::new(lines), inner);
}

// ── C: Calibration curve (XY scatter) ────────────────────────────────────────
//
// X axis: predicted probability (bucket midpoint, 5–95%).
// Y axis: actual YES-resolution rate for that bucket.
// Reference diagonal: perfect calibration (x = y).
// Point color: green ≤15pp off diagonal, yellow ≤25pp, red >25pp.

fn render_high_confidence(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let Some(stats) = &app.analytics_stats else {
        f.render_widget(
            empty_state("loading…").block(make_block(" C: Calibration Curve", focused)),
            area,
        );
        return;
    };

    let total_samples: usize = stats.calibration.iter().map(|(_, t)| t).sum();
    let cal_suffix = if app.analytics_loading {
        let sp = SPINNER[(app.tick / 3) as usize % SPINNER.len()];
        if app.calibration_fetch_total > 0 {
            format!(
                " {} {}/{} ",
                sp, app.calibration_fetch_done, app.calibration_fetch_total
            )
        } else {
            format!(" {} ", sp)
        }
    } else {
        " ".to_string()
    };
    let fit_label = if app.regression_weighted {
        "WLS"
    } else {
        "OLS"
    };
    let title = format!(
        " C: Calibration Curve (−{}h before close, n={}, {}){}",
        app.calibration_hours, total_samples, fit_label, cal_suffix
    );

    if total_samples == 0 {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No calibration data yet.",
                    Style::default().fg(theme::VERY_DIM),
                )),
                Line::from(Span::styled(
                    "  Press r to recompute after a snapshot.",
                    Style::default().fg(theme::VERY_DIM),
                )),
            ])
            .block(make_block(&title, focused)),
            area,
        );
        return;
    }

    // Reference diagonal: x = y (perfect calibration), 101 points for smooth braille line.
    let diagonal: Vec<(f64, f64)> = (0..=100).map(|i| (i as f64, i as f64)).collect();

    // Data points split by colour based on deviation from ideal.
    let mut green_pts: Vec<(f64, f64)> = Vec::new();
    let mut yellow_pts: Vec<(f64, f64)> = Vec::new();
    let mut red_pts: Vec<(f64, f64)> = Vec::new();

    for b in 0..10usize {
        let (yes, total) = stats.calibration[b];
        if total == 0 {
            continue;
        }
        let expected = b as f64 * 10.0 + 5.0; // bucket midpoint
        let actual = yes as f64 / total as f64 * 100.0;
        let diff = (actual - expected).abs();
        let pt = (expected, actual);
        if diff <= 15.0 {
            green_pts.push(pt);
        } else if diff <= 25.0 {
            yellow_pts.push(pt);
        } else {
            red_pts.push(pt);
        }
    }

    // Regression: WLS weights each bucket by its observation count; OLS treats all equally.
    let regression: Option<Regression> = {
        let wpts: Vec<(f64, f64, f64)> = (0..10usize)
            .filter_map(|b| {
                let (yes, total) = stats.calibration[b];
                if total == 0 {
                    return None;
                }
                let x = b as f64 * 10.0 + 5.0;
                let y = yes as f64 / total as f64 * 100.0;
                let w = if app.regression_weighted {
                    total as f64
                } else {
                    1.0
                };
                Some((x, y, w))
            })
            .collect();
        if wpts.len() >= 2 {
            let sum_w: f64 = wpts.iter().map(|(_, _, w)| w).sum();
            let sum_wx: f64 = wpts.iter().map(|(x, _, w)| w * x).sum();
            let sum_wy: f64 = wpts.iter().map(|(_, y, w)| w * y).sum();
            let sum_wxx: f64 = wpts.iter().map(|(x, _, w)| w * x * x).sum();
            let sum_wxy: f64 = wpts.iter().map(|(x, y, w)| w * x * y).sum();
            let denom = sum_w * sum_wxx - sum_wx * sum_wx;
            if denom.abs() < 1e-10 {
                None
            } else {
                let m = (sum_w * sum_wxy - sum_wx * sum_wy) / denom;
                let b = (sum_wy - m * sum_wx) / sum_w;
                let pts: Vec<(f64, f64)> = (0..=100)
                    .map(|i| {
                        let x = i as f64;
                        (x, (m * x + b).clamp(0.0, 100.0))
                    })
                    .collect();
                Some((m, b, pts))
            }
        } else {
            None
        }
    };

    let axis_labels = vec![
        Span::styled("0%", Style::default().fg(theme::VERY_DIM)),
        Span::styled("25%", Style::default().fg(theme::VERY_DIM)),
        Span::styled("50%", Style::default().fg(theme::VERY_DIM)),
        Span::styled("75%", Style::default().fg(theme::VERY_DIM)),
        Span::styled("100%", Style::default().fg(theme::VERY_DIM)),
    ];

    let mut datasets: Vec<Dataset<'_>> = vec![
        // Reference diagonal: subtle color, braille for smoothness.
        Dataset::default()
            .data(&diagonal)
            .graph_type(GraphType::Line)
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(theme::BORDER_ACTIVE)),
    ];
    if !green_pts.is_empty() {
        datasets.push(
            Dataset::default()
                .data(&green_pts)
                .graph_type(GraphType::Scatter)
                .marker(symbols::Marker::Dot)
                .style(Style::default().fg(theme::GREEN)),
        );
    }
    if !yellow_pts.is_empty() {
        datasets.push(
            Dataset::default()
                .data(&yellow_pts)
                .graph_type(GraphType::Scatter)
                .marker(symbols::Marker::Dot)
                .style(Style::default().fg(theme::YELLOW)),
        );
    }
    if !red_pts.is_empty() {
        datasets.push(
            Dataset::default()
                .data(&red_pts)
                .graph_type(GraphType::Scatter)
                .marker(symbols::Marker::Dot)
                .style(Style::default().fg(theme::RED)),
        );
    }
    if let Some((_, _, ref reg_pts)) = regression {
        datasets.push(
            Dataset::default()
                .data(reg_pts)
                .graph_type(GraphType::Line)
                .marker(symbols::Marker::Braille)
                .style(Style::default().fg(theme::PURPLE)),
        );
    }

    // Build block: add title_bottom with regression crossings when available.
    let border_color = if focused {
        theme::BORDER_ACTIVE
    } else {
        theme::BORDER
    };
    let mut block = Block::bordered()
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme::CYAN)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme::PANEL_BG));
    if let Some((m, b, _)) = &regression {
        if m.abs() > 1e-10 {
            let x_zero = -b / m;
            let x_hundred = (100.0 - b) / m;
            block = block.title_bottom(Span::styled(
                format!(" fit: 0% at {:.0}%  ·  100% at {:.0}% ", x_zero, x_hundred),
                Style::default().fg(theme::PURPLE),
            ));
        }
    }

    let chart = Chart::new(datasets)
        .block(block)
        .style(Style::default().bg(theme::PANEL_BG))
        .x_axis(
            Axis::default()
                .title(Span::styled("Predicted →", Style::default().fg(theme::DIM)))
                .bounds([0.0, 100.0])
                .labels(axis_labels.clone())
                .style(Style::default().fg(theme::VERY_DIM)),
        )
        .y_axis(
            Axis::default()
                .title(Span::styled("↑ Actual", Style::default().fg(theme::DIM)))
                .bounds([0.0, 100.0])
                .labels(axis_labels)
                .style(Style::default().fg(theme::VERY_DIM)),
        );

    f.render_widget(chart, area);
}
