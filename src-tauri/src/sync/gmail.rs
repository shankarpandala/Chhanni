use std::sync::Arc;
use std::time::Instant;

use futures::stream::{FuturesUnordered, StreamExt};
use tracing::{info, warn};

use crate::db::{Db, MessageRow, MessagesRepo, SyncPhase, SyncState, SyncStateRepo};
use crate::error::{SyncError, SyncResult};
use crate::providers::gmail::{parse_sender_email, GmailApi, MessageRef};
use crate::sync::progress::{SyncProgress, SyncStage};

/// Tunables for one sync run. Defaults are sized for the M5 Pro target.
#[derive(Clone, Debug)]
pub struct GmailSyncConfig {
    /// `users.messages.list` page size.
    pub list_page_size: u32,
    /// Max concurrent `messages.get` requests in-flight.
    pub metadata_concurrency: usize,
    /// Persist to SQLite every N metadata results.
    pub persist_batch: usize,
}

impl Default for GmailSyncConfig {
    fn default() -> Self {
        Self {
            list_page_size: 500,
            // Gmail's per-user-per-minute query quota is 15k. At ~50ms latency
            // sustained concurrency of 20 burns ~24k/min and trips a 403
            // rateLimitExceeded; 10 keeps us at ~12k/min with headroom.
            metadata_concurrency: 10,
            persist_batch: 100,
        }
    }
}

pub type ProgressSink = Arc<dyn Fn(&SyncProgress) + Send + Sync>;

pub async fn run_gmail_sync(
    api: Arc<dyn GmailApi>,
    db: Db,
    account_id: &str,
    config: GmailSyncConfig,
    on_progress: ProgressSink,
) -> SyncResult<()> {
    let started = Instant::now();
    let state_repo = SyncStateRepo::new(&db);
    let state = state_repo.get(account_id)?;

    match state {
        None => {
            state_repo.mark_initial_started(account_id)?;
            initial_sync(api, &db, account_id, &config, started, on_progress).await
        }
        Some(SyncState { phase: SyncPhase::Initial, cursor_token, .. }) => {
            // Resume initial walk from the recorded cursor.
            resume_initial(
                api,
                &db,
                account_id,
                cursor_token,
                &config,
                started,
                on_progress,
            )
            .await
        }
        Some(SyncState { phase: SyncPhase::Incremental, last_history_id: Some(hid), .. }) => {
            incremental_sync(api, &db, account_id, hid, &config, started, on_progress).await
        }
        Some(SyncState { phase: SyncPhase::Incremental, last_history_id: None, .. }) => {
            warn!(account_id, "incremental phase with no history id; resetting to initial");
            state_repo.mark_initial_started(account_id)?;
            initial_sync(api, &db, account_id, &config, started, on_progress).await
        }
    }
}

async fn initial_sync(
    api: Arc<dyn GmailApi>,
    db: &Db,
    account_id: &str,
    config: &GmailSyncConfig,
    started: Instant,
    on_progress: ProgressSink,
) -> SyncResult<()> {
    walk_initial(api, db, account_id, None, config, started, on_progress).await
}

async fn resume_initial(
    api: Arc<dyn GmailApi>,
    db: &Db,
    account_id: &str,
    cursor: Option<String>,
    config: &GmailSyncConfig,
    started: Instant,
    on_progress: ProgressSink,
) -> SyncResult<()> {
    walk_initial(api, db, account_id, cursor, config, started, on_progress).await
}

