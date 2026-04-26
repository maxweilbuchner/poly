//! SQLite-backed storage for market snapshots and resolutions.
//!
//! One database at `$XDG_DATA_HOME/poly/poly.db` holds two tables:
//!
//! ```text
//! snapshots(snapshot_at, condition_id, question, slug, category, status,
//!           end_date, volume, liquidity, outcome, price)
//!           PRIMARY KEY (snapshot_at, condition_id, outcome)
//!
//! resolutions(condition_id, question, slug, end_date, resolution, last_trade_price)
//!             PRIMARY KEY condition_id
//! ```

use std::collections::HashSet;
use std::path::Path;

/// A single row from calibration queries: `(question, slug, peak_volume, yes_price, resolution)`.
pub type CalibrationRow = (String, String, f64, f64, String);

use rusqlite::{params, Connection};

// ── Schema ─────────────────────────────────────────────────────────────────────

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;

CREATE TABLE IF NOT EXISTS snapshots (
    snapshot_at  TEXT NOT NULL,
    condition_id TEXT NOT NULL,
    question     TEXT NOT NULL DEFAULT '',
    slug         TEXT NOT NULL DEFAULT '',
    category     TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT '',
    end_date     TEXT NOT NULL DEFAULT '',
    volume       REAL NOT NULL DEFAULT 0,
    liquidity    REAL NOT NULL DEFAULT 0,
    outcome      TEXT NOT NULL,
    price        REAL NOT NULL DEFAULT 0,
    PRIMARY KEY (snapshot_at, condition_id, outcome)
);
CREATE INDEX IF NOT EXISTS idx_snap_ts  ON snapshots(snapshot_at);
CREATE INDEX IF NOT EXISTS idx_snap_cid ON snapshots(condition_id, snapshot_at);

CREATE TABLE IF NOT EXISTS resolutions (
    condition_id      TEXT NOT NULL PRIMARY KEY,
    question          TEXT NOT NULL DEFAULT '',
    slug              TEXT NOT NULL DEFAULT '',
    end_date          TEXT NOT NULL DEFAULT '',
    resolution        TEXT NOT NULL,
    last_trade_price  REAL,
    clob_token_id     TEXT,
    calibration_price REAL,
    calibration_hours INTEGER
);

CREATE TABLE IF NOT EXISTS net_worth_log (
    logged_at TEXT NOT NULL PRIMARY KEY,
    balance   REAL NOT NULL,
    positions REAL NOT NULL,
    net_worth REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS market_peak_vol (
    condition_id TEXT NOT NULL PRIMARY KEY,
    volume       REAL NOT NULL DEFAULT 0
);
";

// ── Open / initialise ─────────────────────────────────────────────────────────

/// Open (or create) the database at `path`, applying the schema if needed.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    // Migrate existing databases: add new columns if absent.
    // Each statement fails silently when the column already exists (fresh DB).
    let _ = conn.execute(
        "ALTER TABLE resolutions ADD COLUMN last_trade_price REAL",
        [],
    );
    let _ = conn.execute("ALTER TABLE resolutions ADD COLUMN clob_token_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE resolutions ADD COLUMN calibration_price REAL",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE resolutions ADD COLUMN calibration_hours INTEGER",
        [],
    );

    // Backfill market_peak_vol from snapshots once. After this, insert_snapshots
    // keeps it incrementally up-to-date via UPSERT, so the full GROUP BY scan
    // never has to run again.
    let peak_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM market_peak_vol", [], |r| r.get(0))
        .unwrap_or(0);
    if peak_count == 0 {
        let _ = conn.execute(
            "INSERT INTO market_peak_vol (condition_id, volume)
             SELECT condition_id, MAX(volume) FROM snapshots GROUP BY condition_id",
            [],
        );
    }

    Ok(conn)
}

// ── Data types ─────────────────────────────────────────────────────────────────

pub struct SnapshotRow {
    pub snapshot_at: String,
    pub condition_id: String,
    pub question: String,
    pub slug: String,
    pub category: String,
    pub status: String,
    pub end_date: String,
    pub volume: f64,
    pub liquidity: f64,
    pub outcome: String,
    pub price: f64,
}

