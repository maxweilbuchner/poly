//! Round-trip serialization tests for persist.rs.
//!
//! Each test writes to a temp directory (overriding XDG_DATA_HOME) and verifies
//! that load returns exactly what was saved. Also tests backward-compatible
//! parsing and version-guard behavior.
//!
//! Tests are serialized via a Mutex because XDG_DATA_HOME is process-global.

use std::collections::HashSet;
use std::sync::Mutex;

use poly::persist::{
    load_snapshot_meta, load_ui_state, load_watchlist, save_snapshot_meta, save_ui_state,
    save_watchlist, SnapshotMeta, UiState,
};
use poly::tui::{DateFilter, ProbFilter, SortMode, VolumeFilter};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Run `f` with `XDG_DATA_HOME` pointed at a fresh temp directory.
/// Serialized via ENV_LOCK to prevent parallel env-var interference.
fn with_temp_data_dir<F: FnOnce()>(f: F) {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let prev = std::env::var("XDG_DATA_HOME").ok();
    unsafe { std::env::set_var("XDG_DATA_HOME", dir.path()) };
    f();
    match prev {
        Some(v) => unsafe { std::env::set_var("XDG_DATA_HOME", v) },
        None => unsafe { std::env::remove_var("XDG_DATA_HOME") },
    }
}

// ── UiState ──────────────────────────────────────────────────────────────────

#[test]
fn ui_state_round_trip() {
    with_temp_data_dir(|| {
        let state = UiState {
            sort_mode: SortMode::EndDate,
            date_filter: DateFilter::Month,
            prob_filter: ProbFilter::Prob85_98,
            volume_filter: VolumeFilter::K100,
            category_filter: Some("crypto".to_string()),
        };
        save_ui_state(&state);
        let loaded = load_ui_state();

        assert_eq!(
            format!("{:?}", state.sort_mode),
            format!("{:?}", loaded.sort_mode)
        );
        assert_eq!(
            format!("{:?}", state.date_filter),
            format!("{:?}", loaded.date_filter)
        );
        assert_eq!(
            format!("{:?}", state.prob_filter),
            format!("{:?}", loaded.prob_filter)
        );
        assert_eq!(
            format!("{:?}", state.volume_filter),
            format!("{:?}", loaded.volume_filter)
        );
        assert_eq!(loaded.category_filter, Some("crypto".to_string()));
    });
}

#[test]
fn ui_state_defaults_on_missing_file() {
    with_temp_data_dir(|| {
        let loaded = load_ui_state();
        assert_eq!(
            format!("{:?}", loaded.sort_mode),
            format!("{:?}", SortMode::default())
        );
        assert!(loaded.category_filter.is_none());
    });
}

#[test]
fn ui_state_defaults_on_corrupt_json() {
    with_temp_data_dir(|| {
        let dir =
            std::path::PathBuf::from(std::env::var("XDG_DATA_HOME").unwrap()).join("poly");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ui_state.json"), "not valid json {{{").unwrap();
        let loaded = load_ui_state();
        assert_eq!(
            format!("{:?}", loaded.sort_mode),
            format!("{:?}", SortMode::default())
        );
    });
}

#[test]
fn ui_state_defaults_on_future_version() {
    with_temp_data_dir(|| {
        let dir =
            std::path::PathBuf::from(std::env::var("XDG_DATA_HOME").unwrap()).join("poly");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("ui_state.json"),
            r#"{"version":999,"sort_mode":"EndDate"}"#,
        )
        .unwrap();
        let loaded = load_ui_state();
        // Should fall back to defaults rather than use the future-version data.
        assert_eq!(
            format!("{:?}", loaded.sort_mode),
            format!("{:?}", SortMode::default())
        );
    });
}

// ── SnapshotMeta ─────────────────────────────────────────────────────────────

#[test]
fn snapshot_meta_round_trip() {
    with_temp_data_dir(|| {
        let meta = SnapshotMeta {
            last_snapshot_at: Some("2026-04-18T12:00:00+00:00".to_string()),
            last_snapshot_count: 7133,
        };
        save_snapshot_meta(&meta);
        let loaded = load_snapshot_meta();

        assert_eq!(loaded.last_snapshot_at, meta.last_snapshot_at);
        assert_eq!(loaded.last_snapshot_count, 7133);
    });
}

#[test]
fn snapshot_meta_defaults_on_missing_file() {
    with_temp_data_dir(|| {
        let loaded = load_snapshot_meta();
        assert!(loaded.last_snapshot_at.is_none());
        assert_eq!(loaded.last_snapshot_count, 0);
    });
}

// ── Watchlist ────────────────────────────────────────────────────────────────

#[test]
fn watchlist_round_trip() {
    with_temp_data_dir(|| {
        let mut wl = HashSet::new();
        wl.insert("cond-id-alpha".to_string());
        wl.insert("cond-id-beta".to_string());
        wl.insert("cond-id-gamma".to_string());
        save_watchlist(&wl);
        let loaded = load_watchlist();

        assert_eq!(loaded.len(), 3);
        assert!(loaded.contains("cond-id-alpha"));
        assert!(loaded.contains("cond-id-beta"));
        assert!(loaded.contains("cond-id-gamma"));
    });
}

#[test]
fn watchlist_empty_round_trip() {
    with_temp_data_dir(|| {
        let wl = HashSet::new();
        save_watchlist(&wl);
        let loaded = load_watchlist();
        assert!(loaded.is_empty());
    });
}

#[test]
fn watchlist_v0_compat_reads_raw_array() {
    with_temp_data_dir(|| {
        // Older binaries wrote a bare JSON array without version wrapper.
        let dir =
            std::path::PathBuf::from(std::env::var("XDG_DATA_HOME").unwrap()).join("poly");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("watchlist.json"),
            r#"["old-id-1","old-id-2"]"#,
        )
        .unwrap();
        let loaded = load_watchlist();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.contains("old-id-1"));
        assert!(loaded.contains("old-id-2"));
    });
}

#[test]
fn watchlist_defaults_on_corrupt_json() {
    with_temp_data_dir(|| {
        let dir =
            std::path::PathBuf::from(std::env::var("XDG_DATA_HOME").unwrap()).join("poly");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("watchlist.json"), "garbage").unwrap();
        let loaded = load_watchlist();
        assert!(loaded.is_empty());
    });
}
