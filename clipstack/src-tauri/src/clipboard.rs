// P1 · 剪贴板捕获引擎（+ P2 持久化落库）
//
// 流程：后台线程检测剪贴板变更 → 读取内容 → 类型识别 → hash 去重
//       → 来源应用过滤（忽略列表）→ 写入 SQLite（`history` 表）→ 广播 `clipboard-changed`。
//
// 关于「事件驱动」：
//   - Windows 采用系统级 `GetClipboardSequenceNumber()`：任意进程写入剪贴板时该序号自增，
//     无需打开/读取剪贴板内容即可判断变更（与 macOS 的 `NSPasteboard.changeCount` 语义一致）。
//     序列号为 0 的极少数情况下回退为「读取内容计算 hash」比对，保证不漏检。
//   - macOS 没有公开的剪贴板变更通知，业界（Maccy / Paste 等）均通过轮询
//     `NSPasteboard.changeCount` 实现。此处采用 300ms 间隔的 changeCount 比对——
//     这是一次整数比较，绝非忙等循环，符合「不阻塞、不空转」的设计意图。

use std::collections::{hash_map::DefaultHasher, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use arboard::Clipboard;
use tauri::{AppHandle, Emitter, Manager};
use zeroize::Zeroize;

use crate::crypto::Key;
use crate::db::{self, AppDb, DbState, now_ms, SENSITIVE_MASK};
use crate::lan::LanManager;
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

/// P2 · 内存清零：剪贴板明文在捕获周期结束后从内存擦除（Text 字符串 / Image 字节）。
/// `RawContent` 为独占所有权、无 Clone，捕获流程中仅移动一次并在 `capture` 末尾 drop，
/// 故此处清零不会影响分类 / 去重 / 加密前的读取（那些都在 drop 之前发生）。
impl Drop for RawContent {
    fn drop(&mut self) {
        match self {
            RawContent::Text(s) => s.zeroize(),
            RawContent::Image { bytes, .. } => bytes.zeroize(),
            // 文件路径本身非敏感正文，无需擦除。
            RawContent::Files(_) | RawContent::None => {}
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
                        // L3 · 局域网共享：本地捕获成功后（若已开启 share_out）广播给 mesh 对端。
                        // 仅在「真实 OS 剪贴板变更」分支触发，对端写入不重复触发，避免回环放大。
                        if let Some(lan) = app.try_state::<LanManager>() {
                            let mgr = lan.inner().clone();
                            let ct = item.content_type.as_str().to_string();
                            let txt = item.content_text.clone();
                            let src = item.source_app.clone();
                            tauri::async_runtime::spawn(async move {
                                mgr.broadcast_local(&ct, &txt, &src).await;
                            });
                        }
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
    // 内部数据库加密密钥在启动阶段已载入内存（db.key 常驻），用于落库加密；
    // 主密码仅作为「应用锁」凭据，不影响此处加密。因此无论是否锁定、是否启用主密码，
    // 捕获到的内容都以内部密钥加密存储，复制永不丢失。
    let source = current_app_name();

    // 忽略列表过滤（来源应用名小写匹配）。
    {
        let st = state.lock().unwrap();
        if st.ignored.iter().any(|ig| ig.eq_ignore_ascii_case(&source)) {
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

    let (content_type, _preview, content_text, content_blob, size) = materialize(&raw);

    // P1c · 敏感内容识别：命中规则（文本 / 链接 / 代码类）即标记 is_sensitive（纯识别结果，
    // 与「掩码敏感内容」开关无关）。实际预览是否遮挡在「读取时」按当前开关实时计算
    // （见 db.rs `mask_sensitive_read`），使开关切换对所有（含历史）条目立即生效。
    // 此处（刚捕获、尚未加密存储）也按当前开关计算 preview，保证新条目即时生效。
    // 原文仍加密存储，复制不受影响。
    let is_sensitive = matches!(
        content_type,
        ContentType::Text | ContentType::Link | ContentType::Code
    ) && is_sensitive(&content_text);
    let preview = {
        let conn = db.conn.lock().unwrap();
        let mask_on = db::get_string_setting(&conn, "mask_sensitive", "0") != "0";
        if mask_on && is_sensitive {
            SENSITIVE_MASK.to_string()
        } else {
            content_text.clone()
        }
    };

    let hash = content_hash(&raw);

    // P1a：按「保存历史记录类型」设置过滤；禁用类型不捕获。
    // 决策：不提供「一键清理已禁用类型历史」按钮，仅控制后续是否继续捕获。
    {
        let conn = db.conn.lock().unwrap();
        if !db::save_type_enabled(&conn, content_type) {
            println!("[clipstack] capture skipped by save-type filter: {content_type:?}");
            return None;
        }
    }

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
        is_sensitive,
    };

    let (id, is_pinned, is_favorite) = {
        let conn = db.conn.lock().unwrap();
        // 内部数据库密钥常驻内存（db.key），直接用于加密存储。
        let key_guard = db.key.lock().unwrap();
        let eff: Option<&Key> = key_guard.as_ref();
        match db::insert_or_bump(&conn, eff, &new) {
            Ok(id) => {
                // 复用内容（bump）或新增后，读回该行的真实置顶 / 收藏状态，
                // 避免把用户设置的 is_pinned/is_favorite 覆盖为 false（否则置顶会被后台重新捕获悄悄取消）。
                let item = db::get_item(&conn, key_guard.as_ref(), id).ok();
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
        is_sensitive,
        created_at: timestamp,
        origin_device: String::new(),
        is_remote: false,
        deleted_at: None,
    })
}

/// 用 arboard 读取剪贴板：文件优先，其次图片，再次文本。
///
/// 关键顺序：必须先判断 `file_list`，再判断 `get_text`。
/// 在 macOS 上，Finder 复制文件时粘贴板会同时写入 `NSStringPboardType`，
/// 但其内容仅含**文件名**（不含路径）；若先取文本，会把「文件」误判为「文本」——
/// 导致文件以 `content_type=text` + `content_text=文件名` 入库，既无法一键粘贴为文件本身，
/// 又会在类型筛选「文件」与托盘菜单中表现为文本。优先识别文件列表可彻底规避该误判。
///
/// Windows 图片捕获补全：arboard 3.x 的 `get_image` 只读取 `PNG` 与 `CF_DIBV5` 两种格式，
/// 对部分截图软件（如 Windows 截图工具、Snipaste 等）写入的 `CF_BITMAP` / `CF_DIB` 无法识别，
/// 导致截图被判定为「无内容」而漏捕获。故 Windows 端先以 Win32 直接读取全部图片格式，命中即返回。
fn read_clipboard() -> Result<RawContent, arboard::Error> {
    #[cfg(target_os = "windows")]
    {
        if let Some(img) = read_image_win32() {
            return Ok(img);
        }
    }
    let mut cb = Clipboard::new()?;
    if let Ok(files) = cb.get().file_list() {
        if !files.is_empty() {
            return Ok(RawContent::Files(files));
        }
    }
    if let Ok(img) = cb.get_image() {
        return Ok(RawContent::Image {
            width: img.width,
            height: img.height,
            bytes: img.bytes.to_vec(),
        });
    }
    if let Ok(text) = cb.get_text() {
        if !text.trim().is_empty() {
            return Ok(RawContent::Text(text));
        }
    }
    Ok(RawContent::None)
}

/// Windows 专用：用 Win32（clipboard-win）读取 arboard 未覆盖的剪贴板图片格式。
/// 优先 `CF_BITMAP`（截图为 GDI 位图，最常被漏掉），其次 `CF_DIB`。
/// 两者都先取原始字节、再交给 `image` crate 解码为 RGBA（与 `arboard` 的 RGBA 像素格式一致）。
/// 打开剪贴板失败时返回 None，交由下方 arboard 路径兜底（其后再回退到文本）。
#[cfg(target_os = "windows")]
fn read_image_win32() -> Option<RawContent> {
    use clipboard_win::formats;
    use clipboard_win::raw;

    if raw::open().is_err() {
        return None;
    }
    // 无论以何种路径返回，都要关闭剪贴板，否则会占用导致 arboard 后续 `Clipboard::new()` 打开失败。
    let _guard = ClipboardCloseGuard;

    // 1) CF_BITMAP：GDI 位图。clipboard-win 的 get_bitmap 直接产出标准 BMP 文件字节。
    if raw::is_format_avail(formats::CF_BITMAP) {
        let mut bmp = Vec::new();
        if raw::get_bitmap(&mut bmp).is_ok() {
            match bmp_to_rgba(&bmp) {
                Some((w, h, bytes)) => {
                    return Some(RawContent::Image {
                        width: w,
                        height: h,
                        bytes,
                    })
                }
                None => eprintln!("[clipstack] CF_BITMAP 存在但 BMP 解码失败（len={}）", bmp.len()),
            }
        } else {
            eprintln!("[clipstack] CF_BITMAP 存在但 get_bitmap 读取失败");
        }
    }

    // 2) CF_DIB：设备无关位图（无 BITMAPFILEHEADER）。包成 BMP 文件头后交给 image 解码。
    if raw::is_format_avail(formats::CF_DIB) {
        let mut data = Vec::new();
        if raw::get_vec(formats::CF_DIB, &mut data).is_ok() {
            match dib_to_rgba(&data) {
                Some((w, h, bytes)) => {
                    return Some(RawContent::Image {
                        width: w,
                        height: h,
                        bytes,
                    })
                }
                None => eprintln!("[clipstack] CF_DIB 存在但解码失败（len={}）", data.len()),
            }
        } else {
            eprintln!("[clipstack] CF_DIB 存在但 get_vec 读取失败");
        }
    }

    None
}

/// 读取剪贴板后确保调用 `CloseClipboard`，避免长期占用剪贴板句柄。
#[cfg(target_os = "windows")]
struct ClipboardCloseGuard;

#[cfg(target_os = "windows")]
impl Drop for ClipboardCloseGuard {
    fn drop(&mut self) {
        let _ = clipboard_win::raw::close();
    }
}

/// 把标准 BMP 文件字节解码为 RGBA 像素（与 arboard 的 ImageData 像素格式一致：行主序、自上而下）。
#[cfg(target_os = "windows")]
fn bmp_to_rgba(bmp: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let img = image::load_from_memory_with_format(bmp, image::ImageFormat::Bmp).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    Some((w, h, rgba.into_raw()))
}

/// 把 `CF_DIB` 原始字节（BITMAPINFOHEADER + 调色板/掩码 + 像素）包成标准 BMP 文件后再解码为 RGBA。
/// 处理 `BI_BITFIELDS`（16/32 位带位掩码，无调色板、仅有 3 个 DWORD 掩码）与带调色板的低位深情况。
#[cfg(target_os = "windows")]
fn dib_to_rgba(dib: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    if dib.len() < 40 {
        return None;
    }
    let bi_size = u32::from_le_bytes(dib[0..4].try_into().ok()?) as usize;
    if bi_size < 40 || bi_size > dib.len() {
        return None;
    }
    let bi_bit_count = u16::from_le_bytes(dib[14..16].try_into().ok()?) as u32;
    let bi_compression = u32::from_le_bytes(dib[16..20].try_into().ok()?);
    let bi_clr_used = u32::from_le_bytes(dib[32..36].try_into().ok()?) as usize;

    // 计算信息头之后、像素数据之前的额外字节数（调色板或 BI_BITFIELDS 的位掩码）。
    let extra = if bi_compression == 3 {
        // BI_BITFIELDS：3 个 DWORD 掩码（12 字节），无调色板。
        12
    } else if bi_clr_used > 0 {
        bi_clr_used * 4
    } else if bi_bit_count < 24 {
        1 << bi_bit_count
    } else {
        0
    };

    let data_offset = 14 + bi_size + extra;
    let mut bmp = Vec::with_capacity(data_offset + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((data_offset + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0u8; 4]); // 保留字段
    bmp.extend_from_slice(&(data_offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    bmp_to_rgba(&bmp)
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
            // content_text 仍为人类可读的路径拼接（用于列表预览与详情展示）；
            // content_blob 存机器可读的 JSON 路径数组（用于一键复制，避免 ", " 分隔在含逗号 / 空格的文件名上歧义）。
            // 文件内容不入库，库里只保存路径。
            let joined = p
                .iter()
                .map(|x| x.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            // 文件大小取各文件实际字节数之和（文件可能已不存在，best-effort 跳过）。
            let size = p
                .iter()
                .filter_map(|x| std::fs::metadata(x).ok())
                .map(|m| m.len())
                .sum::<u64>() as usize;
            let blob = serde_json::to_vec(p).ok();
            (ContentType::File, truncate(&joined, PREVIEW_MAX), joined.clone(), blob, size)
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

/// 字符串的 Shannon 熵（比特/字符），用于密码启发式判定。
fn shannon_entropy(s: &str) -> f64 {
    use std::collections::HashMap;
    if s.is_empty() {
        return 0.0;
    }
    let mut counts: HashMap<u8, usize> = HashMap::new();
    for b in s.bytes() {
        *counts.entry(b).or_insert(0) += 1;
    }
    let n = s.len() as f64;
    -counts
        .values()
        .map(|&c| {
            let p = c as f64 / n;
            p * p.log2()
        })
        .sum::<f64>()
}

/// Luhn 校验（卡号合法性）。传入已去除空格 / 连字符的纯数字串。
fn luhn_valid(digits: &str) -> bool {
    let ds: Vec<u8> = digits
        .as_bytes()
        .iter()
        .map(|&b| b - b'0')
        .collect();
    if ds.len() < 13 || ds.len() > 19 {
        return false;
    }
    let mut sum = 0u32;
    let mut alt = false;
    for &d in ds.iter().rev() {
        let mut n = d as u32;
        if alt {
            n *= 2;
            if n > 9 {
                n -= 9;
            }
        }
        sum += n;
        alt = !alt;
    }
    sum % 10 == 0
}

/// 启发式识别常见密钥 / Token（不依赖正则依赖，手工匹配主要形态）：
/// - `sk-` + ≥20 位字母数字（OpenAI 等）
/// - `ghp_` + 36 位字母数字（GitHub PAT）
/// - `AKIA` + 16 位大写字母数字（AWS Access Key）
/// - `xox[baprs]-` + 字母数字 / 连字符（Slack Token）
/// - `eyJ…` 开头、含 ≥2 个 `.`、整体 base64url（JWT）
fn looks_like_token(s: &str) -> bool {
    if s.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let alnum = |r: &str| !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric());
    let alnum_dash = |r: &str| {
        !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };

    if let Some(rest) = s.strip_prefix("sk-") {
        if rest.len() >= 20 && alnum(rest) {
            return true;
        }
    }
    if let Some(rest) = s.strip_prefix("ghp_") {
        if rest.len() == 36 && alnum(rest) {
            return true;
        }
    }
    if let Some(rest) = s.strip_prefix("AKIA") {
        if rest.len() == 16 && alnum(rest) {
            return true;
        }
    }
    if let Some(rest) = s.strip_prefix("xox") {
        if let Some(rest2) = rest.strip_prefix(['b', 'a', 'p', 'r', 's']) {
            if let Some(rest3) = rest2.strip_prefix('-') {
                if alnum_dash(rest3) {
                    return true;
                }
            }
        }
    }
    if s.starts_with("eyJ") {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() >= 2
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        {
            return true;
        }
    }
    false
}

/// 密码启发式：长度 ≥ 12、无空白、含 ≥3 种字符类（小写/大写/数字/符号），且经验熵 > 3.0。
/// 要求「多字符类」可排除仅含单一字符类的长单词（如长英文单词），避免误判正文；
/// 经验熵兜底确保即便混合了字符类，也需具备足够随机性才判定为密码。
fn is_strong_password(t: &str) -> bool {
    if t.len() < 12 || t.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let mut classes = 0u8;
    if t.chars().any(|c| c.is_ascii_lowercase()) {
        classes |= 1;
    }
    if t.chars().any(|c| c.is_ascii_uppercase()) {
        classes |= 2;
    }
    if t.chars().any(|c| c.is_ascii_digit()) {
        classes |= 4;
    }
    if t.chars().any(|c| !c.is_ascii_alphanumeric()) {
        classes |= 8;
    }
    classes.count_ones() >= 3 && shannon_entropy(t) > 3.0
}

/// 敏感内容识别：命中以下任一规则即视为敏感（启用掩码时预览被遮挡，原文仍加密存储）。
/// - 常见 Token / 密钥形态；
/// - 整段为纯数字（允许空格/连字符分隔）且通过 Luhn 校验（银行卡号，长度 13–19）；
/// - 长度 ≥ 12、无空白、含 ≥3 种字符类且经验熵 > 3.0（高随机性密码启发式）。
pub fn is_sensitive(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if looks_like_token(t) {
        return true;
    }
    // 卡号：整段只能是数字与分隔符（空格/连字符），去分隔符后做 Luhn 校验。
    if t.chars()
        .all(|c| c.is_ascii_digit() || c == ' ' || c == '-')
    {
        let cleaned: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
        if luhn_valid(&cleaned) {
            return true;
        }
    }
    // 密码启发式：长度 / 多字符类 / 熵三者兼具备。
    if is_strong_password(t) {
        return true;
    }
    false
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
/// 保留系统原始显示名（含中文/原始大小写），匹配时大小写不敏感。
pub fn ignore_app(state: &Arc<Mutex<MonitorState>>, name: &str) {
    state.lock().unwrap().ignored.insert(name.to_string());
}

/// 从忽略集合移除应用（命令层在清理持久化同时调用，即时生效）。
/// 大小写不敏感移除，兼容不同大小写形式的同名项。
pub fn unignore_app(state: &Arc<Mutex<MonitorState>>, name: &str) {
    state.lock().unwrap().ignored.retain(|ig| !ig.eq_ignore_ascii_case(name));
}

/// 枚举系统中已安装应用的显示名（系统本地化名，如中文系统的「访达」「微信」），
/// 供「忽略应用」设置从系统列表选择，并与监控过滤的 `current_app_name()` 同源。
///
/// 性能：逐应用调用 `mdls` 取本地化名开销较大，故使用**会话级缓存**——首次计算后
/// 复用，避免每次打开设置界面都重新扫描导致卡顿；并在启动阶段后台预热（见 lib.rs setup），
/// 使首次打开设置即可命中缓存。扫描范围限定为用户应用目录与系统「实用工具」，
/// 不再遍历整个 `/System/Applications`（数百个系统应用，多数无需忽略且逐 mdls 极慢）。
pub fn list_installed_apps() -> Vec<String> {
    // 会话级缓存：空结果不入缓存（避免 mdls 暂不可用时被永久缓存为空）。
    static CACHE: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(g) = cache.lock() {
        if !g.is_empty() {
            return g.clone();
        }
    }

    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    #[cfg(target_os = "macos")]
    {
        let mut dirs = vec![std::path::PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(std::path::PathBuf::from(home).join("Applications"));
        }
        // 仅额外扫描系统「实用工具」（如终端等用户可能忽略的应用），避免遍历整个 /System/Applications。
        dirs.push(std::path::PathBuf::from("/System/Applications/Utilities"));
        for dir in dirs {
            collect_app_names(&dir, &mut names, 0);
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Windows 枚举「拥有可见主窗口」的运行中进程（即用户实际会复制的应用），
        // 与监控线程 current_app_name() 返回的 exe 名同源，确保忽略匹配一致。
        collect_running_app_names(&mut names);
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        // 非 macOS/Windows 暂不支持枚举，交由前端回退到手动输入。
        let _ = &mut names;
    }
    let result: Vec<String> = names.into_iter().collect();
    if !result.is_empty() {
        if let Ok(mut g) = cache.lock() {
            *g = result.clone();
        }
    }
    result
}

#[cfg(target_os = "macos")]
fn collect_app_names(dir: &std::path::Path, out: &mut std::collections::BTreeSet<String>, depth: usize) {
    // 最多下钻一层子文件夹（如 /Applications/Utilities），避免误入 .app 内部或无限递归拖慢扫描。
    if depth > 1 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let fname = match path.file_name().and_then(|f| f.to_str()) {
            Some(f) => f,
            None => continue,
        };
        if fname.ends_with(".app") {
            if let Some(name) = app_display_name(&path) {
                out.insert(name);
            }
        } else if path.is_dir() {
            // 递归一层：部分应用放在子文件夹（如 /Applications/Utilities）。
            collect_app_names(&path, out, depth + 1);
        }
    }
}

#[cfg(target_os = "windows")]
fn collect_running_app_names(out: &mut std::collections::BTreeSet<String>) {
    use std::collections::HashSet;
    use std::ffi::c_void;

    // 1) 收集「拥有可见、无主窗口」的进程 PID —— 这些才是用户会想要忽略的「应用」主窗口。
    type WNDENUMPROC = unsafe extern "system" fn(*mut c_void, isize) -> i32;
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(lpEnumFunc: WNDENUMPROC, lParam: isize) -> i32;
        fn IsWindowVisible(hWnd: *mut c_void) -> i32;
        fn GetWindow(hWnd: *mut c_void, uCmd: u32) -> *mut c_void;
        fn GetWindowThreadProcessId(hWnd: *mut c_void, lpdwProcessId: *mut u32) -> u32;
    }
    const GW_OWNER: u32 = 4;

    let mut visible_pids: HashSet<u32> = HashSet::new();
    unsafe extern "system" fn enum_cb(hwnd: *mut c_void, lparam: isize) -> i32 {
        let set = &mut *(lparam as *mut HashSet<u32>);
        if IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != 0 {
                set.insert(pid);
            }
        }
        1
    }
    unsafe {
        EnumWindows(enum_cb, &mut visible_pids as *mut _ as isize);
    }

    // 2) 枚举全部进程，取 exe 名；PID 落在可见窗口集合中才纳入（去重、按名排序由 BTreeSet 完成）。
    #[repr(C)]
    struct PROCESSENTRY32W {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u16; 260],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> *mut c_void;
        fn CloseHandle(hObject: *mut c_void) -> i32;
        fn Process32FirstW(hSnapshot: *mut c_void, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(hSnapshot: *mut c_void, lppe: *mut PROCESSENTRY32W) -> i32;
    }
    const TH32CS_SNAPPROCESS: u32 = 0x00000002;

    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if (snap as isize) == -1 {
        return;
    }
    let mut pe: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    if unsafe { Process32FirstW(snap, &mut pe) } != 0 {
        loop {
            let nul = pe
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(0);
            let name = String::from_utf16_lossy(&pe.szExeFile[..nul]);
            let trimmed = name.trim_end_matches(".exe");
            if !trimmed.is_empty() && visible_pids.contains(&pe.th32ProcessID) {
                out.insert(trimmed.to_string());
            }
            if unsafe { Process32NextW(snap, &mut pe) } == 0 {
                break;
            }
        }
    }
    unsafe { CloseHandle(snap) };
}

#[cfg(target_os = "macos")]
fn app_display_name(bundle: &std::path::Path) -> Option<String> {
    // 优先取 Finder 本地化显示名（中文系统即中文名，如「访达」「微信」），
    // 与监控线程 current_app_name() 使用的 localizedName() 同源，确保忽略匹配一致。
    let path = bundle.to_string_lossy();
    if let Ok(out) = std::process::Command::new("mdls")
        .args(["-name", "kMDItemDisplayName", "-raw", &path])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() && s != "(null)" {
            // mdls 可能带回 .app 后缀，去掉以与 localizedName() 对齐（否则忽略匹配失效）。
            let stripped = s.trim_end_matches(".app");
            if !stripped.is_empty() {
                return Some(stripped.to_string());
            }
        }
    }
    // 回退：解析 Info.plist 的显示名键。
    let plist = bundle.join("Contents").join("Info.plist");
    if let Ok(text) = std::fs::read_to_string(&plist) {
        if let Some(n) = plist_string(&text, "CFBundleDisplayName") {
            return Some(n);
        }
        if let Some(n) = plist_string(&text, "CFBundleName") {
            return Some(n);
        }
    }
    // 回退：目录名去掉 .app 后缀。
    bundle
        .file_name()
        .and_then(|f| f.to_str())
        .map(|f| f.trim_end_matches(".app").to_string())
}

/// 从 XML plist 文本提取某个 `<key>` 对应的 `<string>` 值（轻量解析，满足显示名读取）。
#[cfg(target_os = "macos")]
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let idx = plist.find(&needle)?;
    let rest = &plist[idx + needle.len()..];
    let open = rest.find("<string>")?;
    let after = &rest[open + 8..];
    let close = after.find("</string>")?;
    let val = after[..close].trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
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
/// 图片复制见 `set_clipboard_image`，文件复制见 `set_clipboard_file_list`。
pub fn set_clipboard_text(text: &str) -> Result<(), String> {
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text.to_string()).map_err(|e| e.to_string())
}

/// 将一组文件路径写回系统剪贴板（供「复制」按钮、托盘点击使用），
/// 使目标可粘贴为文件本身（如访达 / 资源管理器中粘贴文件）。
pub fn set_clipboard_file_list(paths: &[PathBuf]) -> Result<(), String> {
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    cb.set().file_list(paths).map_err(|e| e.to_string())
}

/// 将 PNG 字节解码为 RGBA 像素（width, height, bytes），供写回剪贴板与去重 hash 使用。
/// 解码失败返回 None（调用方按错误处理）。
fn decode_png(png_bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((
        reader.info().width as usize,
        reader.info().height as usize,
        buf,
    ))
}

/// 将 PNG 图片写回系统剪贴板（供「复制」按钮、托盘点击使用）。
/// arboard 的 `set_image` 接收 RGBA 像素，故先解码 PNG → RGBA 再写入。
pub fn set_clipboard_image(png_bytes: &[u8]) -> Result<(), String> {
    let (width, height, rgba) =
        decode_png(png_bytes).ok_or_else(|| "无法解码图片数据".to_string())?;
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    let image_data = arboard::ImageData {
        width,
        height,
        bytes: rgba.into(),
    };
    cb.set_image(image_data).map_err(|e| e.to_string())
}

/// 主动复制图片占位：把即将写回剪贴板的图片 hash 记入监控去重队列，
/// 使其在 `DEDUP_WINDOW` 内被监控线程判定为重复而跳过捕获——
/// 从而「选中图片重新复制」不会再次入列、也不会改写原复制时间。
/// 应在 `set_clipboard_image` 之前调用。解码失败时静默（仅影响去重，不影响复制本身）。
pub fn note_self_copy_image(state: &Arc<Mutex<MonitorState>>, png_bytes: &[u8]) {
    if let Some((width, height, rgba)) = decode_png(png_bytes) {
        let hash = content_hash(&RawContent::Image {
            width,
            height,
            bytes: rgba,
        });
        let mut st = state.lock().unwrap();
        st.recent.push_back((hash, Instant::now()));
        if st.recent.len() > DEDUP_CAPACITY {
            st.recent.pop_front();
        }
    }
}

/// 主动复制文件占位：用与监控线程一致的 Files hash 记入监控去重队列，
/// 使其在 `DEDUP_WINDOW` 内被判定为重复而跳过捕获——
/// 从而「选中文件重新复制」不会再次入列、也不会改写原复制时间。
/// 应在 `set_clipboard_file_list` 之前调用。
pub fn note_self_copy_files(state: &Arc<Mutex<MonitorState>>, paths: &[PathBuf]) {
    // 与监控线程读回的路径保持一致：arboard 写回剪贴板时会先 `canonicalize`，
    // 监控线程 `file_list` 读回的也是规范化路径，故此处同样规范化以保证去重命中
    // （避免「重新复制文件」被监控线程再次捕获而重复入列）。文件不存在时退化为原路径。
    let canon: Vec<PathBuf> = paths
        .iter()
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect();
    let effective = if canon.is_empty() {
        paths.to_vec()
    } else {
        canon
    };
    let hash = content_hash(&RawContent::Files(effective));
    let mut st = state.lock().unwrap();
    st.recent.push_back((hash, Instant::now()));
    if st.recent.len() > DEDUP_CAPACITY {
        st.recent.pop_front();
    }
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

#[cfg(target_os = "windows")]
fn current_change_count() -> Option<i64> {
    // Windows 没有 NSPasteboard.changeCount，但系统提供等价的「剪贴板序列号」：
    // 任意进程写入剪贴板时该序号都会自增。无需打开/读取剪贴板内容即可判断「是否发生变化」，
    // 与 macOS 的 changeCount 语义一致，开销极低（整数比较，不读内容）。
    #[link(name = "user32")]
    extern "system" {
        fn GetClipboardSequenceNumber() -> u32;
    }
    let seq = unsafe { GetClipboardSequenceNumber() };
    if seq != 0 {
        return Some(seq as i64);
    }
    // 序列号为 0（本进程尚未触发剪贴板序列号维护，极少见）时，回退为读取内容计算 hash，
    // 保证不会漏检早期复制。内容为空时返回 None（不视为变更）。
    match read_clipboard() {
        Ok(raw) if !matches!(raw, RawContent::None) => Some(content_hash(&raw) as i64),
        _ => None,
    }
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn current_change_count() -> Option<i64> {
    // 其它平台（如 Linux）暂未实现事件驱动的变更检测。
    None
}

#[cfg(target_os = "windows")]
fn current_app_name() -> String {
    use std::ffi::c_void;

    // 用「剪贴板所有者窗口」反查来源进程：GetClipboardOwner 返回最后一次向剪贴板写入数据的
    // 窗口句柄（即来源应用的窗口），不打开剪贴板、不影响后续 arboard 读取，是最准确的来源识别。
    #[link(name = "user32")]
    extern "system" {
        fn GetClipboardOwner() -> *mut c_void;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, lpdwProcessId: *mut u32) -> u32;
    }

    let owner = unsafe { GetClipboardOwner() };
    if owner.is_null() {
        return "unknown".to_string();
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(owner, &mut pid) };
    match resolve_exe_name(pid) {
        Some(name) => name,
        None => "unknown".to_string(),
    }
}

/// 由进程 PID 解析其 exe 文件名（去 .exe 后缀），用于来源应用识别；失败返回 None。
#[cfg(target_os = "windows")]
fn resolve_exe_name(pid: u32) -> Option<String> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStringExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> *mut c_void;
        fn CloseHandle(hObject: *mut c_void) -> i32;
        fn QueryFullProcessImageNameW(
            hProcess: *mut c_void,
            dwFlags: u32,
            lpExeName: *mut u16,
            lpdwSize: *mut u32,
        ) -> i32;
    }

    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;

    if pid == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let name = {
        let mut buf = [0u16; 1024];
        let mut size: u32 = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size) };
        if ok != 0 && size > 0 {
            let path = std::ffi::OsString::from_wide(&buf[..size as usize])
                .to_string_lossy()
                .to_string();
            std::path::Path::new(&path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone())
        } else {
            String::new()
        }
    };
    unsafe { CloseHandle(handle) };
    if name.is_empty() {
        None
    } else {
        // 去掉 .exe 后缀，与「忽略应用」的小写匹配规则对齐（如 msedge、Snipaste、notepad）。
        Some(name.trim_end_matches(".exe").to_string())
    }
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn current_app_name() -> String {
    // Linux 等暂未接入来源应用识别。
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
    fn is_sensitive_detects_tokens() {
        // OpenAI 类 sk- token（≥20 位字母数字）
        assert!(is_sensitive(&format!("sk-{}", "a".repeat(24))));
        // GitHub PAT
        assert!(is_sensitive(&format!("ghp_{}", "a".repeat(36))));
        // AWS Access Key
        assert!(is_sensitive(&format!("AKIA{}", "A".repeat(16))));
        // Slack token
        assert!(is_sensitive("xoxb-1234567890-abcdefghij"));
        // JWT（eyJ…eyJ….base64url）
        assert!(is_sensitive("eyJhbGciOiJIUzI.eyJzdWIiOiIxMj.abc123_-def"));
    }

    #[test]
    fn is_sensitive_rejects_plain_text() {
        assert!(!is_sensitive("hello world"));
        assert!(!is_sensitive("https://example.com"));
        assert!(!is_sensitive("fn main() {\n    println!(\"hi\");\n}"));
        assert!(!is_sensitive(""));
    }

    #[test]
    fn is_sensitive_detects_card_numbers() {
        // 4111 1111 1111 1111 是标准 Luhn 合法测试卡号
        assert!(is_sensitive("4111 1111 1111 1111"));
        assert!(is_sensitive("4111-1111-1111-1111"));
        // 明显非法的卡号
        assert!(!is_sensitive("0000 0000 0000 0001"));
        assert!(!is_sensitive("1234 5678 9012 3456"));
    }

    #[test]
    fn is_sensitive_detects_high_entropy_password() {
        // 长度 ≥ 12、含 ≥3 种字符类、高熵 → 命中
        assert!(is_sensitive("aB3$xZ9qLm7&Kr2P"));
        // 长度不足 12 → 不命中
        assert!(!is_sensitive("password123"));
        // 含空白（长句）→ 不命中
        assert!(!is_sensitive("hello world foo bar baz"));
        // 单一字符类的长单词（仅小写字母）→ 不命中（避免误判正文）
        assert!(!is_sensitive("antidisestablishmentarianism"));
    }

    #[test]
    fn is_strong_password_requires_mixed_classes() {
        // 仅数字（单类）→ 不命中
        assert!(!is_strong_password("123456789012"));
        // 小写+数字（两类）→ 不命中（需 ≥3 类）
        assert!(!is_strong_password("abcdefghij12"));
        // 小+大+数字（三类）→ 命中
        assert!(is_strong_password("aBcDeF123456"));
    }

    #[test]
    fn shannon_entropy_monotonic_with_letters() {
        // 重复字符熵低，随机串熵高
        assert!(shannon_entropy("aaaaaa") < shannon_entropy("aB3$xZ"));
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

/// Windows 专用：验证「只写 CF_BITMAP 的截图软件」能被正确捕获为图片。
/// 这一类格式 arboard 3.x 读不到（它只看 PNG / CF_DIBV5），正是此前截图丢失的根因。
#[cfg(all(test, target_os = "windows"))]
mod win_capture_tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer, Rgb, ImageFormat};

    #[test]
    fn captures_cf_bitmap_like_screenshot_tool() {
        // 构造一张 2×2 红色 BMP，模拟截图软件仅写入 CF_BITMAP 的场景。
        let img = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_pixel(2, 2, Rgb([255, 0, 0]));
        let dyn_img = DynamicImage::ImageRgb8(img);
        let mut bmp: Vec<u8> = Vec::new();
        dyn_img
            .write_to(&mut std::io::Cursor::new(&mut bmp), ImageFormat::Bmp)
            .expect("encode bmp");

        // 清空剪贴板后再只写入 CF_BITMAP，避免既有其它格式（PNG/DIBV5）干扰断言。
        clipboard_win::raw::open().expect("open clipboard");
        clipboard_win::empty().expect("empty clipboard");
        clipboard_win::raw::set_bitmap(&bmp)
            .expect("set CF_BITMAP to clipboard");
        clipboard_win::raw::close().expect("close clipboard");

        let raw = read_clipboard().expect("read_clipboard should succeed");
        match raw {
            RawContent::Image { width, height, bytes } => {
                assert_eq!((width, height), (2, 2));
                assert_eq!(bytes.len(), 2 * 2 * 4, "应为 RGBA，每像素 4 字节");
            }
            _ => panic!("期望捕获为 Image，实际不是 Image 变体（截图丢失的根因未修复）"),
        }
    }

    #[test]
    fn resolve_exe_name_returns_self() {
        // 验证 Windows 来源识别的核心：由 PID 反查 exe 名。
        // 用测试进程自身 PID，确定性地证明 Win32 解析逻辑可用（不再是 "unknown"）。
        let pid = std::process::id();
        let name = resolve_exe_name(pid).expect("应能由 PID 解析出本进程 exe 名");
        assert!(!name.is_empty(), "exe 名不应为空");
        assert_ne!(name, "unknown", "不应回退为 unknown");
        // 测试二进制名形如 clipstack-<hash>（含连字符），至少验证它不是 unknown 且非空即可。
        println!("[clipstack-test] resolve_exe_name({pid}) = {name}");
    }
}