pub struct ResolutionRow {
    pub condition_id: String,
    pub question: String,
    pub slug: String,
    pub end_date: String,
    pub resolution: String,
    /// Last traded YES-outcome price at market close (Gamma API `lastTradePrice`).
    pub last_trade_price: Option<f64>,
    /// CLOB Yes-token ID; used to fetch `prices-history` for calibration.
    pub clob_token_id: Option<String>,
}

// ── Writes ─────────────────────────────────────────────────────────────────────

/// Insert a batch of snapshot rows in a single transaction. Ignores duplicates.
pub fn insert_snapshots(conn: &mut Connection, rows: &[SnapshotRow]) -> rusqlite::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO snapshots
             (snapshot_at,condition_id,question,slug,category,status,
              end_date,volume,liquidity,outcome,price)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        )?;
        let mut stmt_peak = tx.prepare_cached(
            "INSERT INTO market_peak_vol (condition_id, volume) VALUES (?1, ?2)
             ON CONFLICT(condition_id) DO UPDATE SET volume = MAX(volume, excluded.volume)",
        )?;
        for r in rows {
            stmt.execute(params![
                r.snapshot_at,
                r.condition_id,
                r.question,
                r.slug,
                r.category,
                r.status,
                r.end_date,
                r.volume,
                r.liquidity,
                r.outcome,
                r.price
            ])?;
            stmt_peak.execute(params![r.condition_id, r.volume])?;
        }
    }
    tx.commit()
}

/// Insert resolution rows in a single transaction.
/// Returns the number of rows that were newly inserted (duplicates ignored).
pub fn insert_resolutions(
    conn: &mut Connection,
    rows: &[ResolutionRow],
) -> rusqlite::Result<usize> {
    if rows.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let mut new_count = 0usize;
    {
        let mut stmt_insert = tx.prepare_cached(
            "INSERT OR IGNORE INTO resolutions
             (condition_id,question,slug,end_date,resolution,last_trade_price,clob_token_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
        )?;
        // Backfill nullable columns on rows that pre-date these fields.
        let mut stmt_backfill = tx.prepare_cached(
            "UPDATE resolutions SET
               last_trade_price = COALESCE(last_trade_price, ?1),
               clob_token_id    = COALESCE(clob_token_id,    ?2)
             WHERE condition_id = ?3",
        )?;
        for r in rows {
            new_count += stmt_insert.execute(params![
                r.condition_id,
                r.question,
                r.slug,
                r.end_date,
                r.resolution,
                r.last_trade_price,
                r.clob_token_id
            ])?;
            if r.last_trade_price.is_some() || r.clob_token_id.is_some() {
                let _ = stmt_backfill.execute(params![
                    r.last_trade_price,
                    r.clob_token_id,
                    r.condition_id
                ]);
            }
        }
    }
    tx.commit()?;
    Ok(new_count)
}

// ── Reads ──────────────────────────────────────────────────────────────────────

/// All condition_ids that have a resolution recorded.
pub fn load_resolved_ids(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT condition_id FROM resolutions")?;
    let ids = stmt
        .query_map([], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// `(category, volume, liquidity, yes_price)` — one entry per market.
type LatestRow = (String, f64, f64, Option<f64>);

/// `(condition_id, question, slug, end_date)` — candidates for cross-reference.
type UnresolvedRow = (String, String, String, Option<String>);

/// Per-market data from the most-recent snapshot run.
/// Returns `(category, volume, liquidity, yes_price)` per unique market.
pub fn query_latest_snapshot(conn: &Connection) -> rusqlite::Result<Vec<LatestRow>> {
    let max_snap: Option<String> = conn
        .query_row("SELECT MAX(snapshot_at) FROM snapshots", [], |r| r.get(0))
        .ok()
        .flatten();
    let Some(ts) = max_snap else {
        return Ok(vec![]);
    };

    let mut stmt = conn.prepare(
        "SELECT condition_id, category, volume, liquidity, outcome, price
         FROM snapshots WHERE snapshot_at = ?1",
    )?;

    use std::collections::HashMap;
    struct Md {
        category: String,
        volume: f64,
        liquidity: f64,
        yes_price: Option<f64>,
    }
    let mut map: HashMap<String, Md> = HashMap::new();

    let rows = stmt.query_map(params![ts], |r| {
        Ok((
            r.get::<_, String>(0)?, // condition_id
            r.get::<_, String>(1)?, // category
            r.get::<_, f64>(2)?,    // volume
            r.get::<_, f64>(3)?,    // liquidity
            r.get::<_, String>(4)?, // outcome
            r.get::<_, f64>(5)?,    // price
        ))
    })?;

    for row in rows.filter_map(|r| r.ok()) {
        let (cid, cat, vol, liq, outcome, price) = row;
        let e = map.entry(cid).or_insert(Md {
            category: if cat.is_empty() {
                "Other".to_string()
            } else {
                cat
            },
            volume: vol,
            liquidity: liq,
            yes_price: None,
        });
        if outcome.eq_ignore_ascii_case("yes") {
            e.yes_price = Some(price);
        }
    }

    Ok(map
        .into_values()
        .map(|m| (m.category, m.volume, m.liquidity, m.yes_price))
        .collect())
}

/// Resolution outcome counts: `(yes, no, other)`.
pub fn query_resolution_counts(conn: &Connection) -> rusqlite::Result<(usize, usize, usize)> {
    let mut stmt = conn.prepare(
        "SELECT LOWER(resolution), COUNT(*) FROM resolutions GROUP BY LOWER(resolution)",
    )?;
    let (mut yes, mut no, mut other) = (0usize, 0usize, 0usize);
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?)))?;
    for (res, count) in rows.filter_map(|r| r.ok()) {
        match res.as_str() {
            "yes" => yes += count,
            "no" => no += count,
            _ => other += count,
        }
    }
    Ok((yes, no, other))
}