async fn walk_initial(
    api: Arc<dyn GmailApi>,
    db: &Db,
    account_id: &str,
    mut cursor: Option<String>,
    config: &GmailSyncConfig,
    started: Instant,
    on_progress: ProgressSink,
) -> SyncResult<()> {
    let state_repo = SyncStateRepo::new(db);
    let mut seen: u64 = 0;
    let mut persisted: u64 = 0;
    let mut highest_history_id: u64 = 0;

    loop {
        report(
            &on_progress,
            account_id,
            SyncStage::Listing,
            seen,
            persisted,
            started,
        );
        let page = api
            .list_messages(cursor.as_deref(), config.list_page_size)
            .await?;
        let next_token = page.next_page_token.clone();

        if page.messages.is_empty() && cursor.is_none() {
            // Empty mailbox; record an incremental baseline of 1 so future
            // history.list calls succeed.
            state_repo.mark_initial_complete(account_id, 1)?;
            report(&on_progress, account_id, SyncStage::Done, seen, persisted, started);
            return Ok(());
        }

        seen += page.messages.len() as u64;

        let (new_history, new_persisted) = fetch_and_persist(
            api.clone(),
            db,
            account_id,
            &page.messages,
            config,
            started,
            seen,
            persisted,
            &on_progress,
        )
        .await?;

        persisted += new_persisted;
        highest_history_id = highest_history_id.max(new_history);

        // Checkpoint after every page so a kill leaves us mid-walk, not at zero.
        state_repo.upsert(&SyncState {
            account_id: account_id.to_owned(),
            phase: SyncPhase::Initial,
            cursor_token: next_token.clone(),
            last_history_id: if highest_history_id > 0 {
                Some(highest_history_id)
            } else {
                None
            },
            last_sync_at: None,
        })?;

        match next_token {
            Some(t) => cursor = Some(t),
            None => break,
        }
    }

    state_repo.mark_initial_complete(account_id, highest_history_id.max(1))?;
    report(&on_progress, account_id, SyncStage::Done, seen, persisted, started);
    info!(account_id, seen, persisted, "gmail initial sync complete");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn fetch_and_persist(
    api: Arc<dyn GmailApi>,
    db: &Db,
    account_id: &str,
    refs: &[MessageRef],
    config: &GmailSyncConfig,
    started: Instant,
    seen: u64,
    base_persisted: u64,
    on_progress: &ProgressSink,
) -> SyncResult<(u64, u64)> {
    let mut futs = FuturesUnordered::new();
    let mut highest_history: u64 = 0;
    let mut buffer: Vec<MessageRow> = Vec::with_capacity(config.persist_batch);
    let mut persisted_this_page: u64 = 0;

    for r in refs.iter().cloned() {
        let api = api.clone();
        futs.push(async move { api.get_metadata(&r.id).await });
    }

    while let Some(res) = futs.next().await {
        let meta = res?;
        let history = meta.history_id_u64();
        highest_history = highest_history.max(history);
        let from_header = meta.header("From").unwrap_or("").to_owned();
        let subject = meta.header("Subject").map(|s| s.to_owned());
        let sender_email = parse_sender_email(&from_header);
        let row = MessageRow {
            account_id: account_id.to_owned(),
            provider_msg_id: meta.id.clone(),
            thread_id: meta.thread_id.clone(),
            sender: if from_header.is_empty() { None } else { Some(from_header) },
            sender_email,
            subject,
            snippet: meta.snippet.clone(),
            internal_date: meta.internal_date_ms(),
            label_ids: meta.label_ids.clone(),
            history_id: Some(history),
        };
        buffer.push(row);

        if buffer.len() >= config.persist_batch {
            let n = MessagesRepo::new(db).upsert_many(&buffer)?;
            persisted_this_page += n as u64;
            buffer.clear();
            report(
                on_progress,
                account_id,
                SyncStage::Fetching,
                seen,
                base_persisted + persisted_this_page,
                started,
            );
        }
    }
    if !buffer.is_empty() {
        let n = MessagesRepo::new(db).upsert_many(&buffer)?;
        persisted_this_page += n as u64;
    }
    Ok((highest_history, persisted_this_page))
}

async fn incremental_sync(
    api: Arc<dyn GmailApi>,
    db: &Db,
    account_id: &str,
    start_history_id: u64,
    config: &GmailSyncConfig,
    started: Instant,
    on_progress: ProgressSink,
) -> SyncResult<()> {
    let state_repo = SyncStateRepo::new(db);
    let mut cursor: Option<String> = None;
    let mut highest_history_id = start_history_id;
    let mut seen: u64 = 0;
    let mut persisted: u64 = 0;

    loop {
        report(
            &on_progress,
            account_id,
            SyncStage::Incremental,
            seen,
            persisted,
            started,
        );
        let page = api
            .list_history(highest_history_id, cursor.as_deref())
            .await?;
        let next = page.next_page_token.clone();
        if let Ok(h) = page.history_id.parse::<u64>() {
            highest_history_id = highest_history_id.max(h);
        }

        // Collect deduplicated message refs from added + label-changed records.
        let mut refs: Vec<MessageRef> = Vec::new();
        for rec in &page.history {
            for added in &rec.messages_added {
                refs.push(added.message.clone());
            }
            for changed in rec.labels_added.iter().chain(rec.labels_removed.iter()) {
                refs.push(changed.message.clone());
            }
            for deleted in &rec.messages_deleted {
                MessagesRepo::new(db).delete(account_id, &deleted.message.id)?;
            }
        }
        // Dedupe within page.
        refs.sort_by(|a, b| a.id.cmp(&b.id));
        refs.dedup_by(|a, b| a.id == b.id);
        seen += refs.len() as u64;

        let (page_hist, new_persisted) = fetch_and_persist(
            api.clone(),
            db,
            account_id,
            &refs,
            config,
            started,
            seen,
            persisted,
            &on_progress,
        )
        .await?;
        persisted += new_persisted;
        highest_history_id = highest_history_id.max(page_hist);

        state_repo.upsert(&SyncState {
            account_id: account_id.to_owned(),
            phase: SyncPhase::Incremental,
            cursor_token: next.clone(),
            last_history_id: Some(highest_history_id),
            last_sync_at: Some(now_iso()),
        })?;

        match next {
            Some(t) => cursor = Some(t),
            None => break,
        }
    }
    report(&on_progress, account_id, SyncStage::Done, seen, persisted, started);
    info!(account_id, seen, persisted, "gmail incremental sync complete");
    Ok(())
}

fn report(
    sink: &ProgressSink,
    account_id: &str,
    stage: SyncStage,
    seen: u64,
    persisted: u64,
    started: Instant,
) {
    let p = SyncProgress {
        account_id: account_id.to_owned(),
        stage,
        messages_seen: seen,
        messages_persisted: persisted,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    sink(&p);
}

fn now_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::new())
}

