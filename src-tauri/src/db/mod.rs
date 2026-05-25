pub mod connection;
pub mod messages;
pub mod sync_state;

pub use connection::{open_in_memory, open_with_path, Db};
pub use messages::{MessageRow, MessagesRepo};
pub use sync_state::{SyncPhase, SyncState, SyncStateRepo};

pub(crate) mod embedded {
    refinery::embed_migrations!("migrations");
}
