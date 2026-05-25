pub mod bootstrap;
pub mod client;
pub mod download;
pub mod lifecycle;
pub mod release;

pub use bootstrap::{Bootstrapper, BootstrapProgress};
pub use client::{EmbeddingClient, HttpSidecarClient};
pub use download::{download_resumable, DownloadProgress, DownloadSpec};
pub use lifecycle::{SidecarHandle, SidecarManager, SidecarSpec};