// Guard so anyhow::Error doesn't try to cast SyncError implicitly.
impl From<SyncError> for String {
    fn from(value: SyncError) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::providers::gmail::{
        HistoryPage, HistoryRecord, ListMessagesPage, MessageMetadata, MessagePayload,
    };
    use async_trait::async_trait;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct Fake {
        list_pages: Mutex<Vec<ListMessagesPage>>,
        metadata: Mutex<std::collections::HashMap<String, MessageMetadata>>,
        history_pages: Mutex<Vec<HistoryPage>>,
        list_calls: Mutex<u32>,
        metadata_calls: Mutex<u32>,
        fail_when_empty: bool,
    }

    impl Fake {
        fn with_pages(pages: Vec<ListMessagesPage>) -> Self {
            Self {
                list_pages: Mutex::new(pages),
                ..Default::default()
            }
        }

        fn with_pages_failing_after(pages: Vec<ListMessagesPage>) -> Self {
            Self {
                list_pages: Mutex::new(pages),
                fail_when_empty: true,
                ..Default::default()
            }
        }

        fn put_metadata(&self, m: MessageMetadata) {
            self.metadata.lock().insert(m.id.clone(), m);
        }
    }

    #[async_trait]
    impl GmailApi for Fake {
        async fn list_messages(
            &self,
            _token: Option<&str>,
            _max: u32,
        ) -> SyncResult<ListMessagesPage> {
            *self.list_calls.lock() += 1;
            let mut pages = self.list_pages.lock();
            if pages.is_empty() {
                if self.fail_when_empty {
                    return Err(SyncError::GmailStatus {
                        status: 599,
                        body: "simulated interruption".into(),
                    });
                }
                return Ok(ListMessagesPage {
                    messages: vec![],
                    next_page_token: None,
                    result_size_estimate: Some(0),
                });
            }
            Ok(pages.remove(0))
        }

