use std::collections::HashSet;

use crate::envelope::SyncEnvelope;

/// 去重集合：按 `hash`（内容指纹）与 `sync_id`（全局唯一 id）双键去重。
///
/// 设计文档 §5：去重靠 `hash` / `sync_id`。同一内容被不同设备编辑后重发、或同一条目
/// 经 mesh 多路径到达，均视为重复，只处理一次。
#[derive(Debug, Default, Clone)]
pub struct DedupSet {
    hashes: HashSet<String>,
    ids: HashSet<String>,
}

impl DedupSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否已处理过该信封（按 hash 或 sync_id 任一命中即判重）。
    pub fn is_duplicate(&self, env: &SyncEnvelope) -> bool {
        self.hashes.contains(&env.hash) || self.ids.contains(&env.sync_id)
    }

    /// 记录已处理；返回此前是否已是重复（true=本次为新增，false=本就是重复）。
    pub fn mark(&mut self, env: &SyncEnvelope) -> bool {
        let was_new = !self.is_duplicate(env);
        self.hashes.insert(env.hash.clone());
        self.ids.insert(env.sync_id.clone());
        was_new
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{ClipKind, SyncEnvelope};

    fn env(id: &str, hash: &str) -> SyncEnvelope {
        SyncEnvelope {
            sync_id: id.into(),
            device_id: "devA".into(),
            lamport: 1,
            kind: ClipKind::Text,
            hash: hash.into(),
            nonce: vec![0u8; 12],
            ciphertext: vec![1, 2, 3],
        }
    }

    #[test]
    fn marks_and_detects_duplicate() {
        let mut d = DedupSet::new();
        let e = env("s1", "h1");
        assert!(!d.is_duplicate(&e));
        assert!(d.mark(&e)); // 新增
        assert!(d.is_duplicate(&e));
        assert!(!d.mark(&e)); // 已是重复
    }

    #[test]
    fn hash_or_id_either_matches() {
        let mut d = DedupSet::new();
        d.mark(&env("s1", "h1"));
        // 同 hash 不同 id -> 判重
        assert!(d.is_duplicate(&env("s2", "h1")));
        // 同 id 不同 hash -> 判重
        assert!(d.is_duplicate(&env("s1", "hx")));
        // 都不同 -> 不重
        assert!(!d.is_duplicate(&env("s9", "h9")));
    }
}
