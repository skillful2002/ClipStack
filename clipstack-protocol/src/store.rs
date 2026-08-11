use crate::clock::{sort_key, LamportClock};
use crate::dedup::DedupSet;
use crate::envelope::{ClipboardItem, SyncEnvelope};
use crate::error::ProtocolError;

/// 收到的条目（含来源标记）。`is_remote` 为 true 表示来自共享对端，
/// 落库时写 `history(is_remote=1)`，且**不向第三方转发**（回环/放大防护）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedClip {
    pub item: ClipboardItem,
    pub is_remote: bool,
}

/// `ingest` 的结果，区分「已入库 / 重复 / 回环」三种情形。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    /// 首次入库（来自对端共享）。
    Stored(ReceivedClip),
    /// 已处理过（按 hash 或 sync_id 命中），丢弃。
    Duplicate,
    /// 检测到回环：信封来源是本机自己，丢弃（不入库、不转发）。
    Loopback,
}

/// 接收端缓冲：去重 + 排序 + 回环防护，保持无 IO（解密由调用方注入）。
///
/// 设计文档 §5：收到对端 `Clip` → 解密 → 按 `hash`/`sync_id` 去重 → 写库（`is_remote=1`）
/// → 不向第三方转发。本结构即「解密后写库前」的那一跳。
#[derive(Debug)]
pub struct ClipStore {
    items: Vec<ReceivedClip>,
    seen: DedupSet,
    self_device_id: String,
    clock: LamportClock,
}

impl ClipStore {
    pub fn new(self_device_id: impl Into<String>) -> Self {
        Self {
            items: Vec::new(),
            seen: DedupSet::new(),
            self_device_id: self_device_id.into(),
            clock: LamportClock::new(),
        }
    }

    /// 设置/更新本机设备 id（用于回环检测）。
    pub fn set_self_device(&mut self, id: impl Into<String>) {
        self.self_device_id = id.into();
    }

    /// 当前 Lamport 时钟（可用于本地捕获时取序）。
    pub fn clock(&self) -> &LamportClock {
        &self.clock
    }

    /// 处理一个信封。
    ///
    /// - `decrypt`：注入的解密函数 `(SyncEnvelope) -> Option<Vec<u8>>`，返回明文 `payload`；
    ///   返回 `None` 表示解密失败（密钥不符/损坏），按 [`ProtocolError::DecryptFailed`] 处理。
    /// - 回环：若 `env.device_id == self_device_id`，直接判 [`IngestOutcome::Loopback`]。
    /// - 去重：若已处理过，返回 [`IngestOutcome::Duplicate`]。
    /// - 否则解密、推进 Lamport 时钟（observe remote）、入库，返回 [`IngestOutcome::Stored`]。
    pub fn ingest<F>(
        &mut self,
        env: &SyncEnvelope,
        decrypt: F,
    ) -> Result<IngestOutcome, ProtocolError>
    where
        F: FnOnce(&SyncEnvelope) -> Option<Vec<u8>>,
    {
        // 1) 回环防护：自己的广播绕回自己，丢弃。
        if env.device_id == self.self_device_id {
            return Ok(IngestOutcome::Loopback);
        }
        // 2) 去重：已处理过则丢弃。
        if self.seen.is_duplicate(env) {
            return Ok(IngestOutcome::Duplicate);
        }
        // 3) 解密明文 payload。
        let payload = decrypt(env).ok_or(ProtocolError::DecryptFailed)?;
        // 4) 推进 Lamport 时钟（以对端 lamport 为基准）。
        self.clock.observe(env.lamport);
        // 5) 记录已处理并入库。
        self.seen.mark(env);
        let item = ClipboardItem {
            sync_id: env.sync_id.clone(),
            device_id: env.device_id.clone(),
            source_app: env.source_app.clone(),
            lamport: env.lamport,
            kind: env.kind,
            hash: env.hash.clone(),
            payload,
        };
        let received = ReceivedClip {
            item: item.clone(),
            is_remote: true,
        };
        self.items.push(received.clone());
        Ok(IngestOutcome::Stored(received))
    }

    /// 条数。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 按 `(lamport, device_id)` 升序（最旧在前）。后端通常倒序展示，自行 `rev()`。
    pub fn sorted_oldest_first(&self) -> Vec<&ReceivedClip> {
        let mut v: Vec<&ReceivedClip> = self.items.iter().collect();
        v.sort_by_key(|r| sort_key(r.item.lamport, &r.item.device_id));
        v
    }

