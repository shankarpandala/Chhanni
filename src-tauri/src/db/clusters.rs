use rusqlite::params;
use serde::Serialize;

use crate::db::Db;
use crate::error::{DbError, DbResult};

#[derive(Clone, Debug, Serialize)]
pub struct ClusterMember {
    pub provider_msg_id: String,
    pub cluster_key: String,
    pub centroid_cosine: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClusterSummary {
    pub cluster_key: String,
    pub sample_sender: Option<String>,
    pub sample_subject: Option<String>,
    pub member_count: i64,
    pub last_internal_date: i64,
}

pub struct ClustersRepo<'a> {
    pub db: &'a Db,
}

impl<'a> ClustersRepo<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub fn upsert_many(&self, account_id: &str, rows: &[ClusterMember]) -> DbResult<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        self.db.with_connection(|conn| {
            let tx = conn.transaction().map_err(DbError::from)?;
            let mut count = 0;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO message_clusters
                            (account_id, provider_msg_id, cluster_key, centroid_cosine)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(account_id, provider_msg_id) DO UPDATE SET
                            cluster_key     = excluded.cluster_key,
                            centroid_cosine = excluded.centroid_cosine,
                            assigned_at     = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                    )
                    .map_err(DbError::from)?;
                for r in rows {
                    stmt.execute(params![
                        account_id,
                        r.provider_msg_id,
                        r.cluster_key,
                        r.centroid_cosine,
                    ])
                    .map_err(DbError::from)?;
                    count += 1;
                }
            }
            tx.commit().map_err(DbError::from)?;
            Ok(count)
        })
    }

    /// One row per cluster_key, ordered by member_count desc.
    pub fn list_summaries(&self, account_id: &str) -> DbResult<Vec<ClusterSummary>> {
        self.db.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT
                        mc.cluster_key,
                        (SELECT m.sender FROM messages m
                          WHERE m.account_id = mc.account_id AND m.provider_msg_id = mc.provider_msg_id
                          LIMIT 1) AS sample_sender,
                        (SELECT m.subject FROM messages m
                          WHERE m.account_id = mc.account_id AND m.provider_msg_id = mc.provider_msg_id
                          LIMIT 1) AS sample_subject,
                        COUNT(*) AS member_count,
                        MAX((SELECT m.internal_date FROM messages m
                              WHERE m.account_id = mc.account_id
                                AND m.provider_msg_id = mc.provider_msg_id)) AS last_internal_date
                     FROM message_clusters mc
                     WHERE mc.account_id = ?1
                     GROUP BY mc.cluster_key
                     ORDER BY member_count DESC, mc.cluster_key ASC",
                )
                .map_err(DbError::from)?;
            let rows = stmt
                .query_map(params![account_id], |row| {
                    Ok(ClusterSummary {
                        cluster_key: row.get(0)?,
                        sample_sender: row.get(1)?,
                        sample_subject: row.get(2)?,
                        member_count: row.get(3)?,
                        last_internal_date: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
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

    pub fn count_clusters(&self, account_id: &str) -> DbResult<i64> {
        self.db.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(DISTINCT cluster_key) FROM message_clusters WHERE account_id = ?1",
                params![account_id],
                |r| r.get(0),
            )
            .map_err(DbError::from)
        })
    }
}
