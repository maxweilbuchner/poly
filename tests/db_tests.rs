use poly::db::{self, ResolutionRow, SnapshotRow};
use std::path::PathBuf;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Create a fresh in-memory-style temp DB and return (Connection, path).
fn temp_db() -> (rusqlite::Connection, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.keep().join("test.db");
    let conn = db::open(&path).unwrap();
    (conn, path)
}

fn snap(ts: &str, cid: &str, outcome: &str, price: f64) -> SnapshotRow {
    SnapshotRow {
        snapshot_at: ts.to_string(),
        condition_id: cid.to_string(),
        question: format!("Q for {}", cid),
        slug: format!("slug-{}", cid),
        category: "test".to_string(),
        status: "Active".to_string(),
        end_date: "2030-01-01T00:00:00Z".to_string(),
        volume: 10000.0,
        liquidity: 5000.0,
        outcome: outcome.to_string(),
        price,
    }
}

fn resolution(cid: &str, res: &str) -> ResolutionRow {
    ResolutionRow {
        condition_id: cid.to_string(),
        question: format!("Q for {}", cid),
        slug: format!("slug-{}", cid),
        end_date: "2030-01-01T00:00:00Z".to_string(),
        resolution: res.to_string(),
        last_trade_price: None,
        clob_token_id: None,
        group_slug: String::new(),
    }
}

// ── open ─────────────────────────────────────────────────────────────────────

