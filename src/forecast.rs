//! Open-Meteo forecast fetcher for Polymarket weather markets.
//!
//! Pulls the ECMWF-IFS ensemble (51 perturbed members) for daily Tmax/Tmin at
//! the airport-station coordinates we already track, plus a single best_match
//! deterministic anchor. Caches results in-memory with a 30-min TTL and on
//! disk so reopening a market or restarting the binary is instant.
//!
//! Only valid for resolution dates within D+0..D+2 (airport-local). Beyond
//! that, ensemble spread exceeds the typical 1°C market bin width and the
//! per-bin probabilities become uninformative.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::weather::Airport;

const ENSEMBLE_URL: &str = "https://ensemble-api.open-meteo.com/v1/ensemble";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const CACHE_FILE: &str = "forecast.json";
const CACHE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distribution {
    pub mean: f64,
    pub p10: f64,
    pub p90: f64,
    pub members: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Forecast {
    pub high: Distribution,
    pub low: Distribution,
    /// best_match deterministic anchor (single value).
    pub high_anchor: f64,
    pub low_anchor: f64,
    /// Model run/init time as reported by Open-Meteo.
    pub run_time: DateTime<Utc>,
    /// 0 = today, 1 = tomorrow, etc. — relative to airport-local today.
    pub lead_days: u8,
}

/// In-memory + on-disk cache. `Instant` for in-process TTL; the on-disk
/// snapshot uses wall-clock time.
pub struct Cache {
    entries: HashMap<(String, NaiveDate), (Forecast, Instant, DateTime<Utc>)>,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub fn new() -> Self {
        Cache {
            entries: HashMap::new(),
        }
    }

    /// Load the on-disk snapshot. Drops stale entries.
    pub fn load_from_disk() -> Self {
        let mut cache = Cache::new();
        let path = match cache_path() {
            Some(p) => p,
            None => return cache,
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return cache,
        };
        let stored: StoredCache = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => return cache,
        };
        if stored.version != CACHE_VERSION {
            return cache;
        }
        let now = Utc::now();
        for entry in stored.entries {
            let age = now.signed_duration_since(entry.fetched_at);
            if age.num_seconds() < 0 || age.to_std().map(|d| d > CACHE_TTL).unwrap_or(true) {
                continue;
            }
            let remaining = CACHE_TTL - age.to_std().unwrap_or(CACHE_TTL);
            let inserted_at = Instant::now()
                .checked_sub(CACHE_TTL - remaining)
                .unwrap_or_else(Instant::now);
            cache.entries.insert(
                (entry.icao, entry.date),
                (entry.forecast, inserted_at, entry.fetched_at),
            );
        }
        cache
    }

    pub fn save_to_disk(&self) {
        let path = match cache_path() {
            Some(p) => p,
            None => return,
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let entries = self
            .entries
            .iter()
            .map(
                |((icao, date), (forecast, _instant, fetched_at))| StoredEntry {
                    icao: icao.clone(),
                    date: *date,
                    forecast: forecast.clone(),
                    fetched_at: *fetched_at,
                },
            )
            .collect();
        let stored = StoredCache {
            version: CACHE_VERSION,
            entries,
        };
        if let Ok(text) = serde_json::to_string(&stored) {
            let _ = std::fs::write(&path, text);
        }
    }

    pub fn get(&self, icao: &str, date: NaiveDate) -> Option<&Forecast> {
        let entry = self.entries.get(&(icao.to_string(), date))?;
        if entry.1.elapsed() > CACHE_TTL {
            return None;
        }
        Some(&entry.0)
    }

    pub fn insert(&mut self, icao: String, date: NaiveDate, forecast: Forecast) {
        self.entries
            .insert((icao, date), (forecast, Instant::now(), Utc::now()));
    }
}

#[derive(Serialize, Deserialize)]
struct StoredCache {
    version: u32,
    entries: Vec<StoredEntry>,
}

#[derive(Serialize, Deserialize)]
struct StoredEntry {
    icao: String,
    date: NaiveDate,
    forecast: Forecast,
    fetched_at: DateTime<Utc>,
}

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .map(|h| h.join(".local").join("share"))
                .unwrap_or_else(|| PathBuf::from("."))
        });
    Some(base.join("poly").join(CACHE_FILE))
}

// ── Fetch ─────────────────────────────────────────────────────────────────────

/// Open-Meteo ensemble response shape (we only deserialize fields we use).
///
/// Ensemble endpoints return hourly per-member values; daily aggregates are
/// not exposed. We aggregate to daily Tmax/Tmin per member ourselves.
#[derive(Deserialize)]
struct EnsembleResponse {
    hourly: EnsembleHourly,
}

