//! ClipStack 共享同步协议库（纯逻辑，无 IO）。
//!
//! 与全量设计 [`clipstack-sync-design.md`] 的协议层完全一致：信封、去重、Lamport 排序、
//! 回环防护。差异只在「路由方式」——全量发往中继，局域网方案广播给 mesh 对端。
//! 加密细节由调用方注入，本库不直接依赖 `crypto.rs`，便于独立单测。

pub mod clock;
pub mod dedup;
pub mod envelope;
pub mod error;
pub mod store;

pub use clock::{LamportClock, sort_key};
pub use dedup::DedupSet;
pub use envelope::{ClipKind, ClipboardItem, NONCE_LEN, SyncEnvelope};
pub use error::ProtocolError;
pub use store::{ClipStore, IngestOutcome, ReceivedClip};
