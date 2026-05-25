//! Microsoft Graph incremental sync. Persists messages to the same SQLite
//! `messages` table as Gmail; provider-specific identifiers are the Graph
//! message id. We reuse `sync_state` and abuse `cursor_token` to store the
//! delta link verbatim (Graph delta tokens are absolute URLs).

use std::sync::Arc;
use std::time::Instant;

use tracing::info;

use crate::db::{Db, MessageRow, MessagesRepo, SyncPhase, SyncState, SyncStateRepo};
use crate::error::SyncResult;
use crate::providers::graph::api::{DeltaPage, GraphApi, GraphMessage};
use crate::sync::progress::{SyncProgress, SyncStage};

pub type ProgressSink = Arc<dyn Fn(&SyncProgress) + Send + Sync>;

pub async fn run_graph_sync(
    api: Arc<dyn GraphApi>,
    db: Db,
    account_id: &str,
    on_progress: ProgressSink,
) -> SyncResult<()> {
    let started = Instant::now();
    let state_repo = SyncStateRepo::new(&db);
    let saved = state_repo.get(account_id)?;
    let delta_link = saved.as_ref().and_then(|s| s.cursor_token.clone());

    let mut seen: u64 = 0;
    let mut persisted: u64 = 0;
    let mut last_delta_link: Option<String> = None;

    let mut page = api.fetch_delta(delta_link.as_deref()).await?;
    loop {
        let (s, p) = ingest_page(&db, account_id, &page)?;
        seen += s;
        persisted += p;
        emit(&on_progress, account_id, SyncStage::Fetching, seen, persisted, started);

        if let Some(d) = page.delta_link.clone() {
            last_delta_link = Some(d);
        }

        match page.next_link.clone() {
            Some(next) => page = api.fetch_next(&next).await?,
            None => break,
        }
    }

    state_repo.upsert(&SyncState {
        account_id: account_id.to_owned(),
        phase: SyncPhase::Incremental,
        cursor_token: last_delta_link,
        last_history_id: None,
        last_sync_at: Some(time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default()),
    })?;
    emit(&on_progress, account_id, SyncStage::Done, seen, persisted, started);
    info!(account_id, seen, persisted, "graph sync complete");
    Ok(())
}

fn ingest_page(db: &Db, account_id: &str, page: &DeltaPage) -> SyncResult<(u64, u64)> {
    let mut to_upsert: Vec<MessageRow> = Vec::new();
    let mut seen = 0u64;
    for m in &page.value {
        seen += 1;
        if m.is_removed() {
            MessagesRepo::new(db).delete(account_id, &m.id)?;
            continue;
        }
        to_upsert.push(row_from_graph(account_id, m));
    }
    let persisted = if !to_upsert.is_empty() {
        MessagesRepo::new(db).upsert_many(&to_upsert)? as u64
    } else {
        0
    };
    Ok((seen, persisted))
}

fn row_from_graph(account_id: &str, m: &GraphMessage) -> MessageRow {
    let sender_email = m.from_email().map(str::to_ascii_lowercase);
    let sender = match (m.from_name(), m.from_email()) {
        (Some(name), Some(addr)) => Some(format!("{name} <{addr}>")),
        (None, Some(addr)) => Some(addr.to_owned()),
        _ => None,
    };
    // Categories are user-applied "tags" in Outlook; we map them into the
    // shared `label_ids` JSON column so downstream code (cluster/classify)
    // works unchanged. We also synthesize a parent-folder pseudo-label.
    let mut labels = m.categories.clone();
    if let Some(folder) = m.parent_folder_id.clone() {
        labels.push(format!("FOLDER:{folder}"));
    }
    if !m.is_read {
        labels.push("UNREAD".to_owned());
    }

    MessageRow {
        account_id: account_id.to_owned(),
        provider_msg_id: m.id.clone(),
        thread_id: m
            .conversation_id
            .clone()
            .unwrap_or_else(|| m.id.clone()),
        sender,
        sender_email,
        subject: m.subject.clone(),
        snippet: m.body_preview.clone(),
        internal_date: m.received_unix_ms(),
        label_ids: labels,
        history_id: None,
    }
}