#[test]
fn open_creates_db_and_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.db");
    assert!(!path.exists());

    let conn = db::open(&path).unwrap();
    assert!(path.exists());

    // Tables should exist
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('snapshots','resolutions')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn open_is_idempotent() {
    let (_, path) = temp_db();

    // Open a second time — should not error
    let conn2 = db::open(&path).unwrap();
    let count: i64 = conn2
        .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

// ── insert_snapshots ────────────────────────────────────────────────────────

#[test]
fn insert_snapshots_basic() {
    let (mut conn, _) = temp_db();

    let rows = vec![
        snap("2025-01-01T00:00:00Z", "0xabc", "Yes", 0.75),
        snap("2025-01-01T00:00:00Z", "0xabc", "No", 0.25),
    ];
    db::insert_snapshots(&mut conn, &rows).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn insert_snapshots_ignores_duplicates() {
    let (mut conn, _) = temp_db();

    let row = snap("2025-01-01T00:00:00Z", "0xabc", "Yes", 0.75);
    db::insert_snapshots(&mut conn, &[row]).unwrap();

    // Insert same PK again
    let row2 = snap("2025-01-01T00:00:00Z", "0xabc", "Yes", 0.80);
    db::insert_snapshots(&mut conn, &[row2]).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);

    // Price should remain 0.75 (first insert wins with INSERT OR IGNORE)
    let price: f64 = conn
        .query_row("SELECT price FROM snapshots", [], |r| r.get(0))
        .unwrap();
    assert!((price - 0.75).abs() < 0.001);
}

#[test]
fn insert_snapshots_empty_is_noop() {
    let (mut conn, _) = temp_db();
    db::insert_snapshots(&mut conn, &[]).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

// ── insert_resolutions ──────────────────────────────────────────────────────

#[test]
fn insert_resolutions_returns_new_count() {
    let (mut conn, _) = temp_db();

    let rows = vec![resolution("0x111", "Yes"), resolution("0x222", "No")];
    let count = db::insert_resolutions(&mut conn, &rows).unwrap();
    assert_eq!(count, 2);
}

#[test]
fn insert_resolutions_ignores_duplicates() {
    let (mut conn, _) = temp_db();

    let rows = vec![resolution("0x111", "Yes")];
    let c1 = db::insert_resolutions(&mut conn, &rows).unwrap();
    assert_eq!(c1, 1);

    // Same condition_id again
    let rows2 = vec![resolution("0x111", "No")];
    let c2 = db::insert_resolutions(&mut conn, &rows2).unwrap();
    assert_eq!(c2, 0); // duplicate ignored

    // Resolution should remain "Yes" (first insert wins)
    let res: String = conn
        .query_row(
            "SELECT resolution FROM resolutions WHERE condition_id = '0x111'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(res, "Yes");
}

#[test]
fn insert_resolutions_backfills_nullable_columns() {
    let (mut conn, _) = temp_db();

    // First insert without optional fields
    let rows = vec![resolution("0x111", "Yes")];
    db::insert_resolutions(&mut conn, &rows).unwrap();

    // Second insert with optional fields — should backfill
    let rows2 = vec![ResolutionRow {
        condition_id: "0x111".to_string(),
        question: "Q".to_string(),
        slug: "s".to_string(),
        end_date: "2030-01-01T00:00:00Z".to_string(),
        resolution: "Yes".to_string(),
        last_trade_price: Some(0.95),
        clob_token_id: Some("tok123".to_string()),
        group_slug: String::new(),
    }];
    db::insert_resolutions(&mut conn, &rows2).unwrap();

    let (price, token): (Option<f64>, Option<String>) = conn
        .query_row(
            "SELECT last_trade_price, clob_token_id FROM resolutions WHERE condition_id = '0x111'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!((price.unwrap() - 0.95).abs() < 0.001);
    assert_eq!(token.unwrap(), "tok123");
}

#[test]
fn insert_resolutions_empty_is_noop() {
    let (mut conn, _) = temp_db();
    let count = db::insert_resolutions(&mut conn, &[]).unwrap();
    assert_eq!(count, 0);
}

// ── load_resolved_ids ───────────────────────────────────────────────────────

#[test]
fn load_resolved_ids_returns_all_condition_ids() {
    let (mut conn, _) = temp_db();
    let rows = vec![resolution("0xaaa", "Yes"), resolution("0xbbb", "No")];
    db::insert_resolutions(&mut conn, &rows).unwrap();

    let ids = db::load_resolved_ids(&conn).unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains("0xaaa"));
    assert!(ids.contains("0xbbb"));
}

#[test]
fn load_resolved_ids_empty_db() {
    let (conn, _) = temp_db();
    let ids = db::load_resolved_ids(&conn).unwrap();
    assert!(ids.is_empty());
}

// ── query_latest_snapshot ───────────────────────────────────────────────────

#[test]
fn query_latest_snapshot_returns_most_recent() {
    let (mut conn, _) = temp_db();

    let rows = vec![
        // Older snapshot
        snap("2025-01-01T00:00:00Z", "0xabc", "Yes", 0.60),
        snap("2025-01-01T00:00:00Z", "0xabc", "No", 0.40),
        // Newer snapshot
        snap("2025-01-02T00:00:00Z", "0xabc", "Yes", 0.75),
        snap("2025-01-02T00:00:00Z", "0xabc", "No", 0.25),
    ];
    db::insert_snapshots(&mut conn, &rows).unwrap();

    let latest = db::query_latest_snapshot(&conn).unwrap();
    assert_eq!(latest.len(), 1);

    let (_, _, _, yes_price) = &latest[0];
    assert!((yes_price.unwrap() - 0.75).abs() < 0.01);
}

#[test]
fn query_latest_snapshot_empty_db() {
    let (conn, _) = temp_db();
    let latest = db::query_latest_snapshot(&conn).unwrap();
    assert!(latest.is_empty());
}

// ── query_resolution_counts ─────────────────────────────────────────────────

#[test]
fn resolution_counts_groups_correctly() {
    let (mut conn, _) = temp_db();
    let rows = vec![
        resolution("0x1", "Yes"),
        resolution("0x2", "Yes"),
        resolution("0x3", "No"),
        resolution("0x4", "Early Resolution"),
    ];
    db::insert_resolutions(&mut conn, &rows).unwrap();

    let (yes, no, other) = db::query_resolution_counts(&conn).unwrap();
    assert_eq!(yes, 2);
    assert_eq!(no, 1);
    assert_eq!(other, 1);
}

#[test]
fn resolution_counts_empty_db() {
    let (conn, _) = temp_db();
    let (yes, no, other) = db::query_resolution_counts(&conn).unwrap();
    assert_eq!((yes, no, other), (0, 0, 0));
}

// ── query_high_confidence_accuracy ──────────────────────────────────────────

#[test]
fn high_confidence_correct_prediction() {
    let (mut conn, _) = temp_db();

    // Market resolved Yes, snapshot showed 0.90 Yes (>0.80) within 6 hours of close
    let end = "2025-06-01T12:00:00Z";
    let snap_time = "2025-06-01T08:00:00Z"; // 4 hours before close

    let mut s = snap(snap_time, "0xhigh", "Yes", 0.90);
    s.end_date = end.to_string();
    db::insert_snapshots(&mut conn, &[s]).unwrap();

    let mut r = resolution("0xhigh", "Yes");
    r.end_date = end.to_string();
    db::insert_resolutions(&mut conn, &[r]).unwrap();

    let (correct, wrong) = db::query_high_confidence_accuracy(&conn).unwrap();
    assert_eq!(correct, 1);
    assert_eq!(wrong, 0);
}

#[test]
fn high_confidence_wrong_prediction() {
    let (mut conn, _) = temp_db();

    // Market resolved No, but snapshot showed 0.85 Yes (>0.80) — wrong prediction
    let end = "2025-06-01T12:00:00Z";
    let snap_time = "2025-06-01T09:00:00Z";

    let mut s = snap(snap_time, "0xwrong", "Yes", 0.85);
    s.end_date = end.to_string();
    db::insert_snapshots(&mut conn, &[s]).unwrap();

    let mut r = resolution("0xwrong", "No");
    r.end_date = end.to_string();
    db::insert_resolutions(&mut conn, &[r]).unwrap();

    let (correct, wrong) = db::query_high_confidence_accuracy(&conn).unwrap();
    assert_eq!(correct, 0);
    assert_eq!(wrong, 1);
}

#[test]
fn high_confidence_skips_uncertain_markets() {
    let (mut conn, _) = temp_db();

    // Snapshot shows 0.55 Yes — not high confidence (< 0.80), should be skipped
    let end = "2025-06-01T12:00:00Z";
    let snap_time = "2025-06-01T09:00:00Z";

    let mut s = snap(snap_time, "0xmid", "Yes", 0.55);
    s.end_date = end.to_string();
    db::insert_snapshots(&mut conn, &[s]).unwrap();

    let mut r = resolution("0xmid", "Yes");
    r.end_date = end.to_string();
    db::insert_resolutions(&mut conn, &[r]).unwrap();

    let (correct, wrong) = db::query_high_confidence_accuracy(&conn).unwrap();
    assert_eq!(correct, 0);
    assert_eq!(wrong, 0);
}

// ── calibration price helpers ───────────────────────────────────────────────

#[test]
fn update_and_query_calibration_price() {
    let (mut conn, _) = temp_db();

    let rows = vec![ResolutionRow {
        condition_id: "0xcal".to_string(),
        question: "Cal test".to_string(),
        slug: "cal-test".to_string(),
        end_date: "2025-06-01T12:00:00Z".to_string(),
        resolution: "Yes".to_string(),
        last_trade_price: None,
        clob_token_id: Some("tok999".to_string()),
        group_slug: String::new(),
    }];
    db::insert_resolutions(&mut conn, &rows).unwrap();

    // Initially unpriced
    let unpriced = db::query_unpriced_resolutions(&conn, 3, 10).unwrap();
    assert_eq!(unpriced.len(), 1);
    assert_eq!(unpriced[0].0, "0xcal");

    // Store a calibration price
    db::update_calibration_price(&conn, "0xcal", 0.72, 3).unwrap();

    // Now priced — should not appear in unpriced query for same horizon
    let unpriced2 = db::query_unpriced_resolutions(&conn, 3, 10).unwrap();
    assert!(unpriced2.is_empty());

    // But should appear for a different horizon
    let unpriced3 = db::query_unpriced_resolutions(&conn, 6, 10).unwrap();
    assert_eq!(unpriced3.len(), 1);
}

// ── query_calibration ───────────────────────────────────────────────────────

#[test]
fn calibration_buckets_with_stored_prices() {
    let (mut conn, _) = temp_db();

    // Two markets with calibration prices at 3h
    let rows = vec![
        ResolutionRow {
            condition_id: "0xc1".to_string(),
            question: "C1".to_string(),
            slug: "c1".to_string(),
            end_date: "2025-06-01T12:00:00Z".to_string(),
            resolution: "Yes".to_string(),
            last_trade_price: None,
            clob_token_id: Some("t1".to_string()),
            group_slug: String::new(),
        },
        ResolutionRow {
            condition_id: "0xc2".to_string(),
            question: "C2".to_string(),
            slug: "c2".to_string(),
            end_date: "2025-06-01T12:00:00Z".to_string(),
            resolution: "No".to_string(),
            last_trade_price: None,
            clob_token_id: Some("t2".to_string()),
            group_slug: String::new(),
        },
    ];
    db::insert_resolutions(&mut conn, &rows).unwrap();

    // 0xc1: Yes at price 0.85 → bucket 8 (80-90%), resolved Yes → correct
    db::update_calibration_price(&conn, "0xc1", 0.85, 3).unwrap();
    // 0xc2: Yes at price 0.25 → bucket 2 (20-30%), resolved No → correct
    db::update_calibration_price(&conn, "0xc2", 0.25, 3).unwrap();

    let buckets = db::query_calibration(&conn, 3).unwrap();

    // Bucket 2 (20-30%): 0 yes out of 1 total
    assert_eq!(buckets[2], (0, 1));
    // Bucket 8 (80-90%): 1 yes out of 1 total
    assert_eq!(buckets[8], (1, 1));
    // Other buckets empty
    assert_eq!(buckets[0], (0, 0));
    assert_eq!(buckets[5], (0, 0));
}

// ── query_edge_vs_volume ────────────────────────────────────────────────────

#[test]
fn edge_vs_volume_tiers() {
    let (mut conn, _) = temp_db();

    // Market with snapshots (for peak volume) and a calibration price
    let snaps = vec![snap("2025-01-01T00:00:00Z", "0xvol", "Yes", 0.70)];
    db::insert_snapshots(&mut conn, &snaps).unwrap();

    let rows = vec![ResolutionRow {
        condition_id: "0xvol".to_string(),
        question: "Vol test".to_string(),
        slug: "vol-test".to_string(),
        end_date: "2025-06-01T12:00:00Z".to_string(),
        resolution: "Yes".to_string(),
        last_trade_price: None,
        clob_token_id: Some("t1".to_string()),
        group_slug: String::new(),
    }];
    db::insert_resolutions(&mut conn, &rows).unwrap();
    db::update_calibration_price(&conn, "0xvol", 0.70, 3).unwrap();

    let tiers = db::query_edge_vs_volume(&conn).unwrap();
    assert_eq!(tiers.len(), 5);

    // Volume is 10000 → tier 2 ($10K–$100K) since CASE uses < 10000 for tier 1
    assert_eq!(tiers[2].2, 1); // count = 1
                               // MAE = |0.70 - 1.0| = 0.30
    assert!((tiers[2].1 - 0.30).abs() < 0.01);
}

// ── query_unresolved_closed ─────────────────────────────────────────────────

#[test]
fn unresolved_closed_finds_candidates() {
    let (mut conn, _) = temp_db();

    // Closed market without a resolution
    let mut s = snap("2025-01-01T00:00:00Z", "0xclosed", "Yes", 0.90);
    s.status = "Closed".to_string();
    db::insert_snapshots(&mut conn, &[s]).unwrap();

    let candidates = db::query_unresolved_closed(&conn).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].0, "0xclosed");
}

#[test]
fn unresolved_closed_excludes_resolved() {
    let (mut conn, _) = temp_db();

    // Closed market that IS resolved
    let mut s = snap("2025-01-01T00:00:00Z", "0xresolved", "Yes", 0.95);
    s.status = "Closed".to_string();
    db::insert_snapshots(&mut conn, &[s]).unwrap();
    db::insert_resolutions(&mut conn, &[resolution("0xresolved", "Yes")]).unwrap();

    let candidates = db::query_unresolved_closed(&conn).unwrap();
    assert!(candidates.is_empty());
}

#[test]
fn unresolved_closed_excludes_active() {
    let (mut conn, _) = temp_db();

    // Active market (not closed) without a resolution
    let s = snap("2025-01-01T00:00:00Z", "0xactive", "Yes", 0.60);
    db::insert_snapshots(&mut conn, &[s]).unwrap();

    let candidates = db::query_unresolved_closed(&conn).unwrap();
    assert!(candidates.is_empty());
}

// ── migrate_from_csvs ───────────────────────────────────────────────────────

#[test]
fn migrate_imports_csv_data() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate.db");

    let snap_csv = dir.path().join("snapshots.csv");
    std::fs::write(
        &snap_csv,
        "snapshot_at,condition_id,question,slug,category,status,end_date,volume,liquidity,outcome,price\n\
         2025-01-01T00:00:00Z,0xm1,Question One,slug-1,crypto,Active,2030-01-01T00:00:00Z,5000,1000,Yes,0.65\n\
         2025-01-01T00:00:00Z,0xm1,Question One,slug-1,crypto,Active,2030-01-01T00:00:00Z,5000,1000,No,0.35\n",
    )
    .unwrap();

    let res_csv = dir.path().join("resolutions.csv");
    std::fs::write(
        &res_csv,
        "condition_id,question,slug,end_date,resolution\n\
         0xr1,Resolved Q,resolved-q,2025-01-01T00:00:00Z,Yes\n",
    )
    .unwrap();

    let (snap_n, res_n) = db::migrate_from_csvs(&db_path, &snap_csv, &res_csv);
    assert_eq!(snap_n, 2);
    assert_eq!(res_n, 1);
}

