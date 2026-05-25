pub mod api;
pub mod mutations;
pub mod parse;

pub use api::{
    GmailApi, GmailClient, Header, HistoryMessageEntry, HistoryPage, HistoryRecord,
    ListMessagesPage, MessageMetadata, MessagePayload, MessageRef,
};
pub use mutations::{GmailMutations, GmailMutationsClient};
pub use parse::parse_sender_email;