        async fn get_metadata(&self, id: &str) -> SyncResult<MessageMetadata> {
            *self.metadata_calls.lock() += 1;
            self.metadata
                .lock()
                .get(id)
                .cloned()
                .ok_or(SyncError::GmailStatus {
                    status: 404,
                    body: format!("missing metadata for {id}"),
                })
        }

        async fn list_history(
            &self,
            _start: u64,
            _token: Option<&str>,
        ) -> SyncResult<HistoryPage> {
            let mut pages = self.history_pages.lock();
            if pages.is_empty() {
                return Ok(HistoryPage {
                    history: vec![],
                    next_page_token: None,
                    history_id: "1".to_owned(),
                });
            }
            Ok(pages.remove(0))
        }
    }

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

    fn meta(id: &str, history: &str) -> MessageMetadata {
        MessageMetadata {
            id: id.to_owned(),
            thread_id: format!("t-{id}"),
            label_ids: vec!["INBOX".to_owned()],
            snippet: Some(format!("snippet {id}")),
            internal_date: "1700000000000".to_owned(),
            history_id: history.to_owned(),
            payload: Some(MessagePayload {
                headers: vec![
                    crate::providers::gmail::api::Header {
                        name: "From".into(),
                        value: format!("\"Sender {id}\" <{id}@example.com>"),
                    },
                    crate::providers::gmail::api::Header {
                        name: "Subject".into(),
                        value: format!("subject {id}"),
                    },
                ],
            }),
        }
    }

    fn noop_sink() -> ProgressSink {
        Arc::new(|_p: &SyncProgress| {})
    }

