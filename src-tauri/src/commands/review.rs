use rusqlite::params;
use serde::Serialize;
use tauri::State;

use crate::actions::rules::{propose_actions, ActionType, ClusterFacts, ProposedAction, RuleConfig};
use crate::actions::{StagedAction, StagedActionsRepo};
use crate::commands::gmail::AppState;
use crate::db::Category;
use crate::error::{DbError, DbResult};

#[derive(Serialize)]
pub struct ReviewClusterEntry {
    pub cluster_key: String,
    pub category: String,
    pub confidence: f32,
    pub member_count: i64,
    pub oldest_message_age_days: Option<i64>,
    pub has_list_unsubscribe: bool,
    pub sample_sender: Option<String>,
    pub sample_subject: Option<String>,
    pub proposed: Vec<ProposedAction>,
    pub staged_action_types: Vec<String>,
}

#[derive(Serialize)]
pub struct ReviewQueue {
    pub entries: Vec<ReviewClusterEntry>,
    pub staged_total: i64,
}

#[tauri::command]
pub fn list_review_queue(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<ReviewQueue, String> {
    let entries = collect_entries(&state, &account_id).map_err(|e| e.to_string())?;
    let staged_total = StagedActionsRepo::new(&state.db)
        .count_for_account(&account_id)
        .map_err(|e| e.to_string())?;
    Ok(ReviewQueue {
        entries,
        staged_total,
    })
}

#[tauri::command]
pub fn stage_action(
    state: State<'_, AppState>,
    account_id: String,
    cluster_key: String,
    action_type: String,
    reason: Option<String>,
) -> Result<i64, String> {
    let action = parse_action(&action_type)?;
    StagedActionsRepo::new(&state.db)
        .stage_cluster(&account_id, &cluster_key, action, reason.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn unstage_action(
    state: State<'_, AppState>,
    account_id: String,
    cluster_key: String,
    action_type: String,
) -> Result<usize, String> {
    let action = parse_action(&action_type)?;
    StagedActionsRepo::new(&state.db)
        .unstage_cluster(&account_id, &cluster_key, action)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_staged_actions(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<StagedAction>, String> {
    StagedActionsRepo::new(&state.db)
        .list_for_account(&account_id)
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct ClusterSampleMessage {
    pub provider_msg_id: String,
    pub sender: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub internal_date: i64,
}

#[tauri::command]
pub fn expand_cluster(
    state: State<'_, AppState>,
    account_id: String,
    cluster_key: String,
    limit: u32,
) -> Result<Vec<ClusterSampleMessage>, String> {
    state
        .db
        .with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT m.provider_msg_id, m.sender, m.subject, m.snippet, m.internal_date
                     FROM message_clusters mc
                     JOIN messages m
                       ON m.account_id = mc.account_id AND m.provider_msg_id = mc.provider_msg_id
                     WHERE mc.account_id = ?1 AND mc.cluster_key = ?2
                     ORDER BY m.internal_date DESC
                     LIMIT ?3",
                )
                .map_err(DbError::from)?;
            let rows = stmt
                .query_map(params![account_id, cluster_key, limit as i64], |r| {
                    Ok(ClusterSampleMessage {
                        provider_msg_id: r.get(0)?,
                        sender: r.get(1)?,
                        subject: r.get(2)?,
                        snippet: r.get(3)?,
                        internal_date: r.get(4)?,
                    })
                })
                .map_err(DbError::from)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(DbError::from)?);
            }
            Ok(out)
        })
        .map_err(|e: DbError| e.to_string())
}

fn collect_entries(state: &AppState, account_id: &str) -> DbResult<Vec<ReviewClusterEntry>> {
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp() * 1000;
    let cfg = RuleConfig::default();
    let staged = StagedActionsRepo::new(&state.db).list_for_account(account_id)?;
    let staged_index = staged_index(&staged);

    state.db.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT
                    mc.cluster_key,
                    COALESCE(cc.category, 'unknown') AS category,
                    COALESCE(cc.confidence, 0.0) AS confidence,
                    COUNT(*) AS member_count,
                    MIN(m.internal_date) AS oldest_date,
                    MAX(CASE WHEN json_extract(m.label_ids, '$') LIKE '%List-Unsubscribe%' THEN 1 ELSE 0 END) AS has_unsub,
                    (SELECT m2.sender FROM messages m2 WHERE m2.account_id = mc.account_id
                       AND m2.provider_msg_id = (SELECT provider_msg_id FROM message_clusters
                                                 WHERE account_id = mc.account_id AND cluster_key = mc.cluster_key
                                                 LIMIT 1)
                       LIMIT 1) AS sample_sender,
                    (SELECT m2.subject FROM messages m2 WHERE m2.account_id = mc.account_id
                       AND m2.provider_msg_id = (SELECT provider_msg_id FROM message_clusters
                                                 WHERE account_id = mc.account_id AND cluster_key = mc.cluster_key
                                                 LIMIT 1)
                       LIMIT 1) AS sample_subject
                 FROM message_clusters mc
                 JOIN messages m
                   ON m.account_id = mc.account_id AND m.provider_msg_id = mc.provider_msg_id
                 LEFT JOIN cluster_classifications cc
                   ON cc.account_id = mc.account_id AND cc.cluster_key = mc.cluster_key
                 WHERE mc.account_id = ?1
                 GROUP BY mc.cluster_key
                 ORDER BY member_count DESC",
            )
            .map_err(DbError::from)?;
        let rows = stmt
            .query_map(params![account_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)? as f32,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, i64>(5)? != 0,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(DbError::from)?;

        let mut out = Vec::new();
        for r in rows {
            let (
                cluster_key,
                category_s,
                confidence,
                member_count,
                oldest_date_ms,
                has_unsub,
                sample_sender,
                sample_subject,
            ) = r.map_err(DbError::from)?;
            let category = Category::parse(&category_s);
            let oldest_message_age_days =
                oldest_date_ms.map(|d| (now_ms - d) / (1000 * 60 * 60 * 24));
            let facts = ClusterFacts {
                cluster_key: cluster_key.clone(),
                category,
                confidence,
                member_count,
                oldest_message_age_days,
                has_list_unsubscribe: has_unsub,
            };
            let proposed = propose_actions(&facts, &cfg);
            let staged_action_types = staged_index
                .get(&cluster_key)
                .cloned()
                .unwrap_or_default();
            out.push(ReviewClusterEntry {
                cluster_key,
                category: category_s,
                confidence,
                member_count,
                oldest_message_age_days,
                has_list_unsubscribe: has_unsub,
                sample_sender,
                sample_subject,
                proposed,
                staged_action_types,
            });
        }
        Ok(out)
    })
}

fn staged_index(rows: &[StagedAction]) -> std::collections::HashMap<String, Vec<String>> {
    let mut out: std::collections::HashMap<String, Vec<String>> = Default::default();
    for r in rows {
        if r.provider_msg_id.is_none() {
            out.entry(r.cluster_key.clone())
                .or_default()
                .push(r.action_type.clone());
        }
    }
    out
}

fn parse_action(s: &str) -> Result<ActionType, String> {
    match s {
        "archive" => Ok(ActionType::Archive),
        "trash" => Ok(ActionType::Trash),
        "add_label" => Ok(ActionType::AddLabel),
        "remove_label" => Ok(ActionType::RemoveLabel),
        "mark_read" => Ok(ActionType::MarkRead),
        "unsubscribe" => Ok(ActionType::Unsubscribe),
        other => Err(format!("unknown action_type: {other}")),
    }
}