#[derive(Deserialize)]
struct EnsembleHourly {
    /// ISO 8601 timestamps in the requested timezone (one entry per hour).
    time: Vec<String>,
    /// Member series keyed `temperature_2m`, `temperature_2m_member01`, etc.
    #[serde(flatten)]
    series: HashMap<String, Vec<Option<f64>>>,
}

#[derive(Deserialize)]
struct DeterministicResponse {
    daily: DeterministicDaily,
}

#[derive(Deserialize)]
struct DeterministicDaily {
    time: Vec<String>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
}

pub async fn fetch_forecast(
    client: &reqwest::Client,
    airport: &Airport,
    resolution_date: NaiveDate,
) -> Result<Forecast, crate::error::AppError> {
    let date_str = resolution_date.format("%Y-%m-%d").to_string();

    let ensemble_url = format!(
        "{}?latitude={:.4}&longitude={:.4}&elevation={}&timezone=auto\
         &models=ecmwf_ifs025&hourly=temperature_2m\
         &start_date={}&end_date={}",
        ENSEMBLE_URL, airport.lat, airport.lon, airport.elev_m, date_str, date_str
    );
    let deterministic_url = format!(
        "{}?latitude={:.4}&longitude={:.4}&elevation={}&timezone=auto\
         &models=best_match&daily=temperature_2m_max,temperature_2m_min\
         &start_date={}&end_date={}",
        FORECAST_URL, airport.lat, airport.lon, airport.elev_m, date_str, date_str
    );

    let (ensemble_res, deterministic_res) = tokio::try_join!(
        client.get(&ensemble_url).send(),
        client.get(&deterministic_url).send(),
    )
    .map_err(|e| crate::error::AppError::Network(e.to_string()))?;

    let ensemble: EnsembleResponse = ensemble_res
        .error_for_status()
        .map_err(|e| crate::error::AppError::Network(e.to_string()))?
        .json()
        .await
        .map_err(|e| crate::error::AppError::Network(e.to_string()))?;
    let deterministic: DeterministicResponse = deterministic_res
        .error_for_status()
        .map_err(|e| crate::error::AppError::Network(e.to_string()))?
        .json()
        .await
        .map_err(|e| crate::error::AppError::Network(e.to_string()))?;

    let (high_members, low_members) = aggregate_ensemble(&ensemble.hourly, resolution_date);
    if high_members.is_empty() || low_members.is_empty() {
        return Err(crate::error::AppError::Other(
            format!("ensemble returned no members for {}", airport.display).into(),
        ));
    }

    let high_anchor = first_value(&deterministic.daily.temperature_2m_max).ok_or_else(|| {
        crate::error::AppError::Other(
            format!("best_match returned no Tmax for {}", airport.display).into(),
        )
    })?;
    let low_anchor = first_value(&deterministic.daily.temperature_2m_min).ok_or_else(|| {
        crate::error::AppError::Other(
            format!("best_match returned no Tmin for {}", airport.display).into(),
        )
    })?;

    let _ = deterministic.daily.time;

    let today = Utc::now().date_naive();
    let lead_days = (resolution_date - today).num_days().clamp(0, 255) as u8;

    Ok(Forecast {
        high: summarize(high_members),
        low: summarize(low_members),
        high_anchor,
        low_anchor,
        run_time: Utc::now(),
        lead_days,
    })
}

/// For each ensemble member, find the indices in `time` that fall on
/// `target_date` (in the response's local timezone — `timezone=auto` returns
/// `YYYY-MM-DDTHH:MM` strings) and reduce that member's hourly values to
/// (max, min). Returns the lists of per-member daily extremes.
fn aggregate_ensemble(hourly: &EnsembleHourly, target_date: NaiveDate) -> (Vec<f64>, Vec<f64>) {
    let target_prefix = target_date.format("%Y-%m-%d").to_string();
    let day_indices: Vec<usize> = hourly
        .time
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.starts_with(&target_prefix).then_some(i))
        .collect();
    if day_indices.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut highs = Vec::new();
    let mut lows = Vec::new();
    for (key, series) in hourly.series.iter() {
        if !key.starts_with("temperature_2m") {
            continue;
        }
        let mut max = f64::NEG_INFINITY;
        let mut min = f64::INFINITY;
        for &idx in &day_indices {
            if let Some(Some(v)) = series.get(idx) {
                if v.is_finite() {
                    if *v > max {
                        max = *v;
                    }
                    if *v < min {
                        min = *v;
                    }
                }
            }
        }
        if max.is_finite() && min.is_finite() {
            highs.push(max);
            lows.push(min);
        }
    }
    (highs, lows)
}

fn first_value(series: &[Option<f64>]) -> Option<f64> {
    series.first().copied().flatten()
}

