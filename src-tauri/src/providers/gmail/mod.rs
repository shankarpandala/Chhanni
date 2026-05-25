pub mod api;
pub mod parse;

pub use api::{
    GmailApi, GmailClient, Header, HistoryMessageEntry, HistoryPage, HistoryRecord,
    ListMessagesPage, MessageMetadata, MessagePayload, MessageRef,
};
pub use parse::parse_sender_email;
