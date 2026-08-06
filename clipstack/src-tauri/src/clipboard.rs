// P1 · 剪贴板捕获引擎（+ P2 持久化落库）
//
// 流程：后台线程检测剪贴板变更 → 读取内容 → 类型识别 → hash 去重
//       → 来源应用过滤（忽略列表）→ 写入 SQLite（`history` 表）→ 广播 `clipboard-changed`。
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
use std::time::{Duration, Instant};

use arboard::Clipboard;
use tauri::{AppHandle, Emitter};

use crate::db::{self, AppDb, DbState, now_ms};
use crate::models::{ContentType, HistoryItem, NewItem};

/// 去重时间窗：同一 hash 在此窗口内再次出现视为重复，不再广播 / 入库。
const DEDUP_WINDOW: Duration = Duration::from_secs(2);
/// 最近 hash 队列容量（仅用于会话内去重，不影响持久化）。
const DEDUP_CAPACITY: usize = 32;
/// 预览文本最大字符数。
const PREVIEW_MAX: usize = 200;
/// changeCount 轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(300);

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

/// 启动后台监控线程（携带 DB 句柄，捕获即落库）。
pub fn start_monitor(app: AppHandle, state: Arc<Mutex<MonitorState>>, db: DbState) {
    std::thread::spawn(move || {
        // 记录启动时刻的 changeCount，避免把「当前已存在的内容」当作一次新捕获。
        let mut last_count = current_change_count().unwrap_or(-1);
        loop {
            std::thread::sleep(POLL_INTERVAL);
            match current_change_count() {
                Some(count) if count != last_count => {
                    last_count = count;
                    if let Some(item) = capture(&db, &state) {
                        let _ = app.emit("clipboard-changed", &item);
                        println!(
                            "[clipstack] captured {:?} id={} hash={} source={} size={}",
                            item.content_type, item.id, item.hash, item.source_app, item.size_bytes
                        );
                    }
                }
                _ => {}
            }
        }
    });
}

