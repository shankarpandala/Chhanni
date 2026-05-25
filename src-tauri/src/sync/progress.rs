use serde::Serialize;

#[derive(Copy, Clone, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncStage {
    Listing,
    Fetching,
    Incremental,
    Done,
}

#[derive(Clone, Debug, Serialize)]
pub struct SyncProgress {
    pub account_id: String,
    pub stage: SyncStage,
    pub messages_seen: u64,
    pub messages_persisted: u64,
    pub elapsed_ms: u64,
}