/// High-confidence prediction accuracy (Chart D).
///
/// For each resolved binary market, finds the latest "Yes" snapshot within
/// the 6 hours before the market's close.  If that snapshot showed >80%
/// confidence on either side, records whether the prediction was correct.
///
/// Returns `(correct, wrong)`.
pub fn query_high_confidence_accuracy(conn: &Connection) -> rusqlite::Result<(usize, usize)> {
    // SQLite's `datetime()` handles ISO-8601 strings natively.
    let sql = "
        SELECT r.resolution, s.price
        FROM (
            SELECT s.condition_id, MAX(s.snapshot_at) AS best_snap
            FROM snapshots s
            JOIN resolutions r ON s.condition_id = r.condition_id
            WHERE s.outcome = 'Yes'
              AND r.end_date != ''
              AND s.snapshot_at >= datetime(r.end_date, '-6 hours')
              AND s.snapshot_at <= r.end_date
            GROUP BY s.condition_id
        ) best
        JOIN snapshots s
          ON s.condition_id = best.condition_id
         AND s.snapshot_at  = best.best_snap
         AND s.outcome       = 'Yes'
        JOIN resolutions r ON r.condition_id = best.condition_id
        WHERE LOWER(r.resolution) IN ('yes', 'no')
    ";
    let mut stmt = conn.prepare(sql)?;
    let mut correct = 0usize;
    let mut wrong = 0usize;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?;
    for (resolution, yes_price) in rows.filter_map(|r| r.ok()) {
        let no_price = 1.0 - yes_price;
        let predicted_yes = if yes_price > 0.80 {
            true
        } else if no_price > 0.80 {
            false
        } else {
            continue;
        };
        if (resolution.to_lowercase() == "yes") == predicted_yes {
            correct += 1;
        } else {
            wrong += 1;
        }
    }
    Ok((correct, wrong))
}

// ── Calibration price helpers ─────────────────────────────────────────────────

