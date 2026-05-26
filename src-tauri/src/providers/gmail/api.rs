use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::error::{SyncError, SyncResult};

const API_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const PAGE_SIZE: u32 = 500;
const RETRY_MAX: u32 = 4;

#[derive(Clone, Debug, Deserialize)]
pub struct MessageRef {
    pub id: String,
    #[serde(rename = "threadId")]
    pub thread_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListMessagesPage {
    #[serde(default)]
    pub messages: Vec<MessageRef>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: Option<String>,
    #[serde(rename = "resultSizeEstimate", default)]
    pub result_size_estimate: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MessageMetadata {
    pub id: String,
    #[serde(rename = "threadId")]
    pub thread_id: String,
    #[serde(default, rename = "labelIds")]
    pub label_ids: Vec<String>,
    pub snippet: Option<String>,
    #[serde(rename = "internalDate")]
    pub internal_date: String, // gmail returns as string of ms
    #[serde(rename = "historyId")]
    pub history_id: String,
    pub payload: Option<MessagePayload>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MessagePayload {
    #[serde(default)]
    pub headers: Vec<Header>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

impl MessageMetadata {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.payload
            .as_ref()?
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    pub fn internal_date_ms(&self) -> i64 {
        self.internal_date.parse::<i64>().unwrap_or(0)
    }

    pub fn history_id_u64(&self) -> u64 {
        self.history_id.parse::<u64>().unwrap_or(0)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryPage {
    #[serde(default)]
    pub history: Vec<HistoryRecord>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: Option<String>,
    #[serde(rename = "historyId")]
    pub history_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryRecord {
    pub id: String,
    #[serde(default, rename = "messagesAdded")]
    pub messages_added: Vec<HistoryMessageEntry>,
    #[serde(default, rename = "messagesDeleted")]
    pub messages_deleted: Vec<HistoryMessageEntry>,
    #[serde(default, rename = "labelsAdded")]
    pub labels_added: Vec<HistoryLabelEntry>,
    #[serde(default, rename = "labelsRemoved")]
    pub labels_removed: Vec<HistoryLabelEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryMessageEntry {
    pub message: MessageRef,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryLabelEntry {
    pub message: MessageRef,
    #[serde(default, rename = "labelIds")]
    pub label_ids: Vec<String>,
}

/// The trait the sync engine talks to. Implemented by the real `GmailClient`
/// and by test doubles in unit tests.
#[async_trait]
pub trait GmailApi: Send + Sync {
    async fn list_messages(
        &self,
        page_token: Option<&str>,
        max_results: u32,
    ) -> SyncResult<ListMessagesPage>;

    async fn get_metadata(&self, id: &str) -> SyncResult<MessageMetadata>;

    async fn list_history(
        &self,
        start_history_id: u64,
        page_token: Option<&str>,
    ) -> SyncResult<HistoryPage>;
}

#[derive(Clone)]
pub struct GmailClient {
    http: reqwest::Client,
    access_token: String,
    base: String,
}

impl GmailClient {
    pub fn new(http: reqwest::Client, access_token: String) -> Self {
        Self {
            http,
            access_token,
            base: API_BASE.to_owned(),
        }
    }

    pub fn with_base(http: reqwest::Client, access_token: String, base: String) -> Self {
        Self { http, access_token, base }
    }

    async fn get_with_retry(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> SyncResult<serde_json::Value> {
        let url = format!("{}{}", self.base, path);
        let mut delay = Duration::from_millis(500);
        let mut last_err: Option<SyncError> = None;

        for attempt in 1..=RETRY_MAX {
            let resp = self
                .http
                .get(&url)
                .bearer_auth(&self.access_token)
                .query(query)
                .send()
                .await;

            match resp {
                Ok(r) => {
                    let status = r.status();
                    if status.is_success() {
                        return r
                            .json::<serde_json::Value>()
                            .await
                            .map_err(SyncError::Gmail);
                    }
                    let body = r.text().await.unwrap_or_default();
                    if should_retry(status, &body) && attempt < RETRY_MAX {
                        tracing::warn!(attempt, status = status.as_u16(), "gmail retryable error");
                        tokio::time::sleep(delay).await;
                        delay *= 2;
                        continue;
                    }
                    return Err(SyncError::GmailStatus { status: status.as_u16(), body });
                }
                Err(e) if attempt < RETRY_MAX => {
                    tracing::warn!(attempt, error = %e, "gmail network error; retrying");
                    last_err = Some(SyncError::Gmail(e));
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                Err(e) => return Err(SyncError::Gmail(e)),
            }
        }

        Err(last_err.unwrap_or(SyncError::GmailStatus {
            status: 0,
            body: "retry loop exhausted".to_owned(),
        }))
    }
}

fn is_retryable(status: StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error()
}

/// Gmail signals per-minute quota exhaustion as `403` with
/// `reason: "rateLimitExceeded"` (or its newer `RATE_LIMIT_EXCEEDED` form)
/// rather than a `429`. Treat those exactly like a `429` so the existing
/// exponential backoff lets the quota window roll over.
fn should_retry(status: StatusCode, body: &str) -> bool {
    if is_retryable(status) {
        return true;
    }
    status.as_u16() == 403
        && (body.contains("rateLimitExceeded") || body.contains("RATE_LIMIT_EXCEEDED"))
}

#[async_trait]
impl GmailApi for GmailClient {
    async fn list_messages(
        &self,
        page_token: Option<&str>,
        max_results: u32,
    ) -> SyncResult<ListMessagesPage> {
        let mut query = vec![("maxResults".to_owned(), max_results.to_string())];
        // We default to inbox-wide; sync layer can narrow with `q=` later.
        if let Some(tok) = page_token {
            query.push(("pageToken".to_owned(), tok.to_owned()));
        }
        let query_refs: Vec<(&str, String)> =
            query.into_iter().map(|(k, v)| (str_leak(k), v)).collect();
        let v = self.get_with_retry("/messages", &query_refs).await?;
        serde_json::from_value(v).map_err(SyncError::GmailMalformed)
    }

    async fn get_metadata(&self, id: &str) -> SyncResult<MessageMetadata> {
        let path = format!("/messages/{id}");
        let query = vec![
            ("format", "METADATA".to_owned()),
            ("metadataHeaders", "From".to_owned()),
            ("metadataHeaders", "Subject".to_owned()),
            ("metadataHeaders", "List-Unsubscribe".to_owned()),
        ];
        let v = self.get_with_retry(&path, &query).await?;
        serde_json::from_value(v).map_err(SyncError::GmailMalformed)
    }

    async fn list_history(
        &self,
        start_history_id: u64,
        page_token: Option<&str>,
    ) -> SyncResult<HistoryPage> {
        let mut query: Vec<(&str, String)> = vec![
            ("startHistoryId", start_history_id.to_string()),
            ("maxResults", "500".to_owned()),
        ];
        if let Some(tok) = page_token {
            query.push(("pageToken", tok.to_owned()));
        }
        let v = self.get_with_retry("/history", &query).await?;
        serde_json::from_value(v).map_err(SyncError::GmailMalformed)
    }
}

// reqwest's query() wants `&[(&str, T)]`; this helper lets us mix owned
// keys with owned values without allocating again. We don't use it on hot
// paths, just to keep the call sites tidy.
fn str_leak(s: String) -> &'static str {
    // We never call this in hot loops; only for one-off page-token query keys
    // built from owned strings. Acceptable to leak a handful of bytes per sync.
    Box::leak(s.into_boxed_str())
}

pub fn page_size_default() -> u32 {
    PAGE_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_metadata_header_lookup_is_case_insensitive() {
        let m = MessageMetadata {
            id: "1".into(),
            thread_id: "t".into(),
            label_ids: vec![],
            snippet: None,
            internal_date: "1700000000000".into(),
            history_id: "42".into(),
            payload: Some(MessagePayload {
                headers: vec![
                    Header {
                        name: "From".into(),
                        value: "a@b.com".into(),
                    },
                    Header {
                        name: "Subject".into(),
                        value: "hi".into(),
                    },
                ],
            }),
        };
        assert_eq!(m.header("from"), Some("a@b.com"));
        assert_eq!(m.header("Subject"), Some("hi"));
        assert_eq!(m.header("missing"), None);
    }

    #[test]
    fn message_metadata_parses_numeric_fields() {
        let m = MessageMetadata {
            id: "1".into(),
            thread_id: "t".into(),
            label_ids: vec![],
            snippet: None,
            internal_date: "1700000000000".into(),
            history_id: "42".into(),
            payload: None,
        };
        assert_eq!(m.internal_date_ms(), 1_700_000_000_000);
        assert_eq!(m.history_id_u64(), 42);
    }

    #[test]
    fn is_retryable_matches_429_and_5xx() {
        assert!(is_retryable(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable(StatusCode::BAD_GATEWAY));
        assert!(!is_retryable(StatusCode::OK));
        assert!(!is_retryable(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn should_retry_catches_403_rate_limit() {
        // The actual 403 body Gmail returns when the per-minute quota is hit.
        let body = r#"{"error":{"code":403,"errors":[{"reason":"rateLimitExceeded"}]}}"#;
        assert!(should_retry(StatusCode::FORBIDDEN, body));

        // The newer Google RPC ErrorInfo phrasing.
        let body2 = r#"{"error":{"details":[{"reason":"RATE_LIMIT_EXCEEDED"}]}}"#;
        assert!(should_retry(StatusCode::FORBIDDEN, body2));

        // A non-rate-limit 403 (e.g. insufficient scope) must NOT loop forever.
        let body3 =
            r#"{"error":{"code":403,"errors":[{"reason":"insufficientPermissions"}]}}"#;
        assert!(!should_retry(StatusCode::FORBIDDEN, body3));

        // 429 still retries regardless of body.
        assert!(should_retry(StatusCode::TOO_MANY_REQUESTS, ""));
    }
}
