use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Append-only SQLite event log of cast attempts.
pub struct CastLog {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct CastEvent {
    pub spell: String,
    pub channel: String,
    pub exit_code: i32,
    pub version_before: Option<String>,
    pub version_after: Option<String>,
    pub error: Option<String>,
}

impl CastLog {
    pub fn open() -> Result<Self> {
        let path = state_dir()?.join("cast.db");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating state dir {}", parent.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("opening cast log at {}", path.display()))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS casts (
                id              INTEGER PRIMARY KEY,
                ts              INTEGER NOT NULL,
                spell           TEXT    NOT NULL,
                channel         TEXT    NOT NULL,
                exit_code       INTEGER NOT NULL,
                version_before  TEXT,
                version_after   TEXT,
                error           TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_casts_spell ON casts(spell);
            CREATE INDEX IF NOT EXISTS idx_casts_ts    ON casts(ts);
            ",
        )
        .context("initializing cast log schema")?;
        Ok(Self { conn })
    }

    pub fn record(&self, event: &CastEvent) -> Result<()> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.conn
            .execute(
                "INSERT INTO casts
                 (ts, spell, channel, exit_code, version_before, version_after, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    ts,
                    event.spell,
                    event.channel,
                    event.exit_code,
                    event.version_before,
                    event.version_after,
                    event.error,
                ],
            )
            .context("inserting cast event")?;
        Ok(())
    }

    /// Most recent successful cast (exit_code = 0, error IS NULL) for `spell`.
    pub fn last_successful_cast(&self, spell: &str) -> Result<Option<CastRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, channel, version_after FROM casts
             WHERE spell = ?1 AND exit_code = 0 AND error IS NULL
             ORDER BY ts DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![spell])?;
        if let Some(row) = rows.next()? {
            Ok(Some(CastRecord {
                ts: row.get(0)?,
                channel: row.get(1)?,
                version: row.get(2)?,
            }))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone)]
pub struct CastRecord {
    pub ts: i64,
    pub channel: String,
    pub version: Option<String>,
}

fn state_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .context("neither XDG_STATE_HOME nor HOME is set")?;
    Ok(base.join("grimoire"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log() -> CastLog {
        // Test logs go to a per-test temp file via XDG_STATE_HOME override.
        let dir = std::env::temp_dir().join(format!("grimoire-test-{}", rand_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        // Safety: process-wide env mutation; tests in this module run sequentially
        // because they share this var. cargo test by default runs them in parallel
        // across modules but within a module they're independent. We use unique
        // per-test dirs so collisions are avoided.
        unsafe {
            std::env::set_var("XDG_STATE_HOME", &dir);
        }
        CastLog::open().expect("open cast log")
    }

    fn rand_suffix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }

    #[test]
    fn record_roundtrip() {
        let log = temp_log();
        log.record(&CastEvent {
            spell: "rust".into(),
            channel: "rustup".into(),
            exit_code: 0,
            version_before: None,
            version_after: Some("1.95.0".into()),
            error: None,
        })
        .unwrap();

        let count: i64 = log
            .conn
            .query_row("SELECT COUNT(*) FROM casts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
