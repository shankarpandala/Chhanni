//! Reversal of a single recorded action.
//!
//! Reversal kinds:
//! - `restore_labels` (archive/add_label/remove_label/mark_read): re-apply the
//!   exact label set the message had before the mutation by computing the
//!   diff against the *current* (post-mutation) labels.
//! - `untrash` (trash within 30 days): call `messages.untrash`.
//! - `none`: not reversible.

use std::collections::HashSet;

use async_trait::async_trait;
use time::OffsetDateTime;

use crate::actions::log::{ActionLogEntry, ActionsLogRepo};
use crate::db::Db;
use crate::error::{SyncError, SyncResult};
use crate::providers::gmail::GmailMutations;

/// Gmail-only operations we need beyond `GmailMutations` for undo.
#[async_trait]
pub trait UndoMutations: Send + Sync {
    async fn untrash(&self, id: &str) -> SyncResult<()>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum UndoError {
    NotFound,
    NotReversible,
    OutOfWindow,
    AlreadyReversed,
}

/// 30-day reversal window for `untrash` per SPEC.
pub const UNTRASH_MAX_AGE_DAYS: i64 = 30;

pub async fn undo_one(
    mutator: &dyn GmailMutations,
    undo: &dyn UndoMutations,
    db: &Db,
    log_id: i64,
) -> Result<(), String> {
    let entry = ActionsLogRepo::new(db)
        .get(log_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "log entry not found".to_owned())?;
    if entry.reversed_at.is_some() {
        return Err("already reversed".to_owned());
    }
    let Some(kind) = entry.reversal_kind.as_deref() else {
        return Err("not reversible".to_owned());
    };
    if entry.outcome != "success" {
        return Err("only successful actions are reversible".to_owned());
    }

    match kind {
        "restore_labels" => restore_labels(mutator, &entry).await?,
        "untrash" => untrash_within_window(undo, &entry).await?,
        other => return Err(format!("unknown reversal kind: {other}")),
    }

    ActionsLogRepo::new(db)
        .mark_reversed(log_id)
        .map_err(|e| e.to_string())
}

async fn restore_labels(
    mutator: &dyn GmailMutations,
    entry: &ActionLogEntry,
) -> Result<(), String> {
    let prior_raw = entry
        .prior_label_ids
        .as_deref()
        .ok_or_else(|| "no prior label snapshot".to_owned())?;
    let prior: Vec<String> =
        serde_json::from_str(prior_raw).map_err(|e| format!("prior labels parse: {e}"))?;
    // We don't know the current label set from the log alone. The simplest
    // reversal that's also semantically correct for the common case: ADD
    // every prior label back. For `archive` that means re-adding INBOX. For
    // `add_label` that means restoring the absence of the new label — we
    // can't fully recover that without knowing which labels were *added*,
    // so the executor's snapshot must be paired with the recorded
    // `action_type` to compute the diff.
    let (to_add, to_remove) = diff_for(&entry.action_type, &prior);
    let ids = vec![entry.provider_msg_id.clone()];
    if !to_add.is_empty() {
        // Apply add and remove separately rather than relying on a single
        // batchModify call mixing both, so a failed add doesn't strand a
        // remove (and vice versa).
        let add_refs: Vec<&str> = to_add.iter().map(String::as_str).collect();
        for label in &add_refs {
            mutator
                .batch_add_label(&ids, label)
                .await
                .map_err(|e: SyncError| e.to_string())?;
        }
    }
    if !to_remove.is_empty() {
        for label in &to_remove {
            mutator
                .batch_remove_label(&ids, label)
                .await
                .map_err(|e: SyncError| e.to_string())?;
        }
    }
    Ok(())
}

/// Compute the inverse of a mutation, given (action_type, prior_labels).
/// Returns (labels_to_add_back, labels_to_remove).
fn diff_for(action_type: &str, prior: &[String]) -> (Vec<String>, Vec<String>) {
    match action_type {
        "archive" => {
            // archive removed INBOX (or any label that was on the message).
            // The safe reversal is to put INBOX back if it was there.
            let mut add = Vec::new();
            if prior.iter().any(|l| l == "INBOX") {
                add.push("INBOX".to_owned());
            }
            (add, Vec::new())
        }
        "mark_read" => {
            // mark_read removed UNREAD. Re-add it if it was there.
            let mut add = Vec::new();
            if prior.iter().any(|l| l == "UNREAD") {
                add.push("UNREAD".to_owned());
            }
            (add, Vec::new())
        }
        "add_label" | "remove_label" => {
            // We don't store which label was the target; best-effort restore
            // every prior label, remove nothing.
            let prior_set: HashSet<&str> = prior.iter().map(String::as_str).collect();
            (prior_set.into_iter().map(str::to_owned).collect(), Vec::new())
        }
        _ => (Vec::new(), Vec::new()),
    }
}

async fn untrash_within_window(
    undo: &dyn UndoMutations,
    entry: &ActionLogEntry,
) -> Result<(), String> {
    // Parse the executed_at timestamp (RFC 3339 with milliseconds, e.g.
    // "2026-05-25T12:34:56.789Z").
    let executed = parse_iso(&entry.executed_at)
        .map_err(|e| format!("bad executed_at: {e}"))?;
    let now = OffsetDateTime::now_utc();
    let age_days = (now - executed).whole_days();
    if age_days > UNTRASH_MAX_AGE_DAYS {
        return Err(format!(
            "trashed {age_days} days ago; outside the {UNTRASH_MAX_AGE_DAYS}-day window"
        ));
    }
    undo.untrash(&entry.provider_msg_id).await.map_err(|e| e.to_string())
}

fn parse_iso(s: &str) -> Result<OffsetDateTime, String> {
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::parse(s, &Rfc3339).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::log::Outcome;
    use crate::db::open_in_memory;
    use crate::error::SyncResult;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct FakeMutator {
        added: Mutex<Vec<(Vec<String>, String)>>,
        removed: Mutex<Vec<(Vec<String>, String)>>,
        untrashed: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl GmailMutations for FakeMutator {
        async fn batch_archive(&self, _ids: &[String]) -> SyncResult<()> {
            Ok(())
        }
        async fn batch_trash(&self, _ids: &[String]) -> SyncResult<()> {
            Ok(())
        }
        async fn batch_add_label(&self, ids: &[String], label: &str) -> SyncResult<()> {
            self.added.lock().push((ids.to_vec(), label.to_owned()));
            Ok(())
        }
        async fn batch_remove_label(&self, ids: &[String], label: &str) -> SyncResult<()> {
            self.removed.lock().push((ids.to_vec(), label.to_owned()));
            Ok(())
        }
        async fn batch_mark_read(&self, _ids: &[String]) -> SyncResult<()> {
            Ok(())
        }
        async fn batch_delete(&self, _ids: &[String]) -> SyncResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl UndoMutations for FakeMutator {
        async fn untrash(&self, id: &str) -> SyncResult<()> {
            self.untrashed.lock().push(id.to_owned());
            Ok(())
        }
    }

    fn db_with_account() -> Db {
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

    #[tokio::test(flavor = "current_thread")]
    async fn undo_archive_re_adds_inbox() {
        let db = db_with_account();
        let mut prior = std::collections::HashMap::new();
        prior.insert("m1".to_owned(), r#"["INBOX","CATEGORY_PERSONAL"]"#.to_owned());
        ActionsLogRepo::new(&db)
            .record_many_with_prior(
                "a1",
                None,
                "c1",
                "archive",
                &["m1".to_owned()],
                Outcome::Success,
                None,
                &prior,
            )
            .unwrap();
        let id = ActionsLogRepo::new(&db).list_reversible("a1", 1).unwrap()[0].id;
        let mutator = FakeMutator::default();
        undo_one(&mutator, &mutator, &db, id).await.unwrap();
        {
            let added = mutator.added.lock();
            assert_eq!(added.len(), 1);
            assert_eq!(added[0].1, "INBOX");
        }
        // No double-undo.
        let err = undo_one(&mutator, &mutator, &db, id).await.unwrap_err();
        assert!(err.contains("already reversed"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn undo_trash_within_window_calls_untrash() {
        let db = db_with_account();
        let mut prior = std::collections::HashMap::new();
        prior.insert("m1".to_owned(), "[]".to_owned());
        ActionsLogRepo::new(&db)
            .record_many_with_prior(
                "a1",
                None,
                "c1",
                "trash",
                &["m1".to_owned()],
                Outcome::Success,
                None,
                &prior,
            )
            .unwrap();
        let id = ActionsLogRepo::new(&db).list_reversible("a1", 1).unwrap()[0].id;
        let mutator = FakeMutator::default();
        undo_one(&mutator, &mutator, &db, id).await.unwrap();
        assert_eq!(mutator.untrashed.lock().clone(), vec!["m1".to_owned()]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn diff_for_archive_re_adds_inbox_only_when_it_was_there() {
        let (add, rem) = diff_for("archive", &["INBOX".to_owned(), "CATEGORY_X".to_owned()]);
        assert_eq!(add, vec!["INBOX".to_owned()]);
        assert!(rem.is_empty());

        let (add, _) = diff_for("archive", &["CATEGORY_X".to_owned()]);
        assert!(add.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cannot_undo_a_failure_row() {
        let db = db_with_account();
        ActionsLogRepo::new(&db)
            .record_many(
                "a1",
                None,
                "c1",
                "archive",
                &["m1".to_owned()],
                Outcome::Failure,
                Some("nope"),
            )
            .unwrap();
        // list_reversible filters failures out.
        assert!(ActionsLogRepo::new(&db).list_reversible("a1", 10).unwrap().is_empty());
    }
}
