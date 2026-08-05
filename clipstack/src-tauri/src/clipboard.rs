// P1 · 剪贴板捕获引擎
//
// 流程：后台线程检测剪贴板变更 → 读取内容 → 类型识别 → hash 去重
//       → 来源应用过滤（忽略列表）→ 通过 Tauri 事件 `clipboard-changed` 广播。
//
// 关于「事件驱动」：
//   - Windows 有系统级 `WM_CLIPBOARDUPDATE` 事件（后续阶段接入 `AddClipboardFormatListener`）。
//   - macOS 没有公开的剪贴板变更通知，业界（Maccy / Paste 等）均通过轮询
//     `NSPasteboard.changeCount` 实现。此处采用 300ms 间隔的 changeCount 比对——
//     这是一次整数比较，绝非忙等循环，符合「不阻塞、不空转」的设计意图。

use std::collections::{hash_map::DefaultHasher, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arboard::Clipboard;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// 去重时间窗：同一 hash 在此窗口内再次出现视为重复，不再广播。
const DEDUP_WINDOW: Duration = Duration::from_secs(2);
/// 最近 hash 队列容量（仅用于会话内去重，不影响持久化）。
const DEDUP_CAPACITY: usize = 32;
/// 预览文本最大字符数。
const PREVIEW_MAX: usize = 200;
/// changeCount 轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// 内容类型（与前端 / 持久化共用，序列化为小写字符串）。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Text,
    Link,
    Code,
    Image,
    File,
}

/// 广播给前端的剪贴板条目快照。
#[derive(Debug, Clone, Serialize)]
pub struct ClipboardItem {
    pub id: String,
    pub content_type: ContentType,
    pub preview: String,
    pub hash: u64,
    pub source: String,
    pub size: usize,
    pub timestamp: i64,
}

/// 内部原始内容（用于分类与去重，不对外序列化）。
enum RawContent {
    None,
    Text(String),
    Image { width: usize, height: usize, bytes: Vec<u8> },
    Files(Vec<PathBuf>),
}

impl Hash for RawContent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            RawContent::None => 0u8.hash(state),
            RawContent::Text(s) => {
                1u8.hash(state);
                s.hash(state);
            }
            RawContent::Image {
                width,
                height,
                bytes,
            } => {
                2u8.hash(state);
                width.hash(state);
                height.hash(state);
                bytes.hash(state);
            }
            RawContent::Files(p) => {
                3u8.hash(state);
                p.hash(state);
            }
        }
    }
}

/// 监控共享状态：去重队列 + 忽略应用集合。由 Tauri 托管，供命令与监控线程共享。
pub struct MonitorState {
    recent: VecDeque<(u64, Instant)>,
    ignored: HashSet<String>,
}

impl Default for MonitorState {
    fn default() -> Self {
        Self {
            recent: VecDeque::with_capacity(DEDUP_CAPACITY),
            ignored: HashSet::new(),
        }
    }
}

/// 启动后台监控线程。
pub fn start_monitor(app: AppHandle, state: Arc<Mutex<MonitorState>>) {
    std::thread::spawn(move || {
        // 记录启动时刻的 changeCount，避免把「当前已存在的内容」当作一次新捕获。
        let mut last_count = current_change_count().unwrap_or(-1);
        loop {
            std::thread::sleep(POLL_INTERVAL);
            match current_change_count() {
                Some(count) if count != last_count => {
                    last_count = count;
                    if let Some(item) = capture(&state) {
                        let _ = app.emit("clipboard-changed", &item);
                        println!(
                            "[clipstack] captured {:?} hash={:016x} source={} size={}",
                            item.content_type, item.hash, item.source, item.size
                        );
                    }
                }
                _ => {}
            }
        }
    });
}