#[test]
fn migrate_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate2.db");

    let snap_csv = dir.path().join("snapshots.csv");
    std::fs::write(
        &snap_csv,
        "snapshot_at,condition_id,question,slug,category,status,end_date,volume,liquidity,outcome,price\n\
         2025-01-01T00:00:00Z,0xm1,Q,s,c,Active,2030-01-01T00:00:00Z,5000,1000,Yes,0.65\n",
    )
    .unwrap();

    let res_csv = dir.path().join("resolutions.csv");
    std::fs::write(&res_csv, "condition_id,question,slug,end_date,resolution\n").unwrap();

    let (s1, _) = db::migrate_from_csvs(&db_path, &snap_csv, &res_csv);
    assert_eq!(s1, 1);

    // Second call should no-op (snapshots table already has data)
    let (s2, r2) = db::migrate_from_csvs(&db_path, &snap_csv, &res_csv);
    assert_eq!(s2, 0);
    assert_eq!(r2, 0);
}

#[test]
fn migrate_handles_missing_csvs() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate3.db");
    let snap_csv = dir.path().join("nonexistent_snapshots.csv");
    let res_csv = dir.path().join("nonexistent_resolutions.csv");

    let (s, r) = db::migrate_from_csvs(&db_path, &snap_csv, &res_csv);
    assert_eq!((s, r), (0, 0));
}