fn summarize(mut members: Vec<f64>) -> Distribution {
    members.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = members.iter().sum::<f64>() / members.len() as f64;
    let p10 = percentile(&members, 0.10);
    let p90 = percentile(&members, 0.90);
    Distribution {
        mean,
        p10,
        p90,
        members,
    }
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = pos - lo as f64;
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}

// ── Histogram for the inline display ──────────────────────────────────────────

/// Bucket members into 7 1°C-wide bins centered on the median, returning the
/// counts and the (lo, hi) inclusive integer °C labels for the outer bins.
pub fn histogram(members: &[f64]) -> ([u8; 7], i32, i32) {
    let mut counts = [0u8; 7];
    if members.is_empty() {
        return (counts, 0, 0);
    }
    let mut sorted: Vec<f64> = members.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2].round() as i32;
    let lo = median - 3;
    let hi = median + 3;
    for v in members {
        let b = (v.round() as i32 - lo).clamp(0, 6) as usize;
        counts[b] = counts[b].saturating_add(1);
    }
    (counts, lo, hi)
}

/// Render a 7-cell unicode bar histogram from bin counts.
pub fn histogram_bars(counts: &[u8; 7]) -> String {
    const BARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = *counts.iter().max().unwrap_or(&0).max(&1) as f64;
    counts
        .iter()
        .map(|&c| {
            let idx = ((c as f64 / max) * 8.0).round().clamp(0.0, 8.0) as usize;
            BARS[idx]
        })
        .collect()
}

// ── Window check ─────────────────────────────────────────────────────────────

/// Returns Some(lead_days) when `resolution_date` falls within D+0..D+2 of
/// today (UTC, which is close enough to airport-local for window scoping —
/// the actual forecast bucketing uses Open-Meteo's `timezone=auto`).
pub fn lead_days_in_window(resolution_date: NaiveDate, today: NaiveDate) -> Option<u8> {
    let delta = (resolution_date - today).num_days();
    (0..=2).contains(&delta).then_some(delta as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_basic() {
        let s = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        assert!((percentile(&s, 0.10) - 0.9).abs() < 1e-9);
        assert!((percentile(&s, 0.50) - 4.5).abs() < 1e-9);
        assert!((percentile(&s, 0.90) - 8.1).abs() < 1e-9);
    }

    #[test]
    fn summarize_handles_unsorted_input() {
        let d = summarize(vec![3.0, 1.0, 2.0, 5.0, 4.0]);
        assert!((d.mean - 3.0).abs() < 1e-9);
        assert_eq!(d.members, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn histogram_buckets_around_median() {
        // 51 members centered at 16.0
        let members: Vec<f64> = (0..51).map(|i| 16.0 + (i as f64 - 25.0) * 0.1).collect();
        let (counts, lo, hi) = histogram(&members);
        assert_eq!(counts.iter().map(|&c| c as u32).sum::<u32>(), 51);
        assert_eq!(lo, 13);
        assert_eq!(hi, 19);
    }

    #[test]
    fn window_check() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        assert_eq!(lead_days_in_window(today, today), Some(0));
        assert_eq!(
            lead_days_in_window(today.succ_opt().unwrap(), today),
            Some(1)
        );
        assert_eq!(
            lead_days_in_window(today + chrono::Duration::days(2), today),
            Some(2)
        );
        assert_eq!(
            lead_days_in_window(today + chrono::Duration::days(3), today),
            None
        );
        assert_eq!(
            lead_days_in_window(today - chrono::Duration::days(1), today),
            None
        );
    }

    #[test]
    fn aggregate_ensemble_picks_correct_day_per_member() {
        // 2 members × 48 hourly values across two days. Member 1 has its high
        // on day 1 at idx 14; member 2 has its high on day 1 at idx 12.
        let times: Vec<String> = (0..48)
            .map(|i| {
                let d = if i < 24 { 1 } else { 2 };
                format!("2026-05-0{}T{:02}:00", d, i % 24)
            })
            .collect();
        let mut series = HashMap::new();
        let mut m1 = vec![Some(10.0); 48];
        m1[14] = Some(20.0); // peak on target day
        m1[5] = Some(5.0);
        m1[30] = Some(99.0); // ignored — wrong day
        series.insert("temperature_2m".to_string(), m1);
        let mut m2 = vec![Some(11.0); 48];
        m2[12] = Some(21.0);
        m2[3] = Some(6.0);
        series.insert("temperature_2m_member01".to_string(), m2);

        let hourly = EnsembleHourly {
            time: times,
            series,
        };
        let (highs, lows) =
            aggregate_ensemble(&hourly, NaiveDate::from_ymd_opt(2026, 5, 1).unwrap());
        assert_eq!(highs.len(), 2);
        assert_eq!(lows.len(), 2);
        let mut h = highs.clone();
        h.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(h, vec![20.0, 21.0]);
        let mut l = lows.clone();
        l.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(l, vec![5.0, 6.0]);
    }
}
