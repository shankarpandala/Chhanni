use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::{DbError, DbResult};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncPhase {
    Initial,
    Incremental,
}

impl SyncPhase {
    fn as_str(self) -> &'static str {
        match self {
            SyncPhase::Initial => "initial",
            SyncPhase::Incremental => "incremental",
        }
    }

    fn parse(s: &str) -> DbResult<Self> {
        match s {
            "initial" => Ok(SyncPhase::Initial),
            "incremental" => Ok(SyncPhase::Incremental),
            other => Err(DbError::Sqlite(rusqlite::Error::InvalidColumnType(
                0,
                format!("unknown sync phase: {other}"),
                rusqlite::types::Type::Text,
            ))),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncState {
    pub account_id: String,
    pub phase: SyncPhase,
    pub cursor_token: Option<String>,
    pub last_history_id: Option<u64>,
    pub last_sync_at: Option<String>,
}

pub struct SyncStateRepo<'a> {
    pub db: &'a Db,
}

impl<'a> SyncStateRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub fn get(&self, account_id: &str) -> DbResult<Option<SyncState>> {
        self.db.with_connection(|conn| {
            conn.query_row(
                "SELECT account_id, phase, cursor_token, last_history_id, last_sync_at
                 FROM sync_state WHERE account_id = ?1",
                params![account_id],
                |row| {
                    let phase_str: String = row.get(1)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        phase_str,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(DbError::from)?
            .map(|(account_id, phase_s, cursor_token, last_history_id, last_sync_at)| {
                Ok(SyncState {
                    account_id,
                    phase: SyncPhase::parse(&phase_s)?,
                    cursor_token,
                    last_history_id: last_history_id.map(|i| i as u64),
                    last_sync_at,
                })
            })
            .transpose()
        })
    }

    pub fn upsert(&self, state: &SyncState) -> DbResult<()> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sync_state
                     (account_id, phase, cursor_token, last_history_id, last_sync_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(account_id) DO UPDATE SET
                     phase            = excluded.phase,
                     cursor_token     = excluded.cursor_token,
                     last_history_id  = excluded.last_history_id,
                     last_sync_at     = excluded.last_sync_at",
                params![
                    state.account_id,
                    state.phase.as_str(),
                    state.cursor_token,
                    state.last_history_id.map(|i| i as i64),
                    state.last_sync_at,
                ],
            )
            .map(|_| ())
            .map_err(DbError::from)
        })
    }

    pub fn mark_initial_started(&self, account_id: &str) -> DbResult<()> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sync_state (account_id, phase, initial_started_at)
                 VALUES (?1, 'initial', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                 ON CONFLICT(account_id) DO UPDATE SET
                     phase = 'initial',
                     initial_started_at = COALESCE(sync_state.initial_started_at, excluded.initial_started_at)",
                params![account_id],
            )
            .map(|_| ())
            .map_err(DbError::from)
        })
    }

    pub fn mark_initial_complete(&self, account_id: &str, history_id: u64) -> DbResult<()> {
        self.db.with_connection(|conn| {
            conn.execute(
                "UPDATE sync_state
                 SET phase = 'incremental',
                     cursor_token = NULL,
                     last_history_id = ?2,
                     initial_completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     last_sync_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE account_id = ?1",
                params![account_id, history_id as i64],
            )
            .map(|_| ())
            .map_err(DbError::from)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn setup() -> Db {
        let db = open_in_memory().unwrap();
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO accounts (account_id, provider, email) VALUES ('a1', 'gmail', 'a@b')",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        db
    }

    #[test]
    fn upsert_then_get() {
        let db = setup();
        let repo = SyncStateRepo::new(&db);
        assert!(repo.get("a1").unwrap().is_none());
        repo.upsert(&SyncState {
            account_id: "a1".to_owned(),
            phase: SyncPhase::Initial,
            cursor_token: Some("tok".to_owned()),
            last_history_id: None,
            last_sync_at: None,
        })
        .unwrap();
        let got = repo.get("a1").unwrap().unwrap();
        assert_eq!(got.phase, SyncPhase::Initial);
        assert_eq!(got.cursor_token.as_deref(), Some("tok"));
    }

    #[test]
    fn mark_initial_complete_transitions_phase() {
        let db = setup();
        let repo = SyncStateRepo::new(&db);
        repo.mark_initial_started("a1").unwrap();
        assert_eq!(repo.get("a1").unwrap().unwrap().phase, SyncPhase::Initial);
        repo.mark_initial_complete("a1", 42).unwrap();
        let got = repo.get("a1").unwrap().unwrap();
        assert_eq!(got.phase, SyncPhase::Incremental);
        assert_eq!(got.last_history_id, Some(42));
        assert!(got.cursor_token.is_none());
    }
}
