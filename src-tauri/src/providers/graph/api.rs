//! Microsoft Graph `/me/messages` client.
//!
//! Graph paginates with `@odata.nextLink` and gives a `@odata.deltaLink` at
//! the end of a `/delta` walk. Both are absolute URLs — we follow them
//! verbatim. The first sync uses `/me/messages/delta`; subsequent syncs
//! follow the stored deltaLink.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;
use tracing::warn;

use crate::error::{SyncError, SyncResult};

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
const DELTA_PATH: &str = "/me/messages/delta";
const PAGE_FIELDS: &str = "id,internetMessageId,conversationId,subject,bodyPreview,\
                          from,sender,toRecipients,isRead,categories,\
                          parentFolderId,receivedDateTime,internetMessageHeaders";
const RETRY_MAX: u32 = 4;

#[derive(Clone, Debug, Deserialize)]
pub struct GraphMessage {
    pub id: String,
    #[serde(rename = "conversationId")]
    pub conversation_id: Option<String>,
    pub subject: Option<String>,
    #[serde(rename = "bodyPreview")]
    pub body_preview: Option<String>,
    #[serde(rename = "from")]
    pub from: Option<EmailField>,
    #[serde(rename = "receivedDateTime")]
    pub received_date_time: Option<String>,
    #[serde(rename = "parentFolderId")]
    pub parent_folder_id: Option<String>,
    #[serde(rename = "isRead", default)]
    pub is_read: bool,
    #[serde(default)]
    pub categories: Vec<String>,
    /// Marker rows in the delta feed: `@removed: { "reason": "..." }`.
    #[serde(default, rename = "@removed")]
    pub removed: Option<serde_json::Value>,
}

impl GraphMessage {
    pub fn is_removed(&self) -> bool {
        self.removed.is_some()
    }
    pub fn from_email(&self) -> Option<&str> {
        self.from
            .as_ref()
            .and_then(|f| f.email_address.as_ref())
            .and_then(|a| a.address.as_deref())
    }
    pub fn from_name(&self) -> Option<&str> {
        self.from
            .as_ref()
            .and_then(|f| f.email_address.as_ref())
            .and_then(|a| a.name.as_deref())
    }
    pub fn received_unix_ms(&self) -> i64 {
        self.received_date_time
            .as_deref()
            .and_then(|s| time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok())
            .map(|d| d.unix_timestamp() * 1000)
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct EmailField {
    #[serde(rename = "emailAddress")]
    pub email_address: Option<EmailAddress>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeltaPage {
    #[serde(default)]
    pub value: Vec<GraphMessage>,
    #[serde(default, rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(default, rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

#[async_trait]
pub trait GraphApi: Send + Sync {
    /// First sync: pass `None` for `delta_link`. Subsequent syncs: pass the
    /// stored delta link verbatim.
    async fn fetch_delta(&self, delta_link: Option<&str>) -> SyncResult<DeltaPage>;
    /// Follow a `@odata.nextLink` mid-walk.
    async fn fetch_next(&self, next_link: &str) -> SyncResult<DeltaPage>;
}

#[derive(Clone)]
pub struct GraphClient {
    http: reqwest::Client,
    access_token: String,
    base: String,
}

impl GraphClient {
    pub fn new(http: reqwest::Client, access_token: String) -> Self {
        Self {
            http,
            access_token,
            base: GRAPH_BASE.to_owned(),
        }
    }

    pub fn with_base(http: reqwest::Client, access_token: String, base: String) -> Self {
        Self { http, access_token, base }
    }

    async fn get_with_retry(&self, url: &str) -> SyncResult<DeltaPage> {
        let mut delay = Duration::from_millis(500);
        for attempt in 1..=RETRY_MAX {
            let resp = self
                .http
                .get(url)
                .bearer_auth(&self.access_token)
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let status = r.status();
                    if status.is_success() {
                        return r.json::<DeltaPage>().await.map_err(SyncError::Gmail);
                    }
                    let body = r.text().await.unwrap_or_default();
                    if is_retryable(status) && attempt < RETRY_MAX {
                        warn!(attempt, status = status.as_u16(), "graph retryable");
                        tokio::time::sleep(delay).await;
                        delay *= 2;
                        continue;
                    }
                    return Err(SyncError::GmailStatus { status: status.as_u16(), body });
                }
                Err(e) if attempt < RETRY_MAX => {
                    warn!(attempt, error = %e, "graph network error");
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                Err(e) => return Err(SyncError::Gmail(e)),
            }
        }
        Err(SyncError::GmailStatus { status: 0, body: "retry exhausted".into() })
    }
}

fn is_retryable(status: StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error()
}

#[async_trait]
impl GraphApi for GraphClient {
    async fn fetch_delta(&self, delta_link: Option<&str>) -> SyncResult<DeltaPage> {
        let url = match delta_link {
            Some(link) => link.to_owned(),
            None => format!("{}{}?$select={}", self.base, DELTA_PATH, PAGE_FIELDS),
        };
        self.get_with_retry(&url).await
    }

    async fn fetch_next(&self, next_link: &str) -> SyncResult<DeltaPage> {
        self.get_with_retry(next_link).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_message_extracts_from_email() {
        let raw = serde_json::json!({
            "id": "m1",
            "from": { "emailAddress": { "name": "X", "address": "x@example.com" } }
        });
        let m: GraphMessage = serde_json::from_value(raw).unwrap();
        assert_eq!(m.from_email(), Some("x@example.com"));
        assert_eq!(m.from_name(), Some("X"));
    }

    #[test]
    fn detects_removed_marker() {
        let raw = serde_json::json!({
            "id": "m1",
            "@removed": { "reason": "deleted" }
        });
        let m: GraphMessage = serde_json::from_value(raw).unwrap();
        assert!(m.is_removed());
    }

    #[test]
    fn parses_received_date_to_ms() {
        let raw = serde_json::json!({
            "id": "m1",
            "receivedDateTime": "2026-01-01T00:00:00Z"
        });
        let m: GraphMessage = serde_json::from_value(raw).unwrap();
        assert_eq!(m.received_unix_ms(), 1_767_225_600_000);
    }

    #[test]
    fn is_retryable_matrix() {
        assert!(is_retryable(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!is_retryable(StatusCode::OK));
        assert!(!is_retryable(StatusCode::FORBIDDEN));
    }
}
