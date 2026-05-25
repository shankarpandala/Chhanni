//! Per-cluster classification.
//!
//! For each cluster we pick three representative messages (member with the
//! highest centroid_cosine, and two more spread by internal_date), serialise
//! a tightly-bounded prompt, and ask the model for a JSON object matching
//! `category_schema()`. We send the schema along with the request so the
//! llama.cpp server constrains decoding to valid JSON.

use std::sync::Arc;
use std::time::Instant;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::db::{Category, ClassificationRow, ClassificationsRepo, Db};
use crate::error::{DbError, PipelineError, PipelineResult};
use crate::sidecar::CompletionClient;

pub const PROMPT_VERSION: &str = "v1";

/// Minimum confidence the model must report before we accept its label.
/// Anything lower is stored as `Unknown` per DECISIONS.md.
pub const CONFIDENCE_FLOOR: f32 = 0.6;

#[derive(Clone, Debug)]
pub struct ClassifyConfig {
    pub model_version: String,
    pub samples_per_cluster: usize,
    pub max_clusters: Option<usize>,
}

impl Default for ClassifyConfig {
    fn default() -> Self {
        Self {
            model_version: "Qwen3-4B-Instruct-2507/Q4_K_M".to_owned(),
            samples_per_cluster: 3,
            max_clusters: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ClassifyProgress {
    pub account_id: String,
    pub processed: u64,
    pub remaining: u64,
    pub skipped_unchanged: u64,
    pub elapsed_ms: u64,
}

pub type ClassifySink = Arc<dyn Fn(&ClassifyProgress) + Send + Sync>;

#[derive(Clone, Debug)]
struct ClusterSample {
    sender: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
}

#[derive(Clone, Debug)]
struct ClusterToClassify {
    cluster_key: String,
    samples: Vec<ClusterSample>,
    signature: String,
}

#[derive(Deserialize)]
struct ModelOutput {
    category: String,
    confidence: f32,
    #[serde(default)]
    reason: Option<String>,
}

pub async fn classify_account(
    client: Arc<dyn CompletionClient>,
    db: Db,
    account_id: &str,
    config: ClassifyConfig,
    on_progress: ClassifySink,
) -> PipelineResult<u64> {
    let started = Instant::now();
    let clusters = load_clusters(&db, account_id, &config)?;

    let total = clusters.len() as u64;
    let mut processed: u64 = 0;
    let mut skipped: u64 = 0;

    let schema = category_schema();

    for cluster in clusters {
        // Skip if classification exists with identical (signature, model, prompt).
        if let Some(existing) = ClassificationsRepo::new(&db)
            .get(account_id, &cluster.cluster_key)
            .map_err(PipelineError::Db)?
        {
            if existing.cluster_signature == cluster.signature
                && existing.model_version == config.model_version
                && existing.prompt_version == PROMPT_VERSION
            {
                skipped += 1;
                emit(&on_progress, account_id, processed, total - processed - skipped, skipped, started);
                continue;
            }
        }

        let prompt = render_prompt(&cluster);
        let raw = match client.complete_json(&prompt, &schema).await {
            Ok(r) => r,
            Err(e) => {
                warn!(cluster_key = %cluster.cluster_key, error = %e, "classification request failed; marking unknown");
                store_unknown(&db, account_id, &cluster, &config, format!("error: {e}"))?;
                processed += 1;
                emit(&on_progress, account_id, processed, total - processed - skipped, skipped, started);
                continue;
            }
        };

        let row = match parse_and_normalise(&raw) {
            Ok((category, confidence, reason)) => ClassificationRow {
                cluster_key: cluster.cluster_key.clone(),
                category,
                confidence,
                reason,
                model_version: config.model_version.clone(),
                prompt_version: PROMPT_VERSION.to_owned(),
                cluster_signature: cluster.signature.clone(),
            },
            Err(e) => {
                warn!(cluster_key = %cluster.cluster_key, error = %e, "model output failed to parse; marking unknown");
                store_unknown(&db, account_id, &cluster, &config, format!("parse: {e}"))?;
                processed += 1;
                emit(&on_progress, account_id, processed, total - processed - skipped, skipped, started);
                continue;
            }
        };

        ClassificationsRepo::new(&db)
            .upsert_and_propagate(account_id, &row)
            .map_err(PipelineError::Db)?;
        processed += 1;
        emit(&on_progress, account_id, processed, total - processed - skipped, skipped, started);
    }

    info!(account_id, processed, skipped, "classification pass complete");
    Ok(processed)
}

fn emit(
    sink: &ClassifySink,
    account_id: &str,
    processed: u64,
    remaining: u64,
    skipped: u64,
    started: Instant,
) {
    sink(&ClassifyProgress {
        account_id: account_id.to_owned(),
        processed,
        remaining,
        skipped_unchanged: skipped,
        elapsed_ms: started.elapsed().as_millis() as u64,
    });
}

fn store_unknown(
    db: &Db,
    account_id: &str,
    cluster: &ClusterToClassify,
    config: &ClassifyConfig,
    reason: String,
) -> PipelineResult<()> {
    ClassificationsRepo::new(db)
        .upsert_and_propagate(
            account_id,
            &ClassificationRow {
                cluster_key: cluster.cluster_key.clone(),
                category: Category::Unknown,
                confidence: 0.0,
                reason: Some(reason),
                model_version: config.model_version.clone(),
                prompt_version: PROMPT_VERSION.to_owned(),
                cluster_signature: cluster.signature.clone(),
            },
        )
        .map_err(PipelineError::Db)
}

fn parse_and_normalise(raw: &str) -> Result<(Category, f32, Option<String>), String> {
    // Strip code-fence noise that some models still emit despite a schema
    // constraint, then parse.
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let parsed: ModelOutput =
        serde_json::from_str(trimmed).map_err(|e| format!("invalid json: {e}"))?;
    let category = Category::parse(&parsed.category.to_ascii_lowercase());
    let confidence = parsed.confidence.clamp(0.0, 1.0);
    if confidence < CONFIDENCE_FLOOR {
        Ok((Category::Unknown, confidence, parsed.reason))
    } else {
        Ok((category, confidence, parsed.reason))
    }
}

type Row = (String, String, Option<String>, Option<String>, Option<String>, Option<f32>, i64);
type Member = (String, Option<String>, Option<String>, Option<String>, Option<f32>, i64);

fn load_clusters(
    db: &Db,
    account_id: &str,
    config: &ClassifyConfig,
) -> PipelineResult<Vec<ClusterToClassify>> {
    // Pull all (cluster_key, member) pairs and group them in Rust. At our
    // scale (≤ a few hundred clusters) this is trivial.

    let rows: Vec<Row> = db
        .with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT
                        mc.cluster_key,
                        m.provider_msg_id,
                        m.sender,
                        m.subject,
                        m.snippet,
                        mc.centroid_cosine,
                        m.internal_date
                     FROM message_clusters mc
                     JOIN messages m
                       ON m.account_id = mc.account_id AND m.provider_msg_id = mc.provider_msg_id
                     WHERE mc.account_id = ?1
                     ORDER BY mc.cluster_key, m.internal_date ASC",
                )
                .map_err(DbError::from)?;
            let rows = stmt
                .query_map(params![account_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<f32>>(5)?,
                        r.get::<_, i64>(6)?,
                    ))
                })
                .map_err(DbError::from)?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r.map_err(DbError::from)?);
            }
            Ok(v)
        })
        .map_err(PipelineError::Db)?;

    // Group consecutively (we ORDER BY cluster_key) and pick samples.
    let mut out = Vec::new();
    let mut current_key: Option<String> = None;
    let mut current: Vec<Member> = Vec::new();
    for (key, mid, sender, subject, snippet, centroid, date) in rows {
        if current_key.as_deref() != Some(key.as_str()) {
            if let Some(prev_key) = current_key.take() {
                out.push(build_cluster(prev_key, std::mem::take(&mut current), config));
            }
            current_key = Some(key.clone());
        }
        current.push((mid, sender, subject, snippet, centroid, date));
    }
    if let Some(prev_key) = current_key.take() {
        out.push(build_cluster(prev_key, current, config));
    }

    if let Some(cap) = config.max_clusters {
        out.truncate(cap);
    }
    Ok(out)
}

