use rusqlite::params;
use serde::Serialize;

use crate::actions::rules::ActionType;
use crate::db::Db;
use crate::error::{DbError, DbResult};

#[derive(Clone, Debug, Serialize)]
pub struct StagedAction {
    pub id: i64,
    pub account_id: String,
    pub cluster_key: String,
    pub provider_msg_id: Option<String>,
    pub action_type: String,
    pub payload: String,
    pub proposed_reason: Option<String>,
    pub staged_at: String,
}

pub struct StagedActionsRepo<'a> {
    pub db: &'a Db,
}

impl<'a> StagedActionsRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Stage a cluster-wide action (provider_msg_id = NULL). Idempotent on
    /// (account, cluster_key, action_type, NULL).
    pub fn stage_cluster(
        &self,
        account_id: &str,
        cluster_key: &str,
        action: ActionType,
        proposed_reason: Option<&str>,
    ) -> DbResult<i64> {
        self.db.with_connection(|conn| {
            // Use ON CONFLICT against the partial index columns. SQLite
            // matches the partial UNIQUE index `idx_staged_actions_uniq_cluster`
            // (which covers provider_msg_id IS NULL rows).
            conn.execute(
                "INSERT INTO staged_actions
                    (account_id, cluster_key, provider_msg_id, action_type, payload, proposed_reason)
                 VALUES (?1, ?2, NULL, ?3, '{}', ?4)
                 ON CONFLICT(account_id, cluster_key, action_type)
                   WHERE provider_msg_id IS NULL
                 DO UPDATE SET
                    proposed_reason = excluded.proposed_reason,
                    staged_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![account_id, cluster_key, action.as_str(), proposed_reason],
            )
            .map_err(DbError::from)?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn stage_message(
        &self,
        account_id: &str,
        cluster_key: &str,
        provider_msg_id: &str,
        action: ActionType,
        proposed_reason: Option<&str>,
    ) -> DbResult<i64> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO staged_actions
                    (account_id, cluster_key, provider_msg_id, action_type, payload, proposed_reason)
                 VALUES (?1, ?2, ?3, ?4, '{}', ?5)
                 ON CONFLICT(account_id, cluster_key, action_type, provider_msg_id)
                   WHERE provider_msg_id IS NOT NULL
                 DO UPDATE SET
                    proposed_reason = excluded.proposed_reason,
                    staged_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![
                    account_id,
                    cluster_key,
                    provider_msg_id,
                    action.as_str(),
                    proposed_reason,
                ],
            )
            .map_err(DbError::from)?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn unstage_cluster(
        &self,
        account_id: &str,
        cluster_key: &str,
        action: ActionType,
    ) -> DbResult<usize> {
        self.db.with_connection(|conn| {
            conn.execute(
                "DELETE FROM staged_actions
                 WHERE account_id = ?1 AND cluster_key = ?2 AND action_type = ?3
                   AND provider_msg_id IS NULL",
                params![account_id, cluster_key, action.as_str()],
            )
            .map_err(DbError::from)
        })
    }

    pub fn unstage_by_id(&self, id: i64) -> DbResult<usize> {
        self.db.with_connection(|conn| {
            conn.execute(
                "DELETE FROM staged_actions WHERE id = ?1",
                params![id],
            )
            .map_err(DbError::from)
        })
    }

    pub fn list_for_account(&self, account_id: &str) -> DbResult<Vec<StagedAction>> {
        self.db.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, cluster_key, provider_msg_id, action_type,
                            payload, proposed_reason, staged_at
                     FROM staged_actions
                     WHERE account_id = ?1
                     ORDER BY staged_at ASC",
                )
                .map_err(DbError::from)?;
            let rows = stmt
                .query_map(params![account_id], |row| {
                    Ok(StagedAction {
                        id: row.get(0)?,
                        account_id: row.get(1)?,
                        cluster_key: row.get(2)?,
                        provider_msg_id: row.get(3)?,
                        action_type: row.get(4)?,
                        payload: row.get(5)?,
                        proposed_reason: row.get(6)?,
                        staged_at: row.get(7)?,
                    })
                })
                .map_err(DbError::from)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(DbError::from)?);
            }
            Ok(out)
        })
    }

    pub fn count_for_account(&self, account_id: &str) -> DbResult<i64> {
        self.db.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM staged_actions WHERE account_id = ?1",
                params![account_id],
                |r| r.get(0),
            )
            .map_err(DbError::from)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    fn db() -> Db {
        let db = open_in_memory().unwrap();
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO accounts (account_id, provider, email) VALUES ('a1','gmail','a@b')",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        db
    }

    #[test]
    fn stage_then_list() {
        let db = db();
        let repo = StagedActionsRepo::new(&db);
        repo.stage_cluster("a1", "c1", ActionType::Archive, Some("test")).unwrap();
        let v = repo.list_for_account("a1").unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].action_type, "archive");
    }

    #[test]
    fn staging_same_cluster_twice_is_idempotent() {
        let db = db();
        let repo = StagedActionsRepo::new(&db);
        repo.stage_cluster("a1", "c1", ActionType::Archive, Some("first")).unwrap();
        repo.stage_cluster("a1", "c1", ActionType::Archive, Some("second")).unwrap();
        assert_eq!(repo.count_for_account("a1").unwrap(), 1);
        let v = repo.list_for_account("a1").unwrap();
        assert_eq!(v[0].proposed_reason.as_deref(), Some("second"));
    }

    #[test]
    fn cluster_and_message_level_actions_coexist() {
        let db = db();
        let repo = StagedActionsRepo::new(&db);
        repo.stage_cluster("a1", "c1", ActionType::Archive, None).unwrap();
        repo.stage_message("a1", "c1", "m1", ActionType::Archive, None).unwrap();
        assert_eq!(repo.count_for_account("a1").unwrap(), 2);
    }

    #[test]
    fn unstage_removes_only_cluster_level() {
        let db = db();
        let repo = StagedActionsRepo::new(&db);
        repo.stage_cluster("a1", "c1", ActionType::Archive, None).unwrap();
        repo.stage_message("a1", "c1", "m1", ActionType::Archive, None).unwrap();
        let n = repo.unstage_cluster("a1", "c1", ActionType::Archive).unwrap();
        assert_eq!(n, 1);
        assert_eq!(repo.count_for_account("a1").unwrap(), 1);
    }
}
