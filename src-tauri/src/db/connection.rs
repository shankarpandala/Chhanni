use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;

use crate::db::embedded;
use crate::error::{DbError, DbResult};

/// Shared, thread-safe handle around a single SQLite connection. We use a
/// `Mutex` rather than a pool because:
///   - SQLite handles only one writer at a time anyway,
///   - the workload is small (one mailbox at a time),
///   - it sidesteps `r2d2`/`deadpool-sqlite` for now.
#[derive(Clone)]
pub struct Db {
    inner: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn with_connection<R>(&self, f: impl FnOnce(&mut Connection) -> DbResult<R>) -> DbResult<R> {
        let mut guard = self.inner.lock();
        f(&mut guard)
    }
}

/// Default location for the SQLite file. Resolution rules:
///   1. If `CHHANNI_DATA_DIR` is set, use `<dir>/chhanni.sqlite`.
///   2. Otherwise resolve `<platform app data>/chhanni/chhanni.sqlite` via
///      the `directories` crate (macOS: `~/Library/Application Support/...`).
pub fn default_db_path() -> DbResult<PathBuf> {
    if let Ok(dir) = std::env::var("CHHANNI_DATA_DIR") {
        return Ok(PathBuf::from(dir).join("chhanni.sqlite"));
    }
    let project = directories::ProjectDirs::from("com", "chhanni", "chhanni")
        .ok_or(DbError::DataDirUnavailable)?;
    Ok(project.data_dir().join("chhanni.sqlite"))
}

/// Open (or create) the SQLite file at `path`, applying pragmas and pending
/// migrations.
pub fn open_with_path(path: &Path) -> DbResult<Db> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(DbError::Io)?;
    }
    let conn = Connection::open(path).map_err(DbError::Sqlite)?;
    finalize(conn)
}

/// In-memory variant for tests.
pub fn open_in_memory() -> DbResult<Db> {
    let conn = Connection::open_in_memory().map_err(DbError::Sqlite)?;
    finalize(conn)
}

fn finalize(mut conn: Connection) -> DbResult<Db> {
    apply_pragmas(&conn)?;
    embedded::migrations::runner()
        .run(&mut conn)
        .map_err(|e: refinery::Error| DbError::Migration(e.to_string()))?;
    Ok(Db {
        inner: Arc::new(Mutex::new(conn)),
    })
}

fn apply_pragmas(conn: &Connection) -> DbResult<()> {
    // WAL gives us concurrent readers + a single writer with better crash
    // semantics. NORMAL sync is the standard recommendation with WAL.
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(DbError::Sqlite)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(DbError::Sqlite)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(DbError::Sqlite)?;
    conn.pragma_update(None, "temp_store", "MEMORY")
        .map_err(DbError::Sqlite)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_db_runs_migrations() {
        let db = open_in_memory().unwrap();
        db.with_connection(|c| {
            let n: i64 = c
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='messages'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn migrations_are_idempotent_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.sqlite");
        let _ = open_with_path(&path).unwrap();
        let _ = open_with_path(&path).unwrap();
    }
}
