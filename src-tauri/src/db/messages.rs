use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::{DbError, DbResult};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageRow {
    pub account_id: String,
    pub provider_msg_id: String,
    pub thread_id: String,
    pub sender: Option<String>,
    pub sender_email: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub internal_date: i64,
    pub label_ids: Vec<String>,
    pub history_id: Option<u64>,
}

pub struct MessagesRepo<'a> {
    pub db: &'a Db,
}

impl<'a> MessagesRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Insert or update a single row. `ON CONFLICT` keeps inserts idempotent
    /// so re-running an interrupted sync doesn't duplicate.
    pub fn upsert(&self, row: &MessageRow) -> DbResult<()> {
        let label_ids = serde_json::to_string(&row.label_ids).map_err(DbError::Malformed)?;
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO messages
                   (account_id, provider_msg_id, thread_id, sender, sender_email,
                    subject, snippet, internal_date, label_ids, history_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(account_id, provider_msg_id) DO UPDATE SET
                   thread_id    = excluded.thread_id,
                   sender       = excluded.sender,
                   sender_email = excluded.sender_email,
                   subject      = excluded.subject,
                   snippet      = excluded.snippet,
                   internal_date= excluded.internal_date,
                   label_ids    = excluded.label_ids,
                   history_id   = COALESCE(excluded.history_id, messages.history_id)",
                params![
                    row.account_id,
                    row.provider_msg_id,
                    row.thread_id,
                    row.sender,
                    row.sender_email,
                    row.subject,
                    row.snippet,
                    row.internal_date,
                    label_ids,
                    row.history_id.map(|i| i as i64),
                ],
            )
            .map(|_| ())
            .map_err(DbError::from)
        })
    }

    /// Bulk upsert in a single transaction. Returns the number of rows
    /// affected (inserts + updates).
    pub fn upsert_many(&self, rows: &[MessageRow]) -> DbResult<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        self.db.with_connection(|conn| {
            let tx = conn.transaction().map_err(DbError::from)?;
            let mut total = 0usize;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO messages
                           (account_id, provider_msg_id, thread_id, sender, sender_email,
                            subject, snippet, internal_date, label_ids, history_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                         ON CONFLICT(account_id, provider_msg_id) DO UPDATE SET
                           thread_id    = excluded.thread_id,
                           sender       = excluded.sender,
                           sender_email = excluded.sender_email,
                           subject      = excluded.subject,
                           snippet      = excluded.snippet,
                           internal_date= excluded.internal_date,
                           label_ids    = excluded.label_ids,
                           history_id   = COALESCE(excluded.history_id, messages.history_id)",
                    )
                    .map_err(DbError::from)?;
                for row in rows {
                    let label_ids =
                        serde_json::to_string(&row.label_ids).map_err(DbError::Malformed)?;
                    stmt.execute(params![
                        row.account_id,
                        row.provider_msg_id,
                        row.thread_id,
                        row.sender,
                        row.sender_email,
                        row.subject,
                        row.snippet,
                        row.internal_date,
                        label_ids,
                        row.history_id.map(|i| i as i64),
                    ])
                    .map_err(DbError::from)?;
                    total += 1;
                }
            }
            tx.commit().map_err(DbError::from)?;
            Ok(total)
        })
    }

    pub fn delete(&self, account_id: &str, provider_msg_id: &str) -> DbResult<()> {
        self.db.with_connection(|conn| {
            conn.execute(
                "DELETE FROM messages WHERE account_id = ?1 AND provider_msg_id = ?2",
                params![account_id, provider_msg_id],
            )
            .map(|_| ())
            .map_err(DbError::from)
        })
    }

    pub fn count_for_account(&self, account_id: &str) -> DbResult<i64> {
        self.db.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = ?1",
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
                "INSERT INTO accounts (account_id, provider, email) VALUES ('a1', 'gmail', 'a@b')",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        db
    }

    fn row(id: &str) -> MessageRow {
        MessageRow {
            account_id: "a1".to_owned(),
            provider_msg_id: id.to_owned(),
            thread_id: format!("t-{id}"),
            sender: Some("X <x@example.com>".to_owned()),
            sender_email: Some("x@example.com".to_owned()),
            subject: Some("hi".to_owned()),
            snippet: Some("hello there".to_owned()),
            internal_date: 1_700_000_000_000,
            label_ids: vec!["INBOX".to_owned()],
            history_id: Some(1),
        }
    }

    #[test]
    fn upsert_then_count() {
        let db = db();
        let repo = MessagesRepo::new(&db);
        repo.upsert(&row("m1")).unwrap();
        assert_eq!(repo.count_for_account("a1").unwrap(), 1);
    }

    #[test]
    fn upsert_is_idempotent() {
        let db = db();
        let repo = MessagesRepo::new(&db);
        repo.upsert(&row("m1")).unwrap();
        repo.upsert(&row("m1")).unwrap();
        assert_eq!(repo.count_for_account("a1").unwrap(), 1);
    }

    #[test]
    fn upsert_many_in_one_tx() {
        let db = db();
        let repo = MessagesRepo::new(&db);
        let rows: Vec<_> = (0..50).map(|i| row(&format!("m{i}"))).collect();
        let n = repo.upsert_many(&rows).unwrap();
        assert_eq!(n, 50);
        assert_eq!(repo.count_for_account("a1").unwrap(), 50);
    }

    #[test]
    fn delete_removes_row() {
        let db = db();
        let repo = MessagesRepo::new(&db);
        repo.upsert(&row("m1")).unwrap();
        repo.delete("a1", "m1").unwrap();
        assert_eq!(repo.count_for_account("a1").unwrap(), 0);
    }
}