fn emit(
    sink: &ProgressSink,
    account_id: &str,
    stage: SyncStage,
    seen: u64,
    persisted: u64,
    started: Instant,
) {
    sink(&SyncProgress {
        account_id: account_id.to_owned(),
        stage,
        messages_seen: seen,
        messages_persisted: persisted,
        elapsed_ms: started.elapsed().as_millis() as u64,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::providers::graph::api::{EmailAddress, EmailField};
    use async_trait::async_trait;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct Fake {
        delta_pages: Mutex<Vec<DeltaPage>>,
        calls: Mutex<u32>,
    }

    #[async_trait]
    impl GraphApi for Fake {
        async fn fetch_delta(&self, _delta: Option<&str>) -> SyncResult<DeltaPage> {
            *self.calls.lock() += 1;
            Ok(self.delta_pages.lock().remove(0))
        }
        async fn fetch_next(&self, _next: &str) -> SyncResult<DeltaPage> {
            *self.calls.lock() += 1;
            Ok(self.delta_pages.lock().remove(0))
        }
    }

    fn db() -> Db {
        let db = open_in_memory().unwrap();
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO accounts (account_id, provider, email) VALUES ('a1','graph','a@b')",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        db
    }

    fn msg(id: &str, addr: &str) -> GraphMessage {
        GraphMessage {
            id: id.into(),
            conversation_id: Some(format!("c-{id}")),
            subject: Some(format!("subj {id}")),
            body_preview: Some(format!("preview {id}")),
            from: Some(EmailField {
                email_address: Some(EmailAddress {
                    name: Some("Sender".into()),
                    address: Some(addr.into()),
                }),
            }),
            received_date_time: Some("2026-05-01T12:00:00Z".into()),
            parent_folder_id: Some("FOLDER123".into()),
            is_read: false,
            categories: vec![],
            removed: None,
        }
    }

    fn noop_sink() -> ProgressSink {
        Arc::new(|_p| {})
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delta_walk_inserts_rows_and_stores_delta_link() {
        let fake = Fake {
            delta_pages: Mutex::new(vec![DeltaPage {
                value: vec![msg("m1", "x@example.com"), msg("m2", "y@example.com")],
                next_link: None,
                delta_link: Some("https://graph.microsoft.com/v1.0/me/messages/delta?$deltatoken=abc".into()),
            }]),
            ..Default::default()
        };
        let db = db();
        run_graph_sync(Arc::new(fake), db.clone(), "a1", noop_sink())
            .await
            .unwrap();
        assert_eq!(MessagesRepo::new(&db).count_for_account("a1").unwrap(), 2);
        let state = SyncStateRepo::new(&db).get("a1").unwrap().unwrap();
        assert_eq!(state.phase, SyncPhase::Incremental);
        assert!(state.cursor_token.as_deref().unwrap().contains("deltatoken=abc"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn removed_marker_deletes_existing_message() {
        let fake = Fake {
            delta_pages: Mutex::new(vec![
                DeltaPage {
                    value: vec![msg("m1", "a@example.com")],
                    next_link: None,
                    delta_link: Some("delta-1".into()),
                },
                DeltaPage {
                    value: vec![GraphMessage {
                        id: "m1".into(),
                        conversation_id: None,
                        subject: None,
                        body_preview: None,
                        from: None,
                        received_date_time: None,
                        parent_folder_id: None,
                        is_read: false,
                        categories: vec![],
                        removed: Some(serde_json::json!({ "reason": "deleted" })),
                    }],
                    next_link: None,
                    delta_link: Some("delta-2".into()),
                },
            ]),
            ..Default::default()
        };
        let db = db();
        let api = Arc::new(fake);
        run_graph_sync(api.clone(), db.clone(), "a1", noop_sink()).await.unwrap();
        assert_eq!(MessagesRepo::new(&db).count_for_account("a1").unwrap(), 1);
        run_graph_sync(api, db.clone(), "a1", noop_sink()).await.unwrap();
        assert_eq!(MessagesRepo::new(&db).count_for_account("a1").unwrap(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn next_link_is_followed() {
        let fake = Fake {
            delta_pages: Mutex::new(vec![
                DeltaPage {
                    value: vec![msg("m1", "a@x.com")],
                    next_link: Some("https://graph.microsoft.com/v1.0/page2".into()),
                    delta_link: None,
                },
                DeltaPage {
                    value: vec![msg("m2", "b@x.com")],
                    next_link: None,
                    delta_link: Some("delta-x".into()),
                },
            ]),
            ..Default::default()
        };
        let db = db();
        run_graph_sync(Arc::new(fake), db.clone(), "a1", noop_sink())
            .await
            .unwrap();
        assert_eq!(MessagesRepo::new(&db).count_for_account("a1").unwrap(), 2);
    }
}