#[test]
fn migrate_handles_quoted_csv_fields() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate4.db");

    let res_csv = dir.path().join("resolutions.csv");
    std::fs::write(
        &res_csv,
        "condition_id,question,slug,end_date,resolution\n\
         0xq1,\"Will it rain, tomorrow?\",rain-tomorrow,2025-01-01T00:00:00Z,Yes\n",
    )
    .unwrap();

    let snap_csv = dir.path().join("snapshots.csv");
    // No snapshots file

    let (_, r) = db::migrate_from_csvs(&db_path, &snap_csv, &res_csv);
    assert_eq!(r, 1);

    // Verify the comma was handled correctly
    let conn = db::open(&db_path).unwrap();
    let q: String = conn
        .query_row(
            "SELECT question FROM resolutions WHERE condition_id = '0xq1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(q, "Will it rain, tomorrow?");
}

// ── query_recurring_accuracy ────────────────────────────────────────────────

/// Helper that produces a resolution row already tagged with a `group_slug`.
fn resolution_in_group(cid: &str, res: &str, group_slug: &str) -> ResolutionRow {
    ResolutionRow {
        condition_id: cid.to_string(),
        question: format!("Q {}", cid),
        slug: format!("slug-{}", cid),
        end_date: "2025-06-01T12:00:00Z".to_string(),
        resolution: res.to_string(),
        last_trade_price: None,
        clob_token_id: Some(format!("tok-{}", cid)),
        group_slug: group_slug.to_string(),
    }
}