fn build_cluster(
    key: String,
    mut members: Vec<Member>,
    config: &ClassifyConfig,
) -> ClusterToClassify {
    // Sort by centroid_cosine desc, then take the top N as representative.
    // Tie-break by internal_date desc for newer-first.
    members.sort_by(|a, b| {
        b.4.unwrap_or(0.0)
            .partial_cmp(&a.4.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.5.cmp(&a.5))
    });
    let samples: Vec<ClusterSample> = members
        .iter()
        .take(config.samples_per_cluster.max(1))
        .map(|m| ClusterSample {
            sender: m.1.clone(),
            subject: m.2.clone(),
            snippet: m.3.clone(),
        })
        .collect();

    // Signature = stable hash of (cluster_key, all member ids in sorted order,
    // member_count). Any membership change forces re-classification.
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hasher.update(b"|");
    let mut ids: Vec<&str> = members.iter().map(|m| m.0.as_str()).collect();
    ids.sort_unstable();
    for id in &ids {
        hasher.update(id.as_bytes());
        hasher.update(b",");
    }
    let signature = hex::encode(hasher.finalize());

    ClusterToClassify {
        cluster_key: key,
        samples,
        signature,
    }
}

fn render_prompt(cluster: &ClusterToClassify) -> String {
    let mut s = String::with_capacity(1024);
    s.push_str(
        "You are an email triage assistant. Classify the cluster of emails below \
         into exactly ONE category. Categories:\n\
         - transactional: receipts, orders, shipping, account changes\n\
         - newsletter: regularly-scheduled editorial content the user subscribed to\n\
         - social: notifications from social networks\n\
         - personal: written one-to-one by a person to the user\n\
         - work: business correspondence, meetings, work tools\n\
         - security: 2FA codes, account-security alerts, login notices\n\
         - promotional: sales, deals, marketing blasts\n\
         - notification: app/service notifications that aren't transactional or security\n\
         - unknown: if the cluster could plausibly fit more than one category\n\n\
         Reply with JSON only. Use the schema. `confidence` is your subjective \
         probability (0.0-1.0). `reason` is a short phrase, max 80 characters.\n\n\
         Cluster samples:\n",
    );
    for (i, sample) in cluster.samples.iter().enumerate() {
        s.push_str(&format!("\n--- sample {} ---\n", i + 1));
        if let Some(from) = &sample.sender {
            s.push_str(&format!("From: {}\n", truncate(from, 200)));
        }
        if let Some(subj) = &sample.subject {
            s.push_str(&format!("Subject: {}\n", truncate(subj, 200)));
        }
        if let Some(snip) = &sample.snippet {
            s.push_str(&format!("Snippet: {}\n", truncate(snip, 400)));
        }
    }
    s.push_str("\nJSON:\n");
    s
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// JSON-Schema for the classifier output. Passed to llama.cpp's
/// `/completion` endpoint as `json_schema` so the model can only emit
/// tokens that keep the partial JSON valid.
pub fn category_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "category": {
                "type": "string",
                "enum": [
                    "transactional", "newsletter", "social", "personal",
                    "work", "security", "promotional", "notification", "unknown"
                ]
            },
            "confidence": {
                "type": "number",
                "minimum": 0.0,
                "maximum": 1.0
            },
            "reason": {
                "type": "string",
                "maxLength": 120
            }
        },
        "required": ["category", "confidence"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        open_in_memory, ClusterMember, ClustersRepo, MessageRow, MessagesRepo,
    };
    use async_trait::async_trait;
    use crate::error::SidecarResult;
    use parking_lot::Mutex;

    struct ScriptedCompleter {
        responses: Mutex<Vec<String>>,
        calls: Mutex<u32>,
    }

    #[async_trait]
    impl CompletionClient for ScriptedCompleter {
        async fn complete_json(
            &self,
            _prompt: &str,
            _schema: &serde_json::Value,
        ) -> SidecarResult<String> {
            *self.calls.lock() += 1;
            let mut v = self.responses.lock();
            if v.is_empty() {
                return Err(crate::error::SidecarError::Request("no more scripted responses".into()));
            }
            Ok(v.remove(0))
        }
    }

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

    fn cluster_of(db: &Db, key: &str, members: &[(&str, &str, &str, &str)]) {
        let mr = MessagesRepo::new(db);
        let cr = ClustersRepo::new(db);
        let mut cluster_members = Vec::new();
        for (id, sender, subject, snippet) in members {
            mr.upsert(&MessageRow {
                account_id: "a1".into(),
                provider_msg_id: (*id).into(),
                thread_id: format!("t-{id}"),
                sender: Some((*sender).into()),
                sender_email: Some((*sender).into()),
                subject: Some((*subject).into()),
                snippet: Some((*snippet).into()),
                internal_date: 0,
                label_ids: vec![],
                history_id: None,
            })
            .unwrap();
            cluster_members.push(ClusterMember {
                provider_msg_id: (*id).into(),
                cluster_key: key.into(),
                centroid_cosine: Some(1.0),
            });
        }
        cr.upsert_many("a1", &cluster_members).unwrap();
    }

    fn noop_sink() -> ClassifySink {
        Arc::new(|_p| {})
    }

    #[tokio::test(flavor = "current_thread")]
    async fn classifies_clusters_and_propagates() {
        let db = db();
        cluster_of(
            &db,
            "sender::news@example.com",
            &[("m1", "news@example.com", "Weekly Digest", "Stories from this week")],
        );
        cluster_of(
            &db,
            "sender::store@example.com",
            &[("m2", "store@example.com", "Your order shipped", "tracking 123")],
        );

        let completer = Arc::new(ScriptedCompleter {
            responses: Mutex::new(vec![
                r#"{"category":"newsletter","confidence":0.92,"reason":"weekly digest"}"#.to_owned(),
                r#"{"category":"transactional","confidence":0.97,"reason":"shipping"}"#.to_owned(),
            ]),
            calls: Mutex::new(0),
        });
        let n = classify_account(
            completer.clone(),
            db.clone(),
            "a1",
            ClassifyConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();
        assert_eq!(n, 2);
        assert_eq!(*completer.calls.lock(), 2);

        // Both clusters classified; categories propagated to messages.
        db.with_connection(|c| {
            let cat: String = c
                .query_row(
                    "SELECT category FROM messages WHERE provider_msg_id='m1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(cat, "newsletter");
            let cat: String = c
                .query_row(
                    "SELECT category FROM messages WHERE provider_msg_id='m2'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(cat, "transactional");
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rerun_skips_unchanged_clusters() {
        let db = db();
        cluster_of(
            &db,
            "sender::a@example.com",
            &[("m1", "a@example.com", "subj", "body")],
        );
        let completer = Arc::new(ScriptedCompleter {
            responses: Mutex::new(vec![
                r#"{"category":"newsletter","confidence":0.91}"#.to_owned(),
            ]),
            calls: Mutex::new(0),
        });
        classify_account(
            completer.clone(),
            db.clone(),
            "a1",
            ClassifyConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();
        assert_eq!(*completer.calls.lock(), 1);

        // Second run with no scripted responses should NOT call the model.
        let n = classify_account(
            completer.clone(),
            db.clone(),
            "a1",
            ClassifyConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();
        assert_eq!(n, 0, "no clusters should be re-classified");
        assert_eq!(*completer.calls.lock(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn low_confidence_collapses_to_unknown() {
        let db = db();
        cluster_of(
            &db,
            "sender::a@example.com",
            &[("m1", "a@example.com", "subj", "body")],
        );
        let completer = Arc::new(ScriptedCompleter {
            responses: Mutex::new(vec![
                r#"{"category":"newsletter","confidence":0.42}"#.to_owned(),
            ]),
            calls: Mutex::new(0),
        });
        classify_account(
            completer,
            db.clone(),
            "a1",
            ClassifyConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();
        let cat: String = db
            .with_connection(|c| {
                c.query_row(
                    "SELECT category FROM messages WHERE provider_msg_id='m1'",
                    [],
                    |r| r.get(0),
                )
                .map_err(DbError::from)
            })
            .unwrap();
        assert_eq!(cat, "unknown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn malformed_json_falls_back_to_unknown_without_error() {
        let db = db();
        cluster_of(
            &db,
            "sender::a@example.com",
            &[("m1", "a@example.com", "subj", "body")],
        );
        let completer = Arc::new(ScriptedCompleter {
            responses: Mutex::new(vec!["not json at all".to_owned()]),
            calls: Mutex::new(0),
        });
        let n = classify_account(
            completer,
            db.clone(),
            "a1",
            ClassifyConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();
        assert_eq!(n, 1);
        let cat: String = db
            .with_connection(|c| {
                c.query_row(
                    "SELECT category FROM messages WHERE provider_msg_id='m1'",
                    [],
                    |r| r.get(0),
                )
                .map_err(DbError::from)
            })
            .unwrap();
        assert_eq!(cat, "unknown");
    }

    #[test]
    fn schema_includes_all_categories() {
        let schema = category_schema();
        let cats = schema
            .pointer("/properties/category/enum")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(cats.len(), 9);
    }

    #[test]
    fn parse_handles_code_fences() {
        let raw = "```json\n{\"category\":\"work\",\"confidence\":0.8}\n```";
        let (c, conf, _) = parse_and_normalise(raw).unwrap();
        assert_eq!(c, Category::Work);
        assert!((conf - 0.8).abs() < 1e-3);
    }
}