/// 读取 + 分类 + 去重 + 来源过滤，产出可广播的条目；被忽略 / 去重 / 无内容时返回 None。
fn capture(state: &Arc<Mutex<MonitorState>>) -> Option<ClipboardItem> {
    let source = current_app_name();

    // 忽略列表过滤（来源应用名小写匹配）。
    {
        let st = state.lock().unwrap();
        if st.ignored.contains(&source.to_lowercase()) {
            println!("[clipstack] ignored by app filter: {source}");
            return None;
        }
    }

    let raw = match read_clipboard() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[clipstack] read clipboard failed: {e}");
            return None;
        }
    };
    if matches!(raw, RawContent::None) {
        return None;
    }

    let (content_type, preview, size) = classify(&raw);
    let hash = content_hash(&raw);

    // 会话内去重：同一 hash 在窗口内再次出现 → 跳过广播。
    let now = Instant::now();
    {
        let mut st = state.lock().unwrap();
        for (h, t) in st.recent.iter() {
            if *h == hash && now.duration_since(*t) < DEDUP_WINDOW {
                println!("[clipstack] dedup skip hash={hash:016x}");
                return None;
            }
        }
        st.recent.push_back((hash, now));
        if st.recent.len() > DEDUP_CAPACITY {
            st.recent.pop_front();
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    Some(ClipboardItem {
        id: format!("{hash:016x}"),
        content_type,
        preview,
        hash,
        source,
        size,
        timestamp,
    })
}

/// 用 arboard 读取剪贴板：文本优先，其次图片，再次文件。
fn read_clipboard() -> Result<RawContent, arboard::Error> {
    let mut cb = Clipboard::new()?;
    if let Ok(text) = cb.get_text() {
        if !text.trim().is_empty() {
            return Ok(RawContent::Text(text));
        }
    }
    if let Ok(img) = cb.get_image() {
        return Ok(RawContent::Image {
            width: img.width,
            height: img.height,
            bytes: img.bytes.to_vec(),
        });
    }
    if let Ok(files) = cb.get().file_list() {
        if !files.is_empty() {
            return Ok(RawContent::Files(files));
        }
    }
    Ok(RawContent::None)
}

/// 类型识别：文本 →（链接 / 代码）/ 文本；其余按载体识别。
fn classify(raw: &RawContent) -> (ContentType, String, usize) {
    match raw {
        RawContent::Text(s) => {
            let size = s.len();
            let ct = if is_link(s) {
                ContentType::Link
            } else if looks_like_code(s) {
                ContentType::Code
            } else {
                ContentType::Text
            };
            (ct, truncate(s, PREVIEW_MAX), size)
        }
        RawContent::Image { width, height, bytes } => {
            (ContentType::Image, format!("{width}×{height} 图片"), bytes.len())
        }
        RawContent::Files(p) => {
            let joined = p
                .iter()
                .map(|x| x.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            (ContentType::File, truncate(&joined, PREVIEW_MAX), joined.len())
        }
        RawContent::None => (ContentType::Text, String::new(), 0),
    }
}

/// 单行且以常见协议 / www. 开头 → 视为链接。
fn is_link(s: &str) -> bool {
    let t = s.trim();
    if t.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("ftp://")
        || t.starts_with("www.")
}

/// 多行且含典型代码特征 → 视为代码（启发式，后续可由配置增强）。
fn looks_like_code(s: &str) -> bool {
    if !s.contains('\n') {
        return false;
    }
    s.contains(" {")
        || s.contains(";\n")
        || s.contains("=>")
        || s.contains("function ")
        || s.contains("def ")
        || s.contains("import ")
        || s.contains("class ")
        || s.contains("pub fn")
        || s.contains("#include")
        || s.contains("SELECT ")
        || s.contains("</")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let taken: String = s.chars().take(max).collect();
    format!("{taken}…")
}

fn content_hash(raw: &RawContent) -> u64 {
    let mut h = DefaultHasher::new();
    raw.hash(&mut h);
    h.finish()
}

/// 命令：将某来源应用加入忽略列表（手动验证「忽略列表」用；正式管理 UI 在 P6）。
#[tauri::command]
pub fn add_ignored_app(state: State<Arc<Mutex<MonitorState>>>, name: String) {
    state.lock().unwrap().ignored.insert(name.to_lowercase());
}

// ===== 平台相关：剪贴板变更检测与来源应用 =====

#[cfg(target_os = "macos")]
fn current_change_count() -> Option<i64> {
    use objc2_app_kit::NSPasteboard;
    let pb = NSPasteboard::generalPasteboard();
    Some(pb.changeCount() as i64)
}

#[cfg(target_os = "macos")]
fn current_app_name() -> String {
    use objc2_app_kit::NSWorkspace;
    let ws = NSWorkspace::sharedWorkspace();
    match ws.frontmostApplication() {
        Some(app) => match app.localizedName() {
            Some(name) => name.to_string(),
            None => "unknown".to_string(),
        },
        None => "unknown".to_string(),
    }
}

#[cfg(not(target_os = "macos"))]
fn current_change_count() -> Option<i64> {
    // TODO(P4): Windows 接入 AddClipboardFormatListener 实现真正的事件驱动。
    None
}

#[cfg(not(target_os = "macos"))]
fn current_app_name() -> String {
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_text() {
        assert_eq!(
            classify(&RawContent::Text("hello world".into())).0,
            ContentType::Text
        );
    }

    #[test]
    fn classify_link() {
        assert_eq!(
            classify(&RawContent::Text("https://example.com".into())).0,
            ContentType::Link
        );
        assert_eq!(
            classify(&RawContent::Text("www.rust-lang.org".into())).0,
            ContentType::Link
        );
    }

    #[test]
    fn classify_code() {
        let code = "fn main() {\n    println!(\"hi\");\n}\n".to_string();
        assert_eq!(classify(&RawContent::Text(code)).0, ContentType::Code);
    }

    #[test]
    fn classify_image() {
        assert_eq!(
            classify(&RawContent::Image {
                width: 10,
                height: 10,
                bytes: vec![0; 4]
            })
            .0,
            ContentType::Image
        );
    }

    #[test]
    fn classify_files() {
        assert_eq!(
            classify(&RawContent::Files(vec![PathBuf::from("/a.txt")])).0,
            ContentType::File
        );
    }

    #[test]
    fn hash_deterministic() {
        let a = RawContent::Text("same content".into());
        assert_eq!(content_hash(&a), content_hash(&a));
    }

    #[test]
    fn hash_differs_for_different_content() {
        assert_ne!(
            content_hash(&RawContent::Text("a".into())),
            content_hash(&RawContent::Text("b".into()))
        );
    }

    #[test]
    fn truncate_keeps_limit() {
        let s = "x".repeat(500);
        assert!(truncate(&s, 200).chars().count() <= 201);
    }

    #[test]
    fn is_link_rejects_multiline() {
        assert!(!is_link("line1\nline2"));
        assert!(is_link("https://a.b"));
    }
}
