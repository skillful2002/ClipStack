use serde::{Deserialize, Serialize};

use crate::error::ProtocolError;

/// 信封 nonce 长度（字节）。
///
/// 复用 `crypto.rs` 的 AES-256-GCM，其 nonce 为 12 字节——与设计文档 §5 草图中的
/// `[u8;24]`（XChaCha20）不同。本库保持加密无关，但 nonce 长度是线上格式的一部分，
/// 必须与所选算法一致；若将来切回全量设计的 XChaCha20，此处改 24 即可。
pub const NONCE_LEN: usize = 12;

/// 剪贴板条目类型。与 `history.content_type` 对齐（text/link/code/image/file）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    Text,
    Link,
    Code,
    Image,
    File,
}

/// 线上信封：本机捕获后加密，再广播给 mesh 对端（全量设计为发往中继）。
///
/// `ciphertext` 为加密后的 `ClipboardItem`；`nonce` 随信封走，解密方据此还原。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEnvelope {
    /// 条目全局唯一 id。
    pub sync_id: String,
    /// 来源设备。
    pub device_id: String,
    /// 逻辑时钟，用于排序。
    pub lamport: u64,
    /// 条目类型。
    pub kind: ClipKind,
    /// 内容去重指纹（= sha256(payload)）。
    pub hash: String,
    /// 加密 nonce（长度 [NONCE_LEN]）。
    pub nonce: Vec<u8>,
    /// 加密后的 `ClipboardItem`。
    pub ciphertext: Vec<u8>,
}

/// 明文条目：信封解密后的内容。后端据此落库（并补充 `is_remote` / `profile_id` 等元数据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub sync_id: String,
    pub device_id: String,
    pub lamport: u64,
    pub kind: ClipKind,
    pub hash: String,
    /// 明文负载（后端负责序列化为具体存储格式）。
    pub payload: Vec<u8>,
}

impl SyncEnvelope {
    /// 序列化到传输字节（JSON）。前端/后端经 WebSocket 收发时即用此。
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        Ok(serde_json::to_vec(self)?)
    }

    /// 从传输字节反序列化。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

impl ClipboardItem {
    /// 等价于 `SyncEnvelope::hash` 的内容指纹：对明文负载取 sha256 十六进制。
    /// 本机捕获时调用，用于填写信封 `hash` 与去重。
    pub fn content_hash(payload: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(payload);
        hex_encode(&h.finalize())
    }
}

/// 将 32 字节摘要编码为小写十六进制（避免引入额外依赖，自实现）。
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let item = ClipboardItem {
            sync_id: "s1".into(),
            device_id: "devA".into(),
            lamport: 3,
            kind: ClipKind::Text,
            hash: "abc".into(),
            payload: b"hello".to_vec(),
        };
        let env = SyncEnvelope {
            sync_id: item.sync_id.clone(),
            device_id: item.device_id.clone(),
            lamport: item.lamport,
            kind: item.kind,
            hash: item.hash.clone(),
            nonce: vec![0u8; NONCE_LEN],
            ciphertext: b"enc".to_vec(),
        };
        let bytes = env.to_bytes().unwrap();
        let back = SyncEnvelope::from_bytes(&bytes).unwrap();
        assert_eq!(back.sync_id, env.sync_id);
        assert_eq!(back.ciphertext, env.ciphertext);
        // item 自身编解码
        let ib = serde_json::to_vec(&item).unwrap();
        let rit: ClipboardItem = serde_json::from_slice(&ib).unwrap();
        assert_eq!(rit.payload, item.payload);
    }

    #[test]
    fn content_hash_stable_and_distinct() {
        let a = ClipboardItem::content_hash(b"same");
        let b = ClipboardItem::content_hash(b"same");
        let c = ClipboardItem::content_hash(b"different");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64); // sha256 hex
    }
}
