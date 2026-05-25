use rusqlite::{params, OptionalExtension};

use crate::db::Db;
use crate::error::{DbError, DbResult};

#[derive(Clone, Debug)]
pub struct EmbeddingRow {
    pub provider_msg_id: String,
    pub embedding: Vec<f32>,
}

pub struct EmbeddingsRepo<'a> {
    pub db: &'a Db,
}

impl<'a> EmbeddingsRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub fn upsert_many(
        &self,
        account_id: &str,
        model_version: &str,
        rows: &[EmbeddingRow],
    ) -> DbResult<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        self.db.with_connection(|conn| {
            let tx = conn.transaction().map_err(DbError::from)?;
            let mut count = 0usize;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO embeddings
                            (account_id, provider_msg_id, model_version, dim, embedding)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(account_id, provider_msg_id) DO UPDATE SET
                            model_version = excluded.model_version,
                            dim           = excluded.dim,
                            embedding     = excluded.embedding,
                            created_at    = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                    )
                    .map_err(DbError::from)?;
                let mut update_msg = tx
                    .prepare(
                        "UPDATE messages SET embedded_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                         WHERE account_id = ?1 AND provider_msg_id = ?2",
                    )
                    .map_err(DbError::from)?;
                for r in rows {
                    let bytes = f32_to_bytes(&r.embedding);
                    stmt.execute(params![
                        account_id,
                        r.provider_msg_id,
                        model_version,
                        r.embedding.len() as i64,
                        bytes,
                    ])
                    .map_err(DbError::from)?;
                    update_msg
                        .execute(params![account_id, r.provider_msg_id])
                        .map_err(DbError::from)?;
                    count += 1;
                }
            }
            tx.commit().map_err(DbError::from)?;
            Ok(count)
        })
    }

    pub fn get(&self, account_id: &str, provider_msg_id: &str) -> DbResult<Option<Vec<f32>>> {
        self.db.with_connection(|conn| {
            conn.query_row(
                "SELECT embedding FROM embeddings
                 WHERE account_id = ?1 AND provider_msg_id = ?2",
                params![account_id, provider_msg_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(DbError::from)
            .map(|opt| opt.map(|bytes| bytes_to_f32(&bytes)))
        })
    }

    pub fn count_for_account(&self, account_id: &str) -> DbResult<i64> {
        self.db.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM embeddings WHERE account_id = ?1",
                params![account_id],
                |r| r.get(0),
            )
            .map_err(DbError::from)
        })
    }

    /// IDs of messages that exist but have no embedding row yet.
    pub fn list_missing(&self, account_id: &str, limit: usize) -> DbResult<Vec<String>> {
        self.db.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT m.provider_msg_id
                     FROM messages m
                     LEFT JOIN embeddings e
                       ON e.account_id = m.account_id AND e.provider_msg_id = m.provider_msg_id
                     WHERE m.account_id = ?1 AND e.provider_msg_id IS NULL
                     LIMIT ?2",
                )
                .map_err(DbError::from)?;
            let rows = stmt
                .query_map(params![account_id, limit as i64], |r| r.get::<_, String>(0))
                .map_err(DbError::from)?;
            let mut ids = Vec::new();
            for r in rows {
                ids.push(r.map_err(DbError::from)?);
            }
            Ok(ids)
        })
    }
}

fn f32_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        // chunks_exact yields exactly 4 bytes per item.
        let arr = [chunk[0], chunk[1], chunk[2], chunk[3]];
        out.push(f32::from_le_bytes(arr));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::db::{MessageRow, MessagesRepo};

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
        // Two messages to embed against.
        let mr = MessagesRepo::new(&db);
        for id in ["m1", "m2"] {
            mr.upsert(&MessageRow {
                account_id: "a1".into(),
                provider_msg_id: id.into(),
                thread_id: format!("t-{id}"),
                sender: None,
                sender_email: None,
                subject: None,
                snippet: None,
                internal_date: 0,
                label_ids: vec![],
                history_id: None,
            })
            .unwrap();
        }
        db
    }

    #[test]
    fn round_trip_preserves_f32_values() {
        let v = vec![1.5f32, -2.25, 0.0, 1e-7, f32::INFINITY];
        let b = f32_to_bytes(&v);
        let v2 = bytes_to_f32(&b);
        assert_eq!(v.len(), v2.len());
        for (a, b) in v.iter().zip(v2.iter()) {
            if a.is_finite() {
                assert!((a - b).abs() < 1e-12);
            } else {
                assert_eq!(a, b);
            }
        }
    }

    #[test]
    fn upsert_then_get_and_count() {
        let db = db();
        let repo = EmbeddingsRepo::new(&db);
        repo.upsert_many(
            "a1",
            "nomic-v1.5/Q8",
            &[
                EmbeddingRow {
                    provider_msg_id: "m1".into(),
                    embedding: vec![0.1, 0.2, 0.3],
                },
                EmbeddingRow {
                    provider_msg_id: "m2".into(),
                    embedding: vec![0.9, 0.8, 0.7],
                },
            ],
        )
        .unwrap();
        assert_eq!(repo.count_for_account("a1").unwrap(), 2);
        let got = repo.get("a1", "m1").unwrap().unwrap();
        assert!((got[0] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn list_missing_excludes_embedded() {
        let db = db();
        let repo = EmbeddingsRepo::new(&db);
        repo.upsert_many(
            "a1",
            "v1",
            &[EmbeddingRow {
                provider_msg_id: "m1".into(),
                embedding: vec![1.0, 0.0],
            }],
        )
        .unwrap();
        let missing = repo.list_missing("a1", 100).unwrap();
        assert_eq!(missing, vec!["m2".to_owned()]);
    }
}
