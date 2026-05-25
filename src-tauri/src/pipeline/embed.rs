use std::sync::Arc;
use std::time::Instant;

use futures::stream::{FuturesUnordered, StreamExt};
use rusqlite::params_from_iter;
use serde::Serialize;
use tracing::info;

use crate::db::{Db, EmbeddingRow, EmbeddingsRepo};
use crate::error::{DbError, PipelineError, PipelineResult};
use crate::pipeline::text::build_embedding_input;
use crate::sidecar::EmbeddingClient;

#[derive(Clone, Debug)]
pub struct EmbedConfig {
    pub model_version: String,
    pub concurrency: usize,
    pub batch_persist: usize,
    pub expected_dim: Option<usize>,
}

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            model_version: "Qwen3-Embedding-0.6B/Q8_0".to_owned(),
            concurrency: 8,
            batch_persist: 64,
            // Qwen3-Embedding-0.6B emits 1024-dim by default. Matryoshka-capable
            // so callers can request shorter via API; we don't currently.
            expected_dim: Some(1024),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EmbedProgress {
    pub account_id: String,
    pub processed: u64,
    pub remaining: u64,
    pub elapsed_ms: u64,
}

pub type EmbedSink = Arc<dyn Fn(&EmbedProgress) + Send + Sync>;

