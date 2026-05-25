//! Two-stage clustering.
//!
//! Stage 1 — bucket every message by its `sender_email`. Senders are the
//! single strongest signal for "this is the same kind of mail" (newsletter
//! lists, no-reply addresses, vendor notifications). Messages whose
//! sender_email is null fall into a single "unknown" bucket.
//!
//! Stage 2 — within each sender bucket, greedy agglomerative clustering on
//! cosine similarity to running centroids. Threshold is configurable; default
//! 0.85 per SPEC.md.

use std::collections::BTreeMap;

use rusqlite::params;
use tracing::info;
use uuid::Uuid;

use crate::db::{ClusterMember, ClustersRepo, Db};
use crate::error::{DbError, PipelineError, PipelineResult};

type SenderBuckets = BTreeMap<String, Vec<(String, Vec<f32>)>>;

#[derive(Clone, Debug)]
pub struct ClusterConfig {
    pub similarity_threshold: f32,
    /// Below this size, a sender bucket becomes a single cluster regardless
    /// of within-group similarity (saves embeddings work and produces tidier
    /// summaries for tiny senders).
    pub min_split_size: usize,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.85,
            min_split_size: 4,
        }
    }
}

pub fn cluster_account(db: &Db, account_id: &str, config: ClusterConfig) -> PipelineResult<u64> {
    let buckets = load_buckets(db, account_id)?;
    let mut assignments: Vec<ClusterMember> = Vec::new();
    let mut clusters_created = 0u64;

    for (sender_key, items) in buckets {
        if items.len() < config.min_split_size {
            // One cluster for the whole bucket.
            let cluster_key = canonical_key(&sender_key);
            for (id, _vec) in items {
                assignments.push(ClusterMember {
                    provider_msg_id: id,
                    cluster_key: cluster_key.clone(),
                    centroid_cosine: Some(1.0),
                });
            }
            clusters_created += 1;
            continue;
        }

        let assigned = greedy_cluster(&items, config.similarity_threshold);
        let mut local_cluster_count = 0;
        for assignment in assigned {
            let cluster_key = if assignment.local_index == 0 {
                canonical_key(&sender_key)
            } else {
                format!("{}#{}", canonical_key(&sender_key), assignment.local_index)
            };
            local_cluster_count = local_cluster_count.max(assignment.local_index + 1);
            assignments.push(ClusterMember {
                provider_msg_id: assignment.provider_msg_id,
                cluster_key,
                centroid_cosine: Some(assignment.centroid_cosine),
            });
        }
        clusters_created += local_cluster_count as u64;
    }

    let written = ClustersRepo::new(db)
        .upsert_many(account_id, &assignments)
        .map_err(PipelineError::Db)?;
    info!(account_id, clusters = clusters_created, members = written, "clustering complete");
    Ok(clusters_created)
}

fn canonical_key(sender_key: &str) -> String {
    if sender_key.is_empty() {
        // Synthesize a stable id for unknown-sender clusters so they don't
        // collide across accounts.
        format!("unknown::{}", Uuid::new_v4())
    } else {
        format!("sender::{sender_key}")
    }
}

struct LocalAssignment {
    provider_msg_id: String,
    local_index: usize,
    centroid_cosine: f32,
}

fn greedy_cluster(items: &[(String, Vec<f32>)], threshold: f32) -> Vec<LocalAssignment> {
    let mut centroids: Vec<(Vec<f32>, usize)> = Vec::new(); // (centroid, member_count)
    let mut out = Vec::with_capacity(items.len());

    for (id, vec) in items {
        let mut best: Option<(usize, f32)> = None;
        for (idx, (centroid, _)) in centroids.iter().enumerate() {
            let sim = cosine(centroid, vec);
            if best.map(|(_, s)| sim > s).unwrap_or(true) {
                best = Some((idx, sim));
            }
        }
        match best {
            Some((idx, sim)) if sim >= threshold => {
                // Update centroid as running mean.
                let (centroid, count) = &mut centroids[idx];
                *count += 1;
                let n = *count as f32;
                for (c, v) in centroid.iter_mut().zip(vec.iter()) {
                    *c = ((*c) * (n - 1.0) + v) / n;
                }
                out.push(LocalAssignment {
                    provider_msg_id: id.clone(),
                    local_index: idx,
                    centroid_cosine: sim,
                });
            }
            _ => {
                centroids.push((vec.clone(), 1));
                out.push(LocalAssignment {
                    provider_msg_id: id.clone(),
                    local_index: centroids.len() - 1,
                    centroid_cosine: 1.0,
                });
            }
        }
    }
    out
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom < 1e-9 {
        0.0
    } else {
        dot / denom
    }
}

