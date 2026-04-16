use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::tui::{DateFilter, ProbFilter, SortMode, VolumeFilter};
use crate::types::Market;

// ── Data directory ────────────────────────────────────────────────────────────

/// `$XDG_DATA_HOME/poly`, falling back to `~/.local/share/poly`.
fn data_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .map(|h| h.join(".local").join("share"))
                .unwrap_or_else(|| PathBuf::from("."))
        });
    base.join("poly")
}

// ── Schema versions ───────────────────────────────────────────────────────────

const UI_STATE_VERSION: u32 = 1;
const SNAPSHOT_META_VERSION: u32 = 1;
const WATCHLIST_VERSION: u32 = 1;

// ── UiState ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiState {
    pub sort_mode: SortMode,
    pub date_filter: DateFilter,
    pub prob_filter: ProbFilter,
    pub volume_filter: VolumeFilter,
    pub category_filter: Option<String>,
}

/// `$XDG_DATA_HOME/poly/ui_state.json`, falling back to `~/.local/share/poly/ui_state.json`.
fn state_path() -> Option<PathBuf> {
    Some(data_dir().join("ui_state.json"))
}

pub fn load_ui_state() -> UiState {
    let path = match state_path() {
        Some(p) => p,
        None => return UiState::default(),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return UiState::default(),
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return UiState::default(),
    };
    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if version > UI_STATE_VERSION {
        // Written by a newer binary — fall back to defaults rather than corrupt app state.
        return UiState::default();
    }
    // v0 and v1 have identical field schemas; just deserialize.
    serde_json::from_value(value).unwrap_or_default()
}

pub fn save_ui_state(state: &UiState) {
    let path = match state_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    save_versioned(state, UI_STATE_VERSION, &path);
}

// ── Snapshot metadata (persisted across sessions) ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotMeta {
    /// ISO 8601 timestamp of the last completed snapshot.
    pub last_snapshot_at: Option<String>,
    /// Number of markets written in the last snapshot.
    pub last_snapshot_count: usize,
}

fn snapshot_meta_path() -> PathBuf {
    data_dir().join("snapshot_meta.json")
}

pub fn load_snapshot_meta() -> SnapshotMeta {
    let path = snapshot_meta_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return SnapshotMeta::default(),
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return SnapshotMeta::default(),
    };
    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if version > SNAPSHOT_META_VERSION {
        return SnapshotMeta::default();
    }
    serde_json::from_value(value).unwrap_or_default()
}

pub fn save_snapshot_meta(meta: &SnapshotMeta) {
    let path = snapshot_meta_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    save_versioned(meta, SNAPSHOT_META_VERSION, &path);
}

// ── Watchlist ─────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct WatchlistFile {
    version: u32,
    items: Vec<String>,
}

fn watchlist_path() -> Option<PathBuf> {
    Some(data_dir().join("watchlist.json"))
}

pub fn load_watchlist() -> HashSet<String> {
    let path = match watchlist_path() {
        Some(p) => p,
        None => return HashSet::new(),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return HashSet::new(),
    };
    // Try versioned object format (v1+).
    if let Ok(wf) = serde_json::from_str::<WatchlistFile>(&text) {
        return wf.items.into_iter().collect();
    }
    // Fall back to v0: raw JSON array written by older binaries.
    if let Ok(v) = serde_json::from_str::<Vec<String>>(&text) {
        return v.into_iter().collect();
    }
    HashSet::new()
}

pub fn save_watchlist(watchlist: &HashSet<String>) {
    let path = match watchlist_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut sorted: Vec<String> = watchlist.iter().cloned().collect();
    sorted.sort();
    let wf = WatchlistFile {
        version: WATCHLIST_VERSION,
        items: sorted,
    };
    if let Ok(json) = serde_json::to_string(&wf) {
        let _ = std::fs::write(&path, json);
    }
}

// ── CSV / data paths ──────────────────────────────────────────────────────────

pub fn resolutions_csv_path() -> PathBuf {
    data_dir().join("market_resolutions.csv")
}

/// Path to the SQLite database: `$XDG_DATA_HOME/poly/poly.db`.
pub fn db_path() -> PathBuf {
    data_dir().join("poly.db")
}

pub fn snapshot_csv_path() -> PathBuf {
    data_dir().join("market_snapshots.csv")
}

// ── Watchlist export ──────────────────────────────────────────────────────────

/// Write all starred markets to a timestamped JSON file in the poly data directory.
/// Returns the path written on success.
pub fn export_watchlist(
    watchlist: &HashSet<String>,
    markets: &[Market],
) -> Result<PathBuf, std::io::Error> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;

    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let path = dir.join(format!("watchlist_export_{}.json", ts));

    let starred: Vec<&Market> = markets
        .iter()
        .filter(|m| watchlist.contains(&m.condition_id))
        .collect();

    let json = serde_json::to_string_pretty(&starred).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    Ok(path)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Serialize `value` as JSON, inject `"version"` at the top level, then write to `path`.
fn save_versioned<T: Serialize>(value: &T, version: u32, path: &std::path::Path) {
    let mut json_val = match serde_json::to_value(value) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let serde_json::Value::Object(ref mut map) = json_val {
        map.insert("version".to_string(), serde_json::json!(version));
    }
    if let Ok(text) = serde_json::to_string(&json_val) {
        let _ = std::fs::write(path, text);
    }
}