/// Embed every message in `account_id` that doesn't yet have a row in
/// `embeddings`. Idempotent: re-running picks up only what's missing.
pub async fn embed_account(
    client: Arc<dyn EmbeddingClient>,
    db: Db,
    account_id: &str,
    config: EmbedConfig,
    on_progress: EmbedSink,
) -> PipelineResult<u64> {
    let started = Instant::now();
    let mut processed = 0u64;

    loop {
        let pending = EmbeddingsRepo::new(&db)
            .list_missing(account_id, config.batch_persist.max(64))
            .map_err(PipelineError::Db)?;
        if pending.is_empty() {
            on_progress(&EmbedProgress {
                account_id: account_id.to_owned(),
                processed,
                remaining: 0,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
            break;
        }

        // Look up texts for the pending IDs.
        let texts = fetch_inputs(&db, account_id, &pending)?;

        // Drive `concurrency` requests in flight.
        let mut futs = FuturesUnordered::new();
        let mut iter = texts.into_iter();
        let mut completed: Vec<EmbeddingRow> = Vec::new();
        for (id, text) in iter.by_ref().take(config.concurrency) {
            let client = client.clone();
            futs.push(embed_one(client, id, text));
        }
        while let Some(res) = futs.next().await {
            let (id, vec) = res?;
            if let Some(expected) = config.expected_dim {
                if vec.len() != expected {
                    return Err(PipelineError::DimensionMismatch {
                        expected,
                        actual: vec.len(),
                    });
                }
            }
            completed.push(EmbeddingRow {
                provider_msg_id: id,
                embedding: vec,
            });
            processed += 1;
            if completed.len() >= config.batch_persist {
                EmbeddingsRepo::new(&db)
                    .upsert_many(account_id, &config.model_version, &completed)
                    .map_err(PipelineError::Db)?;
                completed.clear();
                on_progress(&EmbedProgress {
                    account_id: account_id.to_owned(),
                    processed,
                    remaining: 0, // we report exact remaining in next loop iteration
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
            // Top up the pipeline.
            if let Some((id, text)) = iter.next() {
                let client = client.clone();
                futs.push(embed_one(client, id, text));
            }
        }
        if !completed.is_empty() {
            EmbeddingsRepo::new(&db)
                .upsert_many(account_id, &config.model_version, &completed)
                .map_err(PipelineError::Db)?;
        }
    }
    info!(account_id, processed, "embedding pass complete");
    Ok(processed)
}

async fn embed_one(
    client: Arc<dyn EmbeddingClient>,
    id: String,
    text: String,
) -> PipelineResult<(String, Vec<f32>)> {
    let vec = client.embed(&text).await.map_err(PipelineError::Sidecar)?;
    Ok((id, vec))
}

fn fetch_inputs(db: &Db, account_id: &str, ids: &[String]) -> PipelineResult<Vec<(String, String)>> {
    db.with_connection(|conn| {
        // Build a parameterized IN list. We cap at 256 ids per call which
        // matches our batch size comfortably.
        let mut sql = String::from(
            "SELECT provider_msg_id, subject, sender, snippet FROM messages
             WHERE account_id = ?1 AND provider_msg_id IN (",
        );
        for i in 0..ids.len() {
            if i > 0 {
                sql.push(',');
            }
            sql.push_str(&format!("?{}", i + 2));
        }
        sql.push(')');
        let mut stmt = conn.prepare(&sql).map_err(DbError::from)?;
        let mut bound: Vec<String> = Vec::with_capacity(ids.len() + 1);
        bound.push(account_id.to_owned());
        for id in ids {
            bound.push(id.clone());
        }
        let rows = stmt
            .query_map(params_from_iter(bound.iter()), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(DbError::from)?;
        let mut out = Vec::new();
        for r in rows {
            let (id, subject, sender, snippet) = r.map_err(DbError::from)?;
            let text = build_embedding_input(subject.as_deref(), sender.as_deref(), snippet.as_deref());
            out.push((id, text));
        }
        Ok(out)
    })
    .map_err(PipelineError::Db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_in_memory, MessageRow, MessagesRepo};
    use async_trait::async_trait;
    use crate::error::SidecarResult;

    struct DeterministicEmbedder {
        dim: usize,
    }

    #[async_trait]
    impl EmbeddingClient for DeterministicEmbedder {
        async fn embed(&self, text: &str) -> SidecarResult<Vec<f32>> {
            // Hash the input into a stable but content-distinguishing vector.
            let mut v = vec![0.0f32; self.dim];
            for (i, b) in text.bytes().enumerate() {
                v[i % self.dim] += (b as f32) / 255.0;
            }
            // L2 normalise.
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
            for x in &mut v {
                *x /= norm;
            }
            Ok(v)
        }
    }

    fn db_with_messages(ids: &[&str]) -> Db {
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
        let mr = MessagesRepo::new(&db);
        for id in ids {
            mr.upsert(&MessageRow {
                account_id: "a1".into(),
                provider_msg_id: (*id).into(),
                thread_id: format!("t-{id}"),
                sender: Some(format!("sender-{id}@example.com")),
                sender_email: Some(format!("sender-{id}@example.com")),
                subject: Some(format!("subj-{id}")),
                snippet: Some(format!("body of {id}")),
                internal_date: 0,
                label_ids: vec![],
                history_id: None,
            })
            .unwrap();
        }
        db
    }

    fn noop_sink() -> EmbedSink {
        Arc::new(|_p| {})
    }

    #[tokio::test(flavor = "current_thread")]
    async fn embeds_all_missing_messages() {
        let db = db_with_messages(&["m1", "m2", "m3"]);
        let client: Arc<dyn EmbeddingClient> = Arc::new(DeterministicEmbedder { dim: 16 });
        let cfg = EmbedConfig {
            model_version: "test".into(),
            concurrency: 2,
            batch_persist: 2,
            expected_dim: Some(16),
        };
        let n = embed_account(client, db.clone(), "a1", cfg, noop_sink())
            .await
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(EmbeddingsRepo::new(&db).count_for_account("a1").unwrap(), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rerun_is_a_noop() {
        let db = db_with_messages(&["m1", "m2"]);
        let client: Arc<dyn EmbeddingClient> = Arc::new(DeterministicEmbedder { dim: 8 });
        let cfg = EmbedConfig {
            model_version: "v".into(),
            concurrency: 2,
            batch_persist: 2,
            expected_dim: Some(8),
        };
        let first = embed_account(client.clone(), db.clone(), "a1", cfg.clone(), noop_sink())
            .await
            .unwrap();
        let second = embed_account(client, db.clone(), "a1", cfg, noop_sink())
            .await
            .unwrap();
        assert_eq!(first, 2);
        assert_eq!(second, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dimension_mismatch_surfaces_error() {
        let db = db_with_messages(&["m1"]);
        let client: Arc<dyn EmbeddingClient> = Arc::new(DeterministicEmbedder { dim: 4 });
        let cfg = EmbedConfig {
            model_version: "v".into(),
            concurrency: 1,
            batch_persist: 1,
            expected_dim: Some(768),
        };
        let err = embed_account(client, db, "a1", cfg, noop_sink())
            .await
            .unwrap_err();
        assert!(matches!(err, PipelineError::DimensionMismatch { .. }));
    }
}