#[test]
fn recurring_accuracy_orders_groups_by_correctness_then_size() {
    let (mut conn, _) = temp_db();

    // Group A: 5/5 correct (perfect series)
    //   - 3 markets resolved YES with calibration price > 0.5
    //   - 2 markets resolved NO  with calibration price < 0.5
    // Group B: 4/5 correct (one wrong call)
    // Group C: only 4 markets total → filtered out by min_markets=5
    let rows = vec![
        // Group A
        resolution_in_group("0xa1", "Yes", "group-a"),
        resolution_in_group("0xa2", "Yes", "group-a"),
        resolution_in_group("0xa3", "Yes", "group-a"),
        resolution_in_group("0xa4", "No", "group-a"),
        resolution_in_group("0xa5", "No", "group-a"),
        // Group B
        resolution_in_group("0xb1", "Yes", "group-b"),
        resolution_in_group("0xb2", "Yes", "group-b"),
        resolution_in_group("0xb3", "No", "group-b"),
        resolution_in_group("0xb4", "No", "group-b"),
        resolution_in_group("0xb5", "Yes", "group-b"), // wrong call below
        // Group C — should be filtered out
        resolution_in_group("0xc1", "Yes", "group-c"),
        resolution_in_group("0xc2", "Yes", "group-c"),
        resolution_in_group("0xc3", "No", "group-c"),
        resolution_in_group("0xc4", "No", "group-c"),
    ];
    db::insert_resolutions(&mut conn, &rows).unwrap();

    // Group A: all five predictions correct.
    db::update_calibration_price(&conn, "0xa1", 0.85, 3).unwrap();
    db::update_calibration_price(&conn, "0xa2", 0.72, 3).unwrap();
    db::update_calibration_price(&conn, "0xa3", 0.55, 3).unwrap();
    db::update_calibration_price(&conn, "0xa4", 0.20, 3).unwrap();
    db::update_calibration_price(&conn, "0xa5", 0.30, 3).unwrap();

    // Group B: four correct, one wrong (0xb5: predicted NO at 0.20 but resolved YES).
    db::update_calibration_price(&conn, "0xb1", 0.85, 3).unwrap();
    db::update_calibration_price(&conn, "0xb2", 0.65, 3).unwrap();
    db::update_calibration_price(&conn, "0xb3", 0.20, 3).unwrap();
    db::update_calibration_price(&conn, "0xb4", 0.30, 3).unwrap();
    db::update_calibration_price(&conn, "0xb5", 0.20, 3).unwrap(); // wrong

    // Group C: doesn't matter, won't appear (only 4 markets).
    db::update_calibration_price(&conn, "0xc1", 0.90, 3).unwrap();
    db::update_calibration_price(&conn, "0xc2", 0.90, 3).unwrap();
    db::update_calibration_price(&conn, "0xc3", 0.10, 3).unwrap();
    db::update_calibration_price(&conn, "0xc4", 0.10, 3).unwrap();

    let result = db::query_recurring_accuracy(&conn, 3, 5, 10).unwrap();

    assert_eq!(result.len(), 2, "group-c filtered by min_markets=5");
    assert_eq!(result[0], ("group-a".to_string(), 5, 5));
    assert_eq!(result[1], ("group-b".to_string(), 5, 4));
}

