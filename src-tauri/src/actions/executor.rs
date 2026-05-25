//! Action executor: drains `staged_actions` against a `GmailMutations` impl
//! in 1,000-message batches with cancellation support.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use rusqlite::params;
use serde::Serialize;
use tokio::sync::Notify;
use tracing::{info, warn};

use crate::actions::log::{ActionsLogRepo, Outcome};
use crate::actions::rules::ActionType;
use crate::actions::staged::StagedActionsRepo;
use crate::db::Db;
use crate::error::{DbError, SyncError, SyncResult};
use crate::providers::gmail::GmailMutations;

/// Default Gmail batch size cap.
pub const BATCH_SIZE: usize = 1000;

#[derive(Clone, Debug)]
pub struct ExecuteConfig {
    pub batch_size: usize,
}

impl Default for ExecuteConfig {
    fn default() -> Self {
        Self { batch_size: BATCH_SIZE }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecuteProgress {
    pub account_id: String,
    pub batches_done: u32,
    pub messages_done: u64,
    pub failures: u64,
    pub elapsed_ms: u64,
}

pub type ExecuteSink = Arc<dyn Fn(&ExecuteProgress) + Send + Sync>;

/// Cooperative cancellation token. Set the `Notify` from the UI command;
/// every batch boundary checks it.
#[derive(Clone, Default)]
pub struct CancellationToken {
    notify: Arc<Notify>,
    flag: Arc<std::sync::atomic::AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Group staged-action rows into work items keyed by (cluster, action_type)
/// expanded against current cluster membership.
#[derive(Clone, Debug)]
struct WorkItem {
    staged_id: i64,
    cluster_key: String,
    action: ActionType,
    target_ids: Vec<String>,
}

pub async fn execute_account(
    mutator: Arc<dyn GmailMutations>,
    db: Db,
    account_id: &str,
    config: ExecuteConfig,
    cancel: CancellationToken,
    on_progress: ExecuteSink,
) -> SyncResult<()> {
    let started = Instant::now();
    let work = collect_work(&db, account_id)?;
    let total_messages: u64 = work.iter().map(|w| w.target_ids.len() as u64).sum();
    info!(account_id, items = work.len(), total_messages, "executing staged actions");

    let mut messages_done: u64 = 0;
    let mut failures: u64 = 0;
    let mut batches_done: u32 = 0;

    for item in work {
        if cancel.is_cancelled() {
            warn!(account_id, "cancellation requested; stopping executor");
            return Err(SyncError::Cancelled);
        }
        for chunk in item.target_ids.chunks(config.batch_size) {
            if cancel.is_cancelled() {
                ActionsLogRepo::new(&db).record_many(
                    account_id,
                    Some(item.staged_id),
                    &item.cluster_key,
                    item.action.as_str(),
                    chunk,
                    Outcome::Cancelled,
                    None,
                )?;
                return Err(SyncError::Cancelled);
            }
            let chunk_owned: Vec<String> = chunk.to_vec();
            // Snapshot the prior label_ids per message BEFORE the mutation so
            // undo can reverse without an extra round-trip.
            let prior = snapshot_labels(&db, account_id, &chunk_owned)?;
            match dispatch(&*mutator, item.action, &chunk_owned).await {
                Ok(()) => {
                    ActionsLogRepo::new(&db).record_many_with_prior(
                        account_id,
                        Some(item.staged_id),
                        &item.cluster_key,
                        item.action.as_str(),
                        &chunk_owned,
                        Outcome::Success,
                        None,
                        &prior,
                    )?;
                    messages_done += chunk_owned.len() as u64;
                }
                Err(e) => {
                    let msg = e.to_string();
                    warn!(
                        account_id,
                        cluster_key = %item.cluster_key,
                        action = %item.action.as_str(),
                        error = %msg,
                        "batch failed"
                    );
                    ActionsLogRepo::new(&db).record_many(
                        account_id,
                        Some(item.staged_id),
                        &item.cluster_key,
                        item.action.as_str(),
                        &chunk_owned,
                        Outcome::Failure,
                        Some(&msg),
                    )?;
                    failures += chunk_owned.len() as u64;
                }
            }
            batches_done += 1;
            on_progress(&ExecuteProgress {
                account_id: account_id.to_owned(),
                batches_done,
                messages_done,
                failures,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
        // Remove the staged row once all its chunks have been processed.
        StagedActionsRepo::new(&db).unstage_by_id(item.staged_id)?;
    }

    info!(account_id, messages_done, failures, "executor complete");
    Ok(())
}

async fn dispatch(
    mutator: &dyn GmailMutations,
    action: ActionType,
    ids: &[String],
) -> SyncResult<()> {
    match action {
        ActionType::Archive => mutator.batch_archive(ids).await,
        ActionType::Trash => mutator.batch_trash(ids).await,
        ActionType::MarkRead => mutator.batch_mark_read(ids).await,
        ActionType::AddLabel | ActionType::RemoveLabel | ActionType::Unsubscribe => {
            // These need extra args (which label, which one-click URL) and
            // aren't exposed by the simple per-action chips in Phase 5.
            Err(SyncError::GmailStatus {
                status: 0,
                body: format!("action {} requires phase-7 features", action.as_str()),
            })
        }
    }
}

fn collect_work(db: &Db, account_id: &str) -> SyncResult<Vec<WorkItem>> {
    // Returns rows: (staged_id, cluster_key, action_type, provider_msg_id).
    // provider_msg_id NULL means "all current cluster members".
    let raw = db.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, cluster_key, action_type, provider_msg_id
                 FROM staged_actions
                 WHERE account_id = ?1
                 ORDER BY staged_at ASC, id ASC",
            )
            .map_err(DbError::from)?;
        let rows = stmt
            .query_map(params![account_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(DbError::from)?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r.map_err(DbError::from)?);
        }
        Ok(v)
    })?;

    let mut grouped: HashMap<(i64, String, String), Vec<String>> = HashMap::new();
    let mut cluster_members_cache: HashMap<String, Vec<String>> = HashMap::new();

    for (staged_id, cluster_key, action_str, provider_msg_id) in raw {
        let ids = match provider_msg_id {
            Some(id) => vec![id],
            None => {
                if let Some(cached) = cluster_members_cache.get(&cluster_key) {
                    cached.clone()
                } else {
                    let members = fetch_members(db, account_id, &cluster_key)?;
                    cluster_members_cache.insert(cluster_key.clone(), members.clone());
                    members
                }
            }
        };
        grouped
            .entry((staged_id, cluster_key, action_str))
            .or_default()
            .extend(ids);
    }

    let mut out = Vec::new();
    for ((staged_id, cluster_key, action_str), mut ids) in grouped {
        // Dedup ids in case both a cluster-level and a per-message stage
        // covered the same message.
        ids.sort();
        ids.dedup();
        if let Some(action) = parse_action(&action_str) {
            out.push(WorkItem {
                staged_id,
                cluster_key,
                action,
                target_ids: ids,
            });
        }
    }
    Ok(out)
}

fn snapshot_labels(
    db: &Db,
    account_id: &str,
    ids: &[String],
) -> SyncResult<HashMap<String, String>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    db.with_connection(|conn| {
        let mut sql = String::from(
            "SELECT provider_msg_id, label_ids FROM messages
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
            .query_map(rusqlite::params_from_iter(bound.iter()), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(DbError::from)?;
        let mut out = HashMap::new();
        for r in rows {
            let (id, labels) = r.map_err(DbError::from)?;
            out.insert(id, labels);
        }
        Ok(out)
    })
    .map_err(SyncError::from)
}

fn fetch_members(db: &Db, account_id: &str, cluster_key: &str) -> SyncResult<Vec<String>> {
    db.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT provider_msg_id FROM message_clusters
                 WHERE account_id = ?1 AND cluster_key = ?2",
            )
            .map_err(DbError::from)?;
        let rows = stmt
            .query_map(params![account_id, cluster_key], |r| r.get::<_, String>(0))
            .map_err(DbError::from)?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r.map_err(DbError::from)?);
        }
        Ok(v)
    })
    .map_err(SyncError::from)
}