/// Markets with a CLOB token ID but no stored calibration price at `hours_before`.
/// Returns `(condition_id, clob_token_id, end_date)` up to `limit` rows.
pub fn query_unpriced_resolutions(
    conn: &Connection,
    hours_before: u64,
    limit: usize,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT condition_id, clob_token_id, end_date
         FROM resolutions
         WHERE clob_token_id IS NOT NULL AND clob_token_id != ''
           AND (calibration_price IS NULL OR calibration_hours != ?1)
           AND LOWER(resolution) IN ('yes','no')
         LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![hours_before as i64, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Persist a calibration price for a single resolution.
pub fn update_calibration_price(
    conn: &Connection,
    condition_id: &str,
    price: f64,
    hours_before: u64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE resolutions SET calibration_price = ?1, calibration_hours = ?2
         WHERE condition_id = ?3",
        params![price, hours_before as i64, condition_id],
    )?;
    Ok(())
}

/// Calibration data (Chart D).
///
/// For each 10% probability bucket (0 = 0–10%, …, 9 = 90–100%), looks up the
/// YES probability `hours_before` hours before market close and checks how often
/// that market actually resolved YES.
///
/// Price priority per market:
///   1. `calibration_price` stored on the resolution row (fetched from the CLOB
///      `prices-history` API at exactly `hours_before` hours before close).
///   2. Fallback: latest hourly snapshot taken at least `hours_before` hours
///      before the market's `end_date`.
///
/// Returns `[(yes_count, total_count); 10]`.
pub fn query_calibration(
    conn: &Connection,
    hours_before: u64,
) -> rusqlite::Result<[(usize, usize); 10]> {
    // Build the datetime modifier string once in Rust.
    let modifier = format!("-{} hours", hours_before);
    let sql = "
        WITH candidates AS (
            SELECT r.condition_id,
                   COALESCE(
                       -- Primary: CLOB price history at the requested horizon.
                       CASE WHEN r.calibration_hours = ?1 THEN r.calibration_price END,
                       -- Fallback: latest snapshot at least N hours before end_date.
                       (SELECT s.price FROM snapshots s
                        WHERE s.condition_id = r.condition_id
                          AND s.outcome = 'Yes'
                          AND s.end_date != ''
                          AND s.snapshot_at <= datetime(s.end_date, ?2)
                        ORDER BY s.snapshot_at DESC LIMIT 1)
                   ) AS yes_price,
                   LOWER(r.resolution) AS res
            FROM resolutions r
            WHERE LOWER(r.resolution) IN ('yes','no')
        )
        SELECT MIN(CAST(yes_price * 10 AS INTEGER), 9) AS bucket, res
        FROM candidates
        WHERE yes_price BETWEEN 0.01 AND 0.99
    ";
    let mut stmt = conn.prepare(sql)?;
    let mut buckets = [(0usize, 0usize); 10];
    let rows = stmt.query_map(params![hours_before as i64, modifier], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    for (bucket, res) in rows.filter_map(|r| r.ok()) {
        let b = bucket.clamp(0, 9) as usize;
        buckets[b].1 += 1;
        if res == "yes" {
            buckets[b].0 += 1;
        }
    }
    Ok(buckets)
}

/// Raw rows for the per-category × per-volume calibration matrix (Chart D).
///
/// One row per resolved binary market with a usable YES-price at the requested
/// horizon. Price priority matches `query_calibration`:
///   1. stored `calibration_price` (when `calibration_hours` matches)
///   2. fallback: latest snapshot at least N hours before `end_date`.
///
/// Returns [`CalibrationRow`] tuples `(question, slug, peak_volume, yes_price, resolution)`.
/// Peak volume comes from the market's latest history of snapshots.
pub fn query_calibration_raw(
    conn: &Connection,
    hours_before: u64,
) -> rusqlite::Result<Vec<CalibrationRow>> {
    let modifier = format!("-{} hours", hours_before);
    let sql = "
        SELECT r.question, r.slug,
               COALESCE(v.volume, 0) AS peak_vol,
               COALESCE(
                   CASE WHEN r.calibration_hours = ?1 THEN r.calibration_price END,
                   (SELECT s.price FROM snapshots s
                    WHERE s.condition_id = r.condition_id
                      AND s.outcome = 'Yes'
                      AND s.end_date != ''
                      AND s.snapshot_at <= datetime(s.end_date, ?2)
                    ORDER BY s.snapshot_at DESC LIMIT 1)
               ) AS yes_price,
               LOWER(r.resolution) AS res
        FROM resolutions r
        LEFT JOIN market_peak_vol v ON r.condition_id = v.condition_id
        WHERE LOWER(r.resolution) IN ('yes','no')
    ";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![hours_before as i64, modifier], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, f64>(2)?,
            r.get::<_, Option<f64>>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows.filter_map(|r| r.ok()) {
        let (q, s, vol, yes_price, res) = row;
        if let Some(yp) = yes_price {
            if (0.01..=0.99).contains(&yp) {
                out.push((q, s, vol, yp, res));
            }
        }
    }
    Ok(out)
}

/// Price accuracy vs market volume (Chart D).
///
/// Groups resolved binary markets (that have a stored calibration price) by
/// their peak trading volume and computes the mean absolute error (MAE)
/// between the calibration price and the binary outcome (1 = YES, 0 = NO).
///
/// Volume tiers (5): <$1K · $1K–$10K · $10K–$100K · $100K–$1M · >$1M
///
/// Returns one entry per tier: `(label, mae, count)`.
/// Tiers with no data have `count == 0`.
pub fn query_edge_vs_volume(conn: &Connection) -> rusqlite::Result<Vec<(String, f64, usize)>> {
    let sql = "
        WITH calibrated AS (
            SELECT
                ABS(r.calibration_price - CASE WHEN LOWER(r.resolution) = 'yes' THEN 1.0 ELSE 0.0 END) AS abs_err,
                CASE
                    WHEN COALESCE(v.volume, 0) < 1000    THEN 0
                    WHEN COALESCE(v.volume, 0) < 10000   THEN 1
                    WHEN COALESCE(v.volume, 0) < 100000  THEN 2
                    WHEN COALESCE(v.volume, 0) < 1000000 THEN 3
                    ELSE 4
                END AS tier
            FROM resolutions r
            LEFT JOIN market_peak_vol v ON r.condition_id = v.condition_id
            WHERE r.calibration_price IS NOT NULL
              AND LOWER(r.resolution) IN ('yes', 'no')
        )
        SELECT tier, AVG(abs_err), COUNT(*) FROM calibrated GROUP BY tier ORDER BY tier
    ";
    let mut stmt = conn.prepare(sql)?;
    const LABELS: [&str; 5] = ["<$1K", "$1K–10K", "$10K–100K", "$100K–1M", ">$1M"];
    let mut result: Vec<(String, f64, usize)> =
        LABELS.iter().map(|&l| (l.to_string(), 0.0, 0)).collect();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, f64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for (tier, avg_err, cnt) in rows.filter_map(|r| r.ok()) {
        let t = tier.clamp(0, 4) as usize;
        result[t] = (LABELS[t].to_string(), avg_err, cnt as usize);
    }
    Ok(result)
}

/// Markets whose most-recent snapshot has `status = 'Closed'` but have no
/// resolution recorded yet.  Used by the cross-reference pass.
///
/// Returns `(condition_id, question, slug, end_date)`.
pub fn query_unresolved_closed(conn: &Connection) -> rusqlite::Result<Vec<UnresolvedRow>> {
    let sql = "
        SELECT s.condition_id, s.question, s.slug,
               NULLIF(s.end_date, '') AS end_date
        FROM snapshots s
        WHERE s.snapshot_at = (
            SELECT MAX(s2.snapshot_at)
            FROM snapshots s2
            WHERE s2.condition_id = s.condition_id
        )
        AND LOWER(s.status) = 'closed'
        AND s.condition_id NOT IN (SELECT condition_id FROM resolutions)
        GROUP BY s.condition_id
    ";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

// ── Net worth log ─────────────────────────────────────────────────────────────

/// Insert a single net worth data point. Ignores duplicates (same timestamp).
pub fn insert_net_worth(
    conn: &Connection,
    logged_at: &str,
    balance: f64,
    positions_value: f64,
    net_worth: f64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO net_worth_log (logged_at, balance, positions, net_worth)
         VALUES (?1, ?2, ?3, ?4)",
        params![logged_at, balance, positions_value, net_worth],
    )?;
    Ok(())
}

/// Load recent net worth history as `(epoch_secs, net_worth)` in chronological order.
/// Returns up to 1000 rows (~7 days at 10-minute intervals).
pub fn query_net_worth_history(conn: &Connection) -> rusqlite::Result<Vec<(f64, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT logged_at, net_worth FROM net_worth_log
         WHERE net_worth > 1.0
         ORDER BY logged_at DESC LIMIT 1000",
    )?;
    let mut rows: Vec<(f64, f64)> = stmt
        .query_map([], |r| {
            let ts: String = r.get(0)?;
            let nw: f64 = r.get(1)?;
            Ok((ts, nw))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(ts, nw)| {
            chrono::DateTime::parse_from_rfc3339(&ts)
                .ok()
                .map(|dt| (dt.timestamp() as f64, nw))
        })
        .collect();
    rows.reverse(); // chronological order (oldest first)
    Ok(rows)
}

// ── One-time CSV migration ─────────────────────────────────────────────────────

/// Import legacy CSV data into SQLite on first run.
///
/// No-ops immediately if the `snapshots` table already has rows (already
/// migrated) or if no CSV files exist (fresh install).
///
/// Returns `(snapshot_rows, resolution_rows)` — the counts that were imported.
/// Returns `(0, 0)` when already migrated or nothing to import.
pub fn migrate_from_csvs(
    db_path: &Path,
    snapshots_csv: &Path,
    resolutions_csv: &Path,
) -> (usize, usize) {
    let Ok(mut conn) = open(db_path) else {
        return (0, 0);
    };

    // Already migrated — nothing to do.
    let snap_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
        .unwrap_or(0);
    if snap_count > 0 {
        return (0, 0);
    }

    let mut imported_res = 0usize;
    let mut imported_snap = 0usize;

    // ── Resolutions (small file, fast) ────────────────────────────────────────
    if resolutions_csv.exists() {
        if let Ok(text) = std::fs::read_to_string(resolutions_csv) {
            let rows: Vec<ResolutionRow> = text
                .lines()
                .skip(1)
                .filter_map(|line| {
                    let f = parse_csv_line(line);
                    if f.len() < 5 {
                        return None;
                    }
                    Some(ResolutionRow {
                        condition_id: f[0].clone(),
                        question: f[1].clone(),
                        slug: f[2].clone(),
                        end_date: f[3].clone(),
                        resolution: f[4].clone(),
                        last_trade_price: None,
                        clob_token_id: None,
                    })
                })
                .collect();
            imported_res = rows.len();
            let _ = insert_resolutions(&mut conn, &rows);
        }
    }

    // ── Snapshots (potentially large — batched in 10 000-row transactions) ────
    if snapshots_csv.exists() {
        use std::io::{BufRead, BufReader};
        let Ok(file) = std::fs::File::open(snapshots_csv) else {
            return (0, imported_res);
        };
        let reader = BufReader::new(file);
        let mut batch: Vec<SnapshotRow> = Vec::with_capacity(10_000);

        for line in reader.lines().skip(1).filter_map(|l| l.ok()) {
            let f = parse_csv_line(&line);
            if f.len() < 11 {
                continue;
            }
            batch.push(SnapshotRow {
                snapshot_at: f[0].clone(),
                condition_id: f[1].clone(),
                question: f[2].clone(),
                slug: f[3].clone(),
                category: f[4].clone(),
                status: f[5].clone(),
                end_date: f[6].clone(),
                volume: f[7].parse().unwrap_or(0.0),
                liquidity: f[8].parse().unwrap_or(0.0),
                outcome: f[9].clone(),
                price: f[10].parse().unwrap_or(0.0),
            });
            if batch.len() >= 10_000 {
                imported_snap += batch.len();
                let _ = insert_snapshots(&mut conn, &batch);
                batch.clear();
            }
        }
        if !batch.is_empty() {
            imported_snap += batch.len();
            let _ = insert_snapshots(&mut conn, &batch);
        }
    }

    (imported_snap, imported_res)
}

// ── Internal helpers ───────────────────────────────────────────────────────────

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => fields.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    fields.push(current);
    fields
}
