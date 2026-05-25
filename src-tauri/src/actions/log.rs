use rusqlite::params;
use serde::Serialize;

use crate::db::Db;
use crate::error::{DbError, DbResult};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Success,
    Failure,
    Cancelled,
    Skipped,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::Failure => "failure",
            Outcome::Cancelled => "cancelled",
            Outcome::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ActionLogEntry {
    pub id: i64,
    pub account_id: String,
    pub staged_action_id: Option<i64>,
    pub cluster_key: String,
    pub provider_msg_id: String,
    pub action_type: String,
    pub outcome: String,
    pub error_message: Option<String>,
    pub executed_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct OutcomeCounts {
    pub success: i64,
    pub failure: i64,
    pub cancelled: i64,
    pub skipped: i64,
}

pub struct ActionsLogRepo<'a> {
    pub db: &'a Db,
}

impl<'a> ActionsLogRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_many(
        &self,
        account_id: &str,
        staged_action_id: Option<i64>,
        cluster_key: &str,
        action_type: &str,
        provider_msg_ids: &[String],
        outcome: Outcome,
        error_message: Option<&str>,
    ) -> DbResult<usize> {
        if provider_msg_ids.is_empty() {
            return Ok(0);
        }
        self.db.with_connection(|conn| {
            let tx = conn.transaction().map_err(DbError::from)?;
            let mut count = 0usize;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO actions_log
                            (account_id, staged_action_id, cluster_key, provider_msg_id,
                             action_type, outcome, error_message)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    )
                    .map_err(DbError::from)?;
                for id in provider_msg_ids {
                    stmt.execute(params![
                        account_id,
                        staged_action_id,
                        cluster_key,
                        id,
                        action_type,
                        outcome.as_str(),
                        error_message,
                    ])
                    .map_err(DbError::from)?;
                    count += 1;
                }
            }
            tx.commit().map_err(DbError::from)?;
            Ok(count)
        })
    }

    pub fn counts(&self, account_id: &str) -> DbResult<OutcomeCounts> {
        self.db.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT outcome, COUNT(*) FROM actions_log
                     WHERE account_id = ?1 GROUP BY outcome",
                )
                .map_err(DbError::from)?;
            let mut counts = OutcomeCounts {
                success: 0,
                failure: 0,
                cancelled: 0,
                skipped: 0,
            };
            let rows = stmt
                .query_map(params![account_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })
                .map_err(DbError::from)?;
            for row in rows {
                let (o, n) = row.map_err(DbError::from)?;
                match o.as_str() {
                    "success" => counts.success = n,
                    "failure" => counts.failure = n,
                    "cancelled" => counts.cancelled = n,
                    "skipped" => counts.skipped = n,
                    _ => {}
                }
            }
            Ok(counts)
        })
    }

    pub fn list_recent(&self, account_id: &str, limit: u32) -> DbResult<Vec<ActionLogEntry>> {
        self.db.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, staged_action_id, cluster_key, provider_msg_id,
                            action_type, outcome, error_message, executed_at
                     FROM actions_log
                     WHERE account_id = ?1
                     ORDER BY executed_at DESC, id DESC
                     LIMIT ?2",
                )
                .map_err(DbError::from)?;
            let rows = stmt
                .query_map(params![account_id, limit as i64], |r| {
                    Ok(ActionLogEntry {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        staged_action_id: r.get(2)?,
                        cluster_key: r.get(3)?,
                        provider_msg_id: r.get(4)?,
                        action_type: r.get(5)?,
                        outcome: r.get(6)?,
                        error_message: r.get(7)?,
                        executed_at: r.get(8)?,
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
    fn record_many_then_counts() {
        let db = db();
        let repo = ActionsLogRepo::new(&db);
        let ids: Vec<String> = (0..5).map(|i| format!("m{i}")).collect();
        repo.record_many("a1", None, "c1", "archive", &ids, Outcome::Success, None)
            .unwrap();
        repo.record_many(
            "a1",
            None,
            "c1",
            "archive",
            &["m6".to_owned()],
            Outcome::Failure,
            Some("http 500"),
        )
        .unwrap();
        let c = repo.counts("a1").unwrap();
        assert_eq!(c.success, 5);
        assert_eq!(c.failure, 1);
        assert_eq!(c.cancelled, 0);
    }

    #[test]
    fn list_recent_orders_newest_first() {
        let db = db();
        let repo = ActionsLogRepo::new(&db);
        repo.record_many("a1", None, "c1", "archive", &["m1".into()], Outcome::Success, None)
            .unwrap();
        repo.record_many("a1", None, "c1", "archive", &["m2".into()], Outcome::Success, None)
            .unwrap();
        let v = repo.list_recent("a1", 10).unwrap();
        assert_eq!(v.len(), 2);
        // Same timestamp resolution, so id desc is the tiebreaker.
        assert_eq!(v[0].provider_msg_id, "m2");
    }
}
