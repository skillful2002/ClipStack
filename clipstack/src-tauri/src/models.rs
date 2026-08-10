// P2 · 数据模型（与前端 / 持久化共用）
//
// 约定：
//   - Rust 端字段用 snake_case；序列化给前端的 JSON 用 `rename_all = "camelCase"`，
//     前端无需额外转换即可拿到驼峰字段。
//   - ContentType 同时用于序列化（写入 SQLite 的 TEXT 列、广播事件、命令参数）。

use serde::{Deserialize, Serialize};

/// 内容类型（文本 / 链接 / 图片 / 代码 / 文件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Text,
    Link,
    Code,
    Image,
    File,
}

impl ContentType {
    /// 存储 / 日志用的小写字符串。
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentType::Text => "text",
            ContentType::Link => "link",
            ContentType::Code => "code",
            ContentType::Image => "image",
            ContentType::File => "file",
        }
    }

    /// 从存储字符串解析；未知值回退为 Text（保证历史数据健壮）。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "text" => Some(ContentType::Text),
            "link" => Some(ContentType::Link),
            "code" => Some(ContentType::Code),
            "image" => Some(ContentType::Image),
            "file" => Some(ContentType::File),
            _ => None,
        }
    }
}

/// 历史条目（持久化行 + 事件负载 + 命令返回的同一结构体）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: i64,
    pub content_type: ContentType,
    /// 文本 / 链接 / 代码：原文；文件：路径拼接；图片："WxH 图片"。
    pub content_text: String,
    /// 展示用预览（长文本截断到预览上限）。
    pub preview: String,
    pub source_app: String,
    pub size_bytes: i64,
    pub hash: String,
    pub is_pinned: bool,
    pub is_favorite: bool,
    /// 是否命中敏感内容识别（启用掩码时预览被遮挡，原文仍加密存储）。
    pub is_sensitive: bool,
    /// 毫秒时间戳。
    pub created_at: i64,
    /// 来源设备名：本地捕获为空字符串；来自局域网共享的对端条目填对端设备名。
    /// 用于历史列表标注「本机 / 某设备」，便于区分共享内容来源。
    #[serde(default)]
    pub origin_device: String,
    /// 是否来自局域网共享（对端设备）；本地捕获为 false。
    #[serde(default)]
    pub is_remote: bool,
    /// 删除时间（仅回收站条目有值；主列表为 None，以便同一结构体复用）。
    #[serde(default)]
    pub deleted_at: Option<i64>,
}

/// 新增条目入参（命令 `add_item` 与监控线程内部共用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewItem {
    pub content_type: ContentType,
    pub content_text: String,
    /// 图片二进制（PNG）；文件则为 JSON 路径数组（不含文件内容）；文本 / 代码 / 链接为 None。
    pub content_blob: Option<Vec<u8>>,
    pub source_app: String,
    pub size_bytes: i64,
    pub hash: String,
    pub created_at: i64,
    /// 是否命中敏感内容识别（写库时由捕获线程计算）。
    pub is_sensitive: bool,
}

/// 设置项（key / value 均为字符串，前端 / 后端自行解释）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub key: String,
    pub value: String,
}