fn load_buckets(db: &Db, account_id: &str) -> PipelineResult<SenderBuckets> {
    db.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT m.provider_msg_id, COALESCE(m.sender_email, ''), e.embedding
                 FROM messages m
                 JOIN embeddings e
                   ON e.account_id = m.account_id AND e.provider_msg_id = m.provider_msg_id
                 WHERE m.account_id = ?1
                 ORDER BY m.internal_date ASC",
            )
            .map_err(DbError::from)?;
        let rows = stmt
            .query_map(params![account_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(DbError::from)?;
        let mut buckets: BTreeMap<String, Vec<(String, Vec<f32>)>> = BTreeMap::new();
        for r in rows {
            let (id, sender, bytes) = r.map_err(DbError::from)?;
            let mut v = Vec::with_capacity(bytes.len() / 4);
            for chunk in bytes.chunks_exact(4) {
                let arr = [chunk[0], chunk[1], chunk[2], chunk[3]];
                v.push(f32::from_le_bytes(arr));
            }
            buckets.entry(sender).or_default().push((id, v));
        }
        Ok(buckets)
    })
    .map_err(PipelineError::Db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_in_memory, EmbeddingRow, EmbeddingsRepo, MessageRow, MessagesRepo};

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

    fn insert(db: &Db, id: &str, sender: &str, embedding: Vec<f32>) {
        MessagesRepo::new(db)
            .upsert(&MessageRow {
                account_id: "a1".into(),
                provider_msg_id: id.into(),
                thread_id: format!("t-{id}"),
                sender: Some(format!("X <{sender}>")),
                sender_email: Some(sender.into()),
                subject: Some(format!("s-{id}")),
                snippet: None,
                internal_date: 0,
                label_ids: vec![],
                history_id: None,
            })
            .unwrap();
        EmbeddingsRepo::new(db)
            .upsert_many(
                "a1",
                "test",
                &[EmbeddingRow {
                    provider_msg_id: id.into(),
                    embedding,
                }],
            )
            .unwrap();
    }

    #[test]
    fn cosine_is_one_for_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_is_zero_for_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn same_sender_small_bucket_becomes_one_cluster() {
        let db = db();
        insert(&db, "m1", "news@example.com", vec![1.0, 0.0, 0.0]);
        insert(&db, "m2", "news@example.com", vec![0.99, 0.01, 0.0]);
        let n = cluster_account(&db, "a1", ClusterConfig::default()).unwrap();
        assert_eq!(n, 1);
        let summaries = ClustersRepo::new(&db).list_summaries("a1").unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].member_count, 2);
    }

    #[test]
    fn dissimilar_messages_within_large_sender_split() {
        let db = db();
        // 4 messages from the same sender: two clearly similar, two clearly
        // different, forming two clusters within the bucket.
        insert(&db, "m1", "bulk@example.com", vec![1.0, 0.0, 0.0]);
        insert(&db, "m2", "bulk@example.com", vec![0.99, 0.01, 0.0]);
        insert(&db, "m3", "bulk@example.com", vec![0.0, 1.0, 0.0]);
        insert(&db, "m4", "bulk@example.com", vec![0.01, 0.99, 0.0]);
        let n = cluster_account(
            &db,
            "a1",
            ClusterConfig {
                similarity_threshold: 0.85,
                min_split_size: 3,
            },
        )
        .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn different_senders_never_share_a_cluster() {
        let db = db();
        insert(&db, "m1", "a@example.com", vec![1.0, 0.0]);
        insert(&db, "m2", "b@example.com", vec![1.0, 0.0]);
        cluster_account(&db, "a1", ClusterConfig::default()).unwrap();
        let summaries = ClustersRepo::new(&db).list_summaries("a1").unwrap();
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn unknown_sender_gets_its_own_synthetic_key() {
        let db = db();
        MessagesRepo::new(&db)
            .upsert(&MessageRow {
                account_id: "a1".into(),
                provider_msg_id: "m1".into(),
                thread_id: "t1".into(),
                sender: None,
                sender_email: None,
                subject: None,
                snippet: None,
                internal_date: 0,
                label_ids: vec![],
                history_id: None,
            })
            .unwrap();
        EmbeddingsRepo::new(&db)
            .upsert_many(
                "a1",
                "v",
                &[EmbeddingRow {
                    provider_msg_id: "m1".into(),
                    embedding: vec![1.0, 0.0],
                }],
            )
            .unwrap();
        cluster_account(&db, "a1", ClusterConfig::default()).unwrap();
        let summaries = ClustersRepo::new(&db).list_summaries("a1").unwrap();
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].cluster_key.starts_with("unknown::"));
    }
}
