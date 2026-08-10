use thiserror::Error;

/// 协议层错误。协议库保持无 IO：解密失败由调用方通过 `Option` 表达，
/// 仅序列化/反序列化与逻辑异常进入此类型。
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("envelope 编解码失败: {0}")]
    Codec(#[from] serde_json::Error),

    #[error("解密返回空（密钥错误或数据损坏）")]
    DecryptFailed,
}