fn parse_action(s: &str) -> Option<ActionType> {
    match s {
        "archive" => Some(ActionType::Archive),
        "trash" => Some(ActionType::Trash),
        "add_label" => Some(ActionType::AddLabel),
        "remove_label" => Some(ActionType::RemoveLabel),
        "mark_read" => Some(ActionType::MarkRead),
        "unsubscribe" => Some(ActionType::Unsubscribe),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::StagedActionsRepo;
    use crate::db::{open_in_memory, ClusterMember, ClustersRepo, MessageRow, MessagesRepo};
    use async_trait::async_trait;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct FakeMutator {
        archive_calls: Mutex<Vec<Vec<String>>>,
        trash_calls: Mutex<Vec<Vec<String>>>,
        fail_archive: bool,
    }

    #[async_trait]
    impl GmailMutations for FakeMutator {
        async fn batch_archive(&self, ids: &[String]) -> SyncResult<()> {
            if self.fail_archive {
                return Err(SyncError::GmailStatus { status: 500, body: "boom".into() });
            }
            self.archive_calls.lock().push(ids.to_vec());
            Ok(())
        }
        async fn batch_trash(&self, ids: &[String]) -> SyncResult<()> {
            self.trash_calls.lock().push(ids.to_vec());
            Ok(())
        }
        async fn batch_add_label(&self, _ids: &[String], _label: &str) -> SyncResult<()> {
            Ok(())
        }
        async fn batch_remove_label(&self, _ids: &[String], _label: &str) -> SyncResult<()> {
            Ok(())
        }
        async fn batch_mark_read(&self, _ids: &[String]) -> SyncResult<()> {
            Ok(())
        }
        async fn batch_delete(&self, _ids: &[String]) -> SyncResult<()> {
            Ok(())
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

    fn seed_cluster(db: &Db, cluster_key: &str, msg_ids: &[&str]) {
        let mr = MessagesRepo::new(db);
        let cr = ClustersRepo::new(db);
        let mut members = Vec::new();
        for id in msg_ids {
            mr.upsert(&MessageRow {
                account_id: "a1".into(),
                provider_msg_id: (*id).into(),
                thread_id: format!("t-{id}"),
                sender: None,
                sender_email: None,
                subject: None,
                snippet: None,
                internal_date: 0,
                label_ids: vec!["INBOX".into()],
                history_id: None,
            })
            .unwrap();
            members.push(ClusterMember {
                provider_msg_id: (*id).into(),
                cluster_key: cluster_key.into(),
                centroid_cosine: Some(1.0),
            });
        }
        cr.upsert_many("a1", &members).unwrap();
    }

    fn noop_sink() -> ExecuteSink {
        Arc::new(|_p| {})
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cluster_level_archive_fans_out_to_members() {
        let db = db();
        seed_cluster(&db, "c1", &["m1", "m2", "m3"]);
        StagedActionsRepo::new(&db)
            .stage_cluster("a1", "c1", ActionType::Archive, None)
            .unwrap();

        let mutator = Arc::new(FakeMutator::default());
        execute_account(
            mutator.clone(),
            db.clone(),
            "a1",
            ExecuteConfig::default(),
            CancellationToken::new(),
            noop_sink(),
        )
        .await
        .unwrap();

        let calls = mutator.archive_calls.lock();
        assert_eq!(calls.len(), 1);
        let mut got = calls[0].clone();
        got.sort();
        assert_eq!(got, vec!["m1".to_owned(), "m2".to_owned(), "m3".to_owned()]);

        let c = ActionsLogRepo::new(&db).counts("a1").unwrap();
        assert_eq!(c.success, 3);
        assert_eq!(c.failure, 0);
        // staged row was consumed
        assert_eq!(StagedActionsRepo::new(&db).count_for_account("a1").unwrap(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn batches_at_configured_size() {
        let db = db();
        let ids: Vec<String> = (0..2500).map(|i| format!("m{i}")).collect();
        let ids_ref: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        seed_cluster(&db, "big", &ids_ref);
        StagedActionsRepo::new(&db)
            .stage_cluster("a1", "big", ActionType::Archive, None)
            .unwrap();

        let mutator = Arc::new(FakeMutator::default());
        execute_account(
            mutator.clone(),
            db.clone(),
            "a1",
            ExecuteConfig { batch_size: 1000 },
            CancellationToken::new(),
            noop_sink(),
        )
        .await
        .unwrap();
        let calls = mutator.archive_calls.lock();
        // 2500 / 1000 = 3 batches.
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].len(), 1000);
        assert_eq!(calls[1].len(), 1000);
        assert_eq!(calls[2].len(), 500);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failure_logs_outcome_and_keeps_staging() {
        let db = db();
        seed_cluster(&db, "c1", &["m1", "m2"]);
        StagedActionsRepo::new(&db)
            .stage_cluster("a1", "c1", ActionType::Archive, None)
            .unwrap();
        let mutator = Arc::new(FakeMutator { fail_archive: true, ..Default::default() });
        execute_account(
            mutator,
            db.clone(),
            "a1",
            ExecuteConfig::default(),
            CancellationToken::new(),
            noop_sink(),
        )
        .await
        .unwrap();
        let c = ActionsLogRepo::new(&db).counts("a1").unwrap();
        assert_eq!(c.failure, 2);
        assert_eq!(c.success, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancellation_halts_after_current_batch() {
        let db = db();
        let ids: Vec<String> = (0..2500).map(|i| format!("m{i}")).collect();
        let ids_ref: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        seed_cluster(&db, "big", &ids_ref);
        StagedActionsRepo::new(&db)
            .stage_cluster("a1", "big", ActionType::Archive, None)
            .unwrap();

        let token = CancellationToken::new();
        token.cancel(); // pre-cancel; first batch boundary aborts.

        let mutator = Arc::new(FakeMutator::default());
        let err = execute_account(
            mutator.clone(),
            db.clone(),
            "a1",
            ExecuteConfig { batch_size: 1000 },
            token,
            noop_sink(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SyncError::Cancelled));
        // No archive call should have completed.
        assert_eq!(mutator.archive_calls.lock().len(), 0);
    }
}
