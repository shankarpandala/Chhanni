use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::{DbError, DbResult};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Transactional,
    Newsletter,
    Social,
    Personal,
    Work,
    Security,
    Promotional,
    Notification,
    Unknown,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Transactional => "transactional",
            Category::Newsletter => "newsletter",
            Category::Social => "social",
            Category::Personal => "personal",
            Category::Work => "work",
            Category::Security => "security",
            Category::Promotional => "promotional",
            Category::Notification => "notification",
            Category::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "transactional" => Category::Transactional,
            "newsletter" => Category::Newsletter,
            "social" => Category::Social,
            "personal" => Category::Personal,
            "work" => Category::Work,
            "security" => Category::Security,
            "promotional" => Category::Promotional,
            "notification" => Category::Notification,
            _ => Category::Unknown,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ClassificationRow {
    pub cluster_key: String,
    pub category: Category,
    pub confidence: f32,
    pub reason: Option<String>,
    pub model_version: String,
    pub prompt_version: String,
    pub cluster_signature: String,
}

pub struct ClassificationsRepo<'a> {
    pub db: &'a Db,
}

impl<'a> ClassificationsRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub fn get(
        &self,
        account_id: &str,
        cluster_key: &str,
    ) -> DbResult<Option<ClassificationRow>> {
        self.db.with_connection(|conn| {
            conn.query_row(
                "SELECT cluster_key, category, confidence, reason,
                        model_version, prompt_version, cluster_signature
                 FROM cluster_classifications
                 WHERE account_id = ?1 AND cluster_key = ?2",
                params![account_id, cluster_key],
                |row| {
                    Ok(ClassificationRow {
                        cluster_key: row.get(0)?,
                        category: Category::parse(&row.get::<_, String>(1)?),
                        confidence: row.get::<_, f64>(2)? as f32,
                        reason: row.get(3)?,
                        model_version: row.get(4)?,
                        prompt_version: row.get(5)?,
                        cluster_signature: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(DbError::from)
        })
    }

    /// Upserts the classification AND propagates the (category, confidence)
    /// to every member message of the cluster in a single transaction.
    pub fn upsert_and_propagate(
        &self,
        account_id: &str,
        row: &ClassificationRow,
    ) -> DbResult<()> {
        self.db.with_connection(|conn| {
            let tx = conn.transaction().map_err(DbError::from)?;
            tx.execute(
                "INSERT INTO cluster_classifications
                    (account_id, cluster_key, category, confidence, reason,
                     model_version, prompt_version, cluster_signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(account_id, cluster_key) DO UPDATE SET
                    category          = excluded.category,
                    confidence        = excluded.confidence,
                    reason            = excluded.reason,
                    model_version     = excluded.model_version,
                    prompt_version    = excluded.prompt_version,
                    cluster_signature = excluded.cluster_signature,
                    created_at        = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![
                    account_id,
                    row.cluster_key,
                    row.category.as_str(),
                    row.confidence as f64,
                    row.reason,
                    row.model_version,
                    row.prompt_version,
                    row.cluster_signature,
                ],
            )
            .map_err(DbError::from)?;

            tx.execute(
                "UPDATE messages
                 SET category = ?3,
                     classified_confidence = ?4,
                     classified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE account_id = ?1
                   AND provider_msg_id IN (
                     SELECT provider_msg_id FROM message_clusters
                     WHERE account_id = ?1 AND cluster_key = ?2
                   )",
                params![
                    account_id,
                    row.cluster_key,
                    row.category.as_str(),
                    row.confidence as f64,
                ],
            )
            .map_err(DbError::from)?;
            tx.commit().map_err(DbError::from)?;
            Ok(())
        })
    }

    pub fn count(&self, account_id: &str) -> DbResult<i64> {
        self.db.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM cluster_classifications WHERE account_id = ?1",
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
    use crate::db::{open_in_memory, ClusterMember, ClustersRepo, MessageRow, MessagesRepo};

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

    fn add_message_and_cluster(db: &Db, msg_id: &str, cluster_key: &str) {
        MessagesRepo::new(db)
            .upsert(&MessageRow {
                account_id: "a1".into(),
                provider_msg_id: msg_id.into(),
                thread_id: format!("t-{msg_id}"),
                sender: None,
                sender_email: None,
                subject: None,
                snippet: None,
                internal_date: 0,
                label_ids: vec![],
                history_id: None,
            })
            .unwrap();
        ClustersRepo::new(db)
            .upsert_many(
                "a1",
                &[ClusterMember {
                    provider_msg_id: msg_id.into(),
                    cluster_key: cluster_key.into(),
                    centroid_cosine: Some(1.0),
                }],
            )
            .unwrap();
    }

    #[test]
    fn upsert_propagates_to_members() {
        let db = db();
        add_message_and_cluster(&db, "m1", "sender::news@example.com");
        add_message_and_cluster(&db, "m2", "sender::news@example.com");
        add_message_and_cluster(&db, "m3", "sender::other@example.com");

        ClassificationsRepo::new(&db)
            .upsert_and_propagate(
                "a1",
                &ClassificationRow {
                    cluster_key: "sender::news@example.com".into(),
                    category: Category::Newsletter,
                    confidence: 0.92,
                    reason: Some("looks like a newsletter".into()),
                    model_version: "qwen3-4b/Q4_K_M".into(),
                    prompt_version: "v1".into(),
                    cluster_signature: "sig-1".into(),
                },
            )
            .unwrap();

        db.with_connection(|c| {
            let m1: Option<String> = c
                .query_row(
                    "SELECT category FROM messages WHERE provider_msg_id='m1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let m3: Option<String> = c
                .query_row(
                    "SELECT category FROM messages WHERE provider_msg_id='m3'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(m1.as_deref(), Some("newsletter"));
            assert!(m3.is_none(), "different cluster should not be touched");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn get_returns_none_when_missing() {
        let db = db();
        let got = ClassificationsRepo::new(&db).get("a1", "missing").unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn category_round_trip() {
        for c in [
            Category::Transactional,
            Category::Newsletter,
            Category::Social,
            Category::Personal,
            Category::Work,
            Category::Security,
            Category::Promotional,
            Category::Notification,
            Category::Unknown,
        ] {
            assert_eq!(Category::parse(c.as_str()), c);
        }
        assert_eq!(Category::parse("garbage"), Category::Unknown);
    }
}