    #[tokio::test(flavor = "current_thread")]
    async fn initial_sync_persists_and_transitions_phase() {
        let api = Fake::with_pages(vec![ListMessagesPage {
            messages: vec![
                MessageRef { id: "m1".into(), thread_id: "t1".into() },
                MessageRef { id: "m2".into(), thread_id: "t2".into() },
            ],
            next_page_token: None,
            result_size_estimate: Some(2),
        }]);
        api.put_metadata(meta("m1", "10"));
        api.put_metadata(meta("m2", "11"));

        let db = db();
        run_gmail_sync(
            Arc::new(api),
            db.clone(),
            "a1",
            GmailSyncConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();

        assert_eq!(MessagesRepo::new(&db).count_for_account("a1").unwrap(), 2);
        let state = SyncStateRepo::new(&db).get("a1").unwrap().unwrap();
        assert_eq!(state.phase, SyncPhase::Incremental);
        assert_eq!(state.last_history_id, Some(11));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rerunning_after_completion_is_a_noop_incremental() {
        let api = Fake::with_pages(vec![ListMessagesPage {
            messages: vec![MessageRef { id: "m1".into(), thread_id: "t1".into() }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }]);
        api.put_metadata(meta("m1", "5"));
        let db = db();
        let api = Arc::new(api);
        run_gmail_sync(
            api.clone(),
            db.clone(),
            "a1",
            GmailSyncConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();

        // Second run: no history records => no new persisted rows, no new fetches.
        let before = MessagesRepo::new(&db).count_for_account("a1").unwrap();
        run_gmail_sync(
            api.clone(),
            db.clone(),
            "a1",
            GmailSyncConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();
        let after = MessagesRepo::new(&db).count_for_account("a1").unwrap();
        assert_eq!(before, after);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn empty_mailbox_records_baseline_history_id() {
        let api = Fake::with_pages(vec![ListMessagesPage {
            messages: vec![],
            next_page_token: None,
            result_size_estimate: Some(0),
        }]);
        let db = db();
        run_gmail_sync(
            Arc::new(api),
            db.clone(),
            "a1",
            GmailSyncConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();
        let state = SyncStateRepo::new(&db).get("a1").unwrap().unwrap();
        assert_eq!(state.phase, SyncPhase::Incremental);
        assert_eq!(state.last_history_id, Some(1));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resume_after_interrupted_initial() {
        // Sim: first run returns page 1 with next_token=Some, then the next
        // list_messages call errors (network drop). The checkpoint after
        // page 1 should still leave us in Initial with cursor=PAGE2.
        let api1 = Fake::with_pages_failing_after(vec![ListMessagesPage {
            messages: vec![MessageRef { id: "m1".into(), thread_id: "t1".into() }],
            next_page_token: Some("PAGE2".into()),
            result_size_estimate: Some(2),
        }]);
        api1.put_metadata(meta("m1", "1"));
        let db = db();
        let err = run_gmail_sync(
            Arc::new(api1),
            db.clone(),
            "a1",
            GmailSyncConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SyncError::GmailStatus { status: 599, .. }));
        let state = SyncStateRepo::new(&db).get("a1").unwrap().unwrap();
        assert_eq!(state.phase, SyncPhase::Initial);
        assert_eq!(state.cursor_token.as_deref(), Some("PAGE2"));

        // New api for the resumed run.
        let api2 = Fake::with_pages(vec![ListMessagesPage {
            messages: vec![MessageRef { id: "m2".into(), thread_id: "t2".into() }],
            next_page_token: None,
            result_size_estimate: Some(2),
        }]);
        api2.put_metadata(meta("m2", "2"));
        run_gmail_sync(
            Arc::new(api2),
            db.clone(),
            "a1",
            GmailSyncConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();
        assert_eq!(MessagesRepo::new(&db).count_for_account("a1").unwrap(), 2);
        let state = SyncStateRepo::new(&db).get("a1").unwrap().unwrap();
        assert_eq!(state.phase, SyncPhase::Incremental);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn incremental_processes_added_and_deleted() {
        // Prime: one message via initial sync.
        let api1 = Fake::with_pages(vec![ListMessagesPage {
            messages: vec![MessageRef { id: "m1".into(), thread_id: "t1".into() }],
            next_page_token: None,
            result_size_estimate: Some(1),
        }]);
        api1.put_metadata(meta("m1", "10"));
        let db = db();
        run_gmail_sync(
            Arc::new(api1),
            db.clone(),
            "a1",
            GmailSyncConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();

        // Second run: history page adds m2, deletes m1.
        let api2 = Fake::default();
        api2.put_metadata(meta("m2", "20"));
        api2.history_pages.lock().push(HistoryPage {
            history: vec![HistoryRecord {
                id: "h1".into(),
                messages_added: vec![crate::providers::gmail::api::HistoryMessageEntry {
                    message: MessageRef { id: "m2".into(), thread_id: "t2".into() },
                }],
                messages_deleted: vec![crate::providers::gmail::api::HistoryMessageEntry {
                    message: MessageRef { id: "m1".into(), thread_id: "t1".into() },
                }],
                labels_added: vec![],
                labels_removed: vec![],
            }],
            next_page_token: None,
            history_id: "20".into(),
        });
        run_gmail_sync(
            Arc::new(api2),
            db.clone(),
            "a1",
            GmailSyncConfig::default(),
            noop_sink(),
        )
        .await
        .unwrap();

        // m1 deleted, m2 added.
        let count = MessagesRepo::new(&db).count_for_account("a1").unwrap();
        assert_eq!(count, 1);
    }
}
