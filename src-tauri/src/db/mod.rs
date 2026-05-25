pub mod clusters;
pub mod connection;
pub mod embeddings;
pub mod messages;
pub mod sync_state;

pub use clusters::{ClusterMember, ClusterSummary, ClustersRepo};
pub use connection::{open_in_memory, open_with_path, Db};
pub use embeddings::{EmbeddingRow, EmbeddingsRepo};
pub use messages::{MessageRow, MessagesRepo};
pub use sync_state::{SyncPhase, SyncState, SyncStateRepo};

pub(crate) mod embedded {
    refinery::embed_migrations!("migrations");
}