#[test]
fn recurring_accuracy_excludes_empty_group_slug() {
    let (mut conn, _) = temp_db();

    // Five resolved markets with empty group_slug → never returned.
    let mut rows: Vec<ResolutionRow> = (0..5)
        .map(|i| resolution(&format!("0xn{}", i), "Yes"))
        .collect();
    // Plus five in a real group, all correct.
    rows.extend((0..5).map(|i| resolution_in_group(&format!("0xg{}", i), "Yes", "real-group")));
    db::insert_resolutions(&mut conn, &rows).unwrap();

    for i in 0..5 {
        db::update_calibration_price(&conn, &format!("0xn{}", i), 0.80, 3).unwrap();
        db::update_calibration_price(&conn, &format!("0xg{}", i), 0.80, 3).unwrap();
    }

    let result = db::query_recurring_accuracy(&conn, 3, 5, 10).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], ("real-group".to_string(), 5, 5));
}

#[test]
fn recurring_accuracy_uses_snapshot_fallback_when_no_calibration_price() {
    let (mut conn, _) = temp_db();

    // Five markets, all in same group, end_date in 2025-06-01, with snapshots
    // taken 4 hours before close (i.e. -4h ≤ -3h, so they qualify as a 3h-fallback).
    let cids: Vec<String> = (0..5).map(|i| format!("0xs{}", i)).collect();
    let rows: Vec<_> = cids
        .iter()
        .enumerate()
        .map(|(i, cid)| {
            let mut r =
                resolution_in_group(cid, if i % 2 == 0 { "Yes" } else { "No" }, "snapshot-group");
            // Resolutions from the helper carry no calibration_price — fallback path will hit.
            r.clob_token_id = None;
            r
        })
        .collect();
    db::insert_resolutions(&mut conn, &rows).unwrap();

    // Snapshot 4h before close (end_date = 2025-06-01T12:00:00Z → snapshot at 08:00).
    // Even-indexed markets resolve YES with yes_price 0.80 (correct).
    // Odd-indexed  markets resolve NO  with yes_price 0.20 (correct).
    let snaps: Vec<_> = cids
        .iter()
        .enumerate()
        .map(|(i, cid)| {
            let price = if i % 2 == 0 { 0.80 } else { 0.20 };
            SnapshotRow {
                snapshot_at: "2025-06-01T08:00:00Z".to_string(),
                condition_id: cid.clone(),
                question: format!("Q {}", cid),
                slug: format!("slug-{}", cid),
                category: "test".to_string(),
                status: "Closed".to_string(),
                end_date: "2025-06-01T12:00:00Z".to_string(),
                volume: 10000.0,
                liquidity: 5000.0,
                outcome: "Yes".to_string(),
                price,
            }
        })
        .collect();
    db::insert_snapshots(&mut conn, &snaps).unwrap();

    let result = db::query_recurring_accuracy(&conn, 3, 5, 10).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], ("snapshot-group".to_string(), 5, 5));
}