    /// 按 `(lamport, device_id)` 降序（最新在前），与历史列表展示一致。
    pub fn sorted_newest_first(&self) -> Vec<&ReceivedClip> {
        let mut v = self.sorted_oldest_first();
        v.reverse();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{ClipKind, SyncEnvelope};

    /// 测试用「假加密」：直接将 payload 当密文，解密原样返回。
    fn fake_encrypt(payload: &[u8]) -> Vec<u8> {
        payload.to_vec()
    }
    fn fake_decrypt(env: &SyncEnvelope) -> Option<Vec<u8>> {
        Some(env.ciphertext.clone())
    }

    fn make_env(id: &str, dev: &str, lamport: u64, payload: &[u8]) -> SyncEnvelope {
        SyncEnvelope {
            sync_id: id.into(),
            device_id: dev.into(),
            source_app: String::new(),
            lamport,
            kind: ClipKind::Text,
            hash: ClipboardItem::content_hash(payload),
            nonce: vec![0u8; 12],
            ciphertext: fake_encrypt(payload),
        }
    }

    #[test]
    fn ingest_remote_stores_and_dedups() {
        let mut store = ClipStore::new("devA"); // 本机 devA
        let env = make_env("s1", "devB", 5, b"from B");
        match store.ingest(&env, fake_decrypt).unwrap() {
            IngestOutcome::Stored(r) => assert_eq!(r.item.payload, b"from B"),
            _ => panic!("expected Stored"),
        }
        assert_eq!(store.len(), 1);
        // 重复到达
        match store.ingest(&env, fake_decrypt).unwrap() {
            IngestOutcome::Duplicate => {}
            _ => panic!("expected Duplicate"),
        }
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn loopback_is_dropped() {
        let mut store = ClipStore::new("devA"); // 本机 devA
        let env = make_env("s1", "devA", 1, b"my own"); // 来源也是 devA
        match store.ingest(&env, fake_decrypt).unwrap() {
            IngestOutcome::Loopback => {}
            other => panic!("expected Loopback, got {other:?}"),
        }
        assert!(store.is_empty());
    }

    #[test]
    fn decrypt_failure_is_error() {
        let mut store = ClipStore::new("devA");
        let env = make_env("s1", "devB", 1, b"x");
        let bad = |_: &SyncEnvelope| None::<Vec<u8>>;
        assert!(matches!(
            store.ingest(&env, bad),
            Err(ProtocolError::DecryptFailed)
        ));
    }

    /// 双端内存模拟：A 捕获 → 加密信封 → 广播给 B 的 store → B 入库，
    /// 且 A 的广播绕回 A 自身被判回环。
    #[test]
    fn two_endpoint_memory_sim() {
        let mut store_a = ClipStore::new("devA");
        let mut store_b = ClipStore::new("devB");

        let env = make_env("s1", "devA", 7, b"clipboard payload");

        // A 自己收到（回环）
        assert!(matches!(
            store_a.ingest(&env, fake_decrypt).unwrap(),
            IngestOutcome::Loopback
        ));
        // B 收到（入库）
        match store_b.ingest(&env, fake_decrypt).unwrap() {
            IngestOutcome::Stored(r) => assert_eq!(r.item.payload, b"clipboard payload"),
            _ => panic!("B should store"),
        }
        assert_eq!(store_b.len(), 1);
        // B 再次收到同一信封（去重）
        assert!(matches!(
            store_b.ingest(&env, fake_decrypt).unwrap(),
            IngestOutcome::Duplicate
        ));
    }

    #[test]
    fn ordering_newest_first() {
        let mut store = ClipStore::new("devA");
        store
            .ingest(&make_env("s1", "devB", 3, b"old"), fake_decrypt)
            .unwrap();
        store
            .ingest(&make_env("s2", "devB", 9, b"new"), fake_decrypt)
            .unwrap();
        let newest = store.sorted_newest_first();
        assert_eq!(newest.len(), 2);
        assert_eq!(newest[0].item.payload, b"new");
        assert_eq!(newest[1].item.payload, b"old");
    }

    #[test]
    fn lamport_advances_on_observe() {
        let mut store = ClipStore::new("devA");
        assert_eq!(store.clock().current(), 0);
        store
            .ingest(&make_env("s1", "devB", 10, b"x"), fake_decrypt)
            .unwrap();
        assert_eq!(store.clock().current(), 11); // max(0,10)+1
    }
}
