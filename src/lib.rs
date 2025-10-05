// Library exports for testing

pub mod input;
pub mod navigation;
pub mod net;
pub mod nns;
pub mod storage;

// Re-export commonly used types for tests
pub use net::{NostrClient, RelayDirectory};
pub use nns::NnsResolver;
pub use storage::Storage;