/// 读取 + 分类 + 去重 + 来源过滤 + 落库，产出可广播的条目；被忽略 / 去重 / 无内容 / 落库失败返回 None。
fn capture(db: &AppDb, state: &Arc<Mutex<MonitorState>>) -> Option<HistoryItem> {
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

    let (content_type, preview, content_text, content_blob, size) = materialize(&raw);
    let hash = content_hash(&raw);

    // 会话内去重：同一 hash 在窗口内再次出现 → 跳过广播与落库。
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

    let timestamp = now_ms();
    let new = NewItem {
        content_type,
        content_text: content_text.clone(),
        content_blob,
        source_app: source.clone(),
        size_bytes: size as i64,
        hash: format!("{hash:016x}"),
        created_at: timestamp,
    };

    let (id, is_pinned, is_favorite) = {
        let conn = db.conn.lock().unwrap();
        match db::insert_or_bump(&conn, &new) {
            Ok(id) => {
                // 复用内容（bump）或新增后，读回该行的真实置顶 / 收藏状态，
                // 避免把用户设置的 is_pinned/is_favorite 覆盖为 false（否则置顶会被后台重新捕获悄悄取消）。
                let item = db::get_item(&conn, id).ok();
                let is_pinned = item.as_ref().map(|i| i.is_pinned).unwrap_or(false);
                let is_favorite = item.as_ref().map(|i| i.is_favorite).unwrap_or(false);
                (id, is_pinned, is_favorite)
            }
            Err(e) => {
                eprintln!("[clipstack] db insert failed: {e}");
                return None;
            }
        }
    };

    Some(HistoryItem {
        id,
        content_type,
        content_text,
        preview,
        source_app: source,
        size_bytes: new.size_bytes,
        hash: new.hash,
        is_pinned,
        is_favorite,
        created_at: timestamp,
        deleted_at: None,
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
fn classify(raw: &RawContent) -> ContentType {
    match raw {
        RawContent::Text(s) => {
            if is_link(s) {
                ContentType::Link
            } else if looks_like_code(s) {
                ContentType::Code
            } else {
                ContentType::Text
            }
        }
        RawContent::Image { .. } => ContentType::Image,
        RawContent::Files(_) => ContentType::File,
        RawContent::None => ContentType::Text,
    }
}

/// 拆分出「类型 / 预览 / 原文 / 二进制 / 大小」，供落库与广播使用。
fn materialize(raw: &RawContent) -> (ContentType, String, String, Option<Vec<u8>>, usize) {
    match raw {
        RawContent::Text(s) => {
            let ct = classify(raw);
            (ct, truncate(s, PREVIEW_MAX), s.clone(), None, s.len())
        }
        RawContent::Image { width, height, bytes } => {
            let label = format!("{width}×{height} 图片");
            // arboard 返回的是原始 RGBA 像素（非图片文件），需编码为 PNG 才能被前端 <img> 直接渲染。
            // 编码失败时降级为存储原始字节（前端将无法预览，仅开发期旧数据可能命中）。
            let blob = encode_rgba_to_png(*width as u32, *height as u32, bytes)
                .or_else(|| Some(bytes.clone()));
            let size = blob.as_ref().map(|b| b.len()).unwrap_or(0);
            (
                ContentType::Image,
                label.clone(),
                label,
                blob,
                size,
            )
        }
        RawContent::Files(p) => {
            let joined = p
                .iter()
                .map(|x| x.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            (
                ContentType::File,
                truncate(&joined, PREVIEW_MAX),
                joined.clone(),
                None,
                joined.len(),
            )
        }
        RawContent::None => (ContentType::Text, String::new(), String::new(), None, 0),
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

/// 将 arboard 返回的 RGBA8 原始像素（行主序、自上而下）编码为 PNG 字节，
/// 供前端 `<img>` 直接渲染。编码失败返回 None（调用方降级为存储原始字节）。
fn encode_rgba_to_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(rgba.len() / 2 + 1024);
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

fn content_hash(raw: &RawContent) -> u64 {
    let mut h = DefaultHasher::new();
    raw.hash(&mut h);
    h.finish()
}

/// 将来源应用加入忽略集合（命令层在持久化同时调用，即时生效）。
pub fn ignore_app(state: &Arc<Mutex<MonitorState>>, name: &str) {
    state.lock().unwrap().ignored.insert(name.to_lowercase());
}

/// 主动复制占位：把即将写回剪贴板的文本 hash 记入监控去重队列，
/// 使其在 `DEDUP_WINDOW` 内被监控线程判定为重复而跳过捕获——
/// 从而「选中条目重新复制」不会再次入列、也不会改写原复制时间。
/// 应在 `set_clipboard_text` 之前调用。
pub fn note_self_copy(state: &Arc<Mutex<MonitorState>>, text: &str) {
    let hash = content_hash(&RawContent::Text(text.to_string()));
    let mut st = state.lock().unwrap();
    st.recent.push_back((hash, Instant::now()));
    if st.recent.len() > DEDUP_CAPACITY {
        st.recent.pop_front();
    }
}

/// 将文本写回系统剪贴板（供「复制」按钮、托盘点击使用）。
///
/// 注意：arboard 的 `set_file_list` 为平台私有、`Set` builder 不暴露文件方法，
/// 故图片 / 文件复制暂不在此支持（前端已对这两类禁用复制按钮）。
pub fn set_clipboard_text(text: &str) -> Result<(), String> {
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text.to_string()).map_err(|e| e.to_string())
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
        assert_eq!(classify(&RawContent::Text("hello world".into())), ContentType::Text);
    }

    #[test]
    fn classify_link() {
        assert_eq!(
            classify(&RawContent::Text("https://example.com".into())),
            ContentType::Link
        );
        assert_eq!(
            classify(&RawContent::Text("www.rust-lang.org".into())),
            ContentType::Link
        );
    }

    #[test]
    fn classify_code() {
        let code = "fn main() {\n    println!(\"hi\");\n}\n".to_string();
        assert_eq!(classify(&RawContent::Text(code)), ContentType::Code);
    }

    #[test]
    fn classify_image() {
        assert_eq!(
            classify(&RawContent::Image {
                width: 10,
                height: 10,
                bytes: vec![0; 4]
            }),
            ContentType::Image
        );
    }

    #[test]
    fn classify_files() {
        assert_eq!(
            classify(&RawContent::Files(vec![PathBuf::from("/a.txt")])),
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

    #[test]
    fn png_roundtrip_preserves_pixels() {
        // 2×2 RGBA：红、绿、蓝、黄
        let rgba: Vec<u8> = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let png = encode_rgba_to_png(2, 2, &rgba).expect("encode");
        // 验证 PNG 文件头签名
        assert_eq!(
            &png[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );
        // 解码回 RGBA 与原数据一致
        let decoder = png::Decoder::new(std::io::Cursor::new(&png));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        assert_eq!(reader.info().width, 2);
        assert_eq!(reader.info().height, 2);
        assert_eq!(buf, rgba);
    }

    #[test]
    fn note_self_copy_records_hash_to_dedup_queue() {
        use std::sync::Arc;
        use std::sync::Mutex;
        let state = Arc::new(Mutex::new(MonitorState::default()));
        let text = "主动复制的内容";
        note_self_copy(&state, text);
        let expected = content_hash(&RawContent::Text(text.to_string()));
        let st = state.lock().unwrap();
        assert!(st.recent.iter().any(|(h, _)| *h == expected));
    }
}
