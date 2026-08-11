// P2 · Tauri 命令层
//
// 命名：`动词_名词`（见开发规范）。入参 / 出参为 `models.rs` 的 serde 结构体。
// 错误统一转 `String`（Tauri 要求命令错误可序列化），便于前端处理。

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::AppState;
use crate::clipboard::MonitorState;
use crate::crypto;
use crate::db::{self, DbState, DEFAULT_LIMIT, now_ms};
use crate::keychain;
use crate::lan::{LanManager, PeerInfo};
use crate::models::{ContentType, HistoryItem, NewItem, Setting};
use uuid::Uuid;

/// P0：内容类命令门禁——锁定态返回 Err("locked")。
fn ensure_unlocked(state: &AppState) -> Result<(), String> {
    if state.is_locked() {
        Err("locked".into())
    } else {
        Ok(())
    }
}

/// 新增条目（手动添加 / 前端回写场景）。返回新行 id。
#[tauri::command]
pub fn add_item(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    item: NewItem,
) -> Result<i64, String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let conn = db.lock();
    let key_guard = db.key.lock().expect("key lock poisoned");
    db::insert_or_bump(&conn, key_guard.as_ref(), &item).map_err(|e| e.to_string())
}

/// 读取历史（默认 500 条、置顶优先、时间倒序）。
#[tauri::command]
pub fn get_history(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<HistoryItem>, String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    let conn = db.lock();
    let key_guard = db.key.lock().expect("key lock poisoned");
    db::get_history(&conn, key_guard.as_ref(), limit, true).map_err(|e| e.to_string())
}

/// 删除条目（移入回收站）。
#[tauri::command]
pub fn delete_item(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let mut conn = db.lock();
    db::delete_item(&mut conn, id).map_err(|e| e.to_string())
}

/// 读取回收站（按删除时间倒序）。
#[tauri::command]
pub fn get_trash(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
) -> Result<Vec<HistoryItem>, String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let conn = db.lock();
    let key_guard = db.key.lock().expect("key lock poisoned");
    db::get_trash(&conn, key_guard.as_ref()).map_err(|e| e.to_string())
}

/// 恢复：从回收站移回历史。
#[tauri::command]
pub fn restore_item(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let mut conn = db.lock();
    db::restore_item(&mut conn, id).map_err(|e| e.to_string())
}

/// 彻底删除：从回收站永久移除。
#[tauri::command]
pub fn purge_item(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let mut conn = db.lock();
    db::purge_item(&mut conn, id).map_err(|e| e.to_string())
}

/// 清空回收站。
#[tauri::command]
pub fn empty_trash(db: State<'_, DbState>, state: State<'_, AppState>) -> Result<(), String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let conn = db.lock();
    db::empty_trash(&conn).map_err(|e| e.to_string())
}

/// 清空全部历史（软删入回收站，可回收站恢复）。
#[tauri::command]
pub fn clear_history(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let mut conn = db.lock();
    db::clear_history(&mut conn).map_err(|e| e.to_string())
}

/// 按 id 批量删除（软删入回收站，可回收站恢复）。用于「按当前查询条件清除」：
/// 前端把 `filterItems` 命中的 id 列表传入，仅删除这些行，不影响其它条目。
#[tauri::command]
pub fn delete_items(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<usize, String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let mut conn = db.lock();
    db::delete_items(&mut conn, &ids).map_err(|e| e.to_string())
}

/// 切换置顶，返回切换后状态。
#[tauri::command]
pub fn toggle_pin(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    id: i64,
) -> Result<bool, String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let conn = db.lock();
    db::toggle_pin(&conn, id).map_err(|e| e.to_string())
}

/// 切换收藏，返回切换后状态。
#[tauri::command]
pub fn toggle_favorite(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    id: i64,
) -> Result<bool, String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let conn = db.lock();
    db::toggle_favorite(&conn, id).map_err(|e| e.to_string())
}

/// 写入 / 覆盖单个设置项。
/// 若保存的键为托盘历史条数，则广播 `tray-settings-changed`，使托盘菜单立即按新值刷新。
#[tauri::command]
pub fn update_setting(
    app: AppHandle,
    db: State<'_, DbState>,
    key: String,
    value: String,
) -> Result<(), String> {
    // 注意：app.emit 会同步触发监听器并再次 lock db，必须先释放本命令持有的 db 锁，
    // 否则 build_menu 在监听器内等待 db 锁 → 与命令持有的锁相互等待 → 死锁卡死。
    // 仅对影响托盘菜单内容的设置变更才重建托盘（避免每次写设置都重建）。
    // mask_sensitive 改变后，托盘最近历史需按新开关重新脱敏。
    let refresh_tray = key == "tray_history_count" || key == "mask_sensitive";
    {
        let conn = db.lock();
        db::update_setting(&conn, &key, &value).map_err(|e| e.to_string())?;
    }
    if refresh_tray {
        let _ = app.emit("tray-settings-changed", ());
    }
    Ok(())
}

/// 读取全部设置项。
#[tauri::command]
pub fn get_settings(db: State<'_, DbState>) -> Result<Vec<Setting>, String> {
    let conn = db.lock();
    db::get_settings(&conn).map_err(|e| e.to_string())
}

/// 将来源应用加入忽略列表：同时更新内存过滤集合（即时生效）与持久化表（重启保留）。
#[tauri::command]
pub fn add_ignored_app(
    db: State<'_, DbState>,
    monitor: State<'_, Arc<Mutex<MonitorState>>>,
    name: String,
) -> Result<(), String> {
    crate::clipboard::ignore_app(&monitor, &name);
    let conn = db.lock();
    db::insert_ignored_app(&conn, &name).map_err(|e| e.to_string())
}

/// 读取全部忽略应用名（系统原始显示名）。
#[tauri::command]
pub fn get_ignored_apps(db: State<'_, DbState>) -> Result<Vec<String>, String> {
    let conn = db.lock();
    db::get_ignored_apps(&conn).map_err(|e| e.to_string())
}

/// 从忽略列表移除应用（内存集 + 持久化同时清理，即时生效）。
#[tauri::command]
pub fn remove_ignored_app(
    db: State<'_, DbState>,
    monitor: State<'_, Arc<Mutex<MonitorState>>>,
    name: String,
) -> Result<(), String> {
    crate::clipboard::unignore_app(&monitor, &name);
    let conn = db.lock();
    db::delete_ignored_app(&conn, &name).map_err(|e| e.to_string())
}

/// 枚举系统中已安装应用的显示名（小写），供忽略应用从系统列表选择。
#[tauri::command]
pub fn list_installed_apps() -> Vec<String> {
    crate::clipboard::list_installed_apps()
}

/// 一键复制：将文本条目内容写回系统剪贴板（文本 / 链接 / 代码）。
/// 图片请使用 `copy_image` 命令（需从数据库读取二进制并解码）。
/// 文件因平台 API 限制暂不支持。
#[tauri::command]
pub fn copy_item(
    content_type: ContentType,
    content_text: String,
    monitor: State<'_, Arc<Mutex<MonitorState>>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    if matches!(content_type, ContentType::Image | ContentType::File) {
        return Err("该类型暂不支持一键复制".into());
    }
    // 先占位，避免监控线程把主动复制的内容重新捕获（改写时间 / 重复入列）。
    crate::clipboard::note_self_copy(&monitor, &content_text);
    crate::clipboard::set_clipboard_text(&content_text)
}

/// 一键复制图片：从数据库读取 PNG 二进制（已解密），解码后写回系统剪贴板。
#[tauri::command]
pub fn copy_image(
    id: i64,
    db: State<'_, DbState>,
    monitor: State<'_, Arc<Mutex<MonitorState>>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let png_bytes = db::read_item_raw(&db, id, "history")
        .and_then(|(b, _)| b)
        .ok_or_else(|| "该图片无二进制数据".to_string())?;
    // 先占位，避免监控线程把主动复制的图片重新捕获。
    crate::clipboard::note_self_copy_image(&monitor, &png_bytes);
    crate::clipboard::set_clipboard_image(&png_bytes)
}

/// 一键复制文件：从数据库读取路径列表（已解密），写回系统剪贴板文件列表，可粘贴为文件本身。
#[tauri::command]
pub fn copy_file(
    id: i64,
    db: State<'_, DbState>,
    monitor: State<'_, Arc<Mutex<MonitorState>>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let (blob, text) = db::read_item_raw(&db, id, "history")
        .ok_or_else(|| "该条目不存在".to_string())?;
    let paths = paths_from_storage(blob.as_deref(), &text);
    if paths.is_empty() {
        return Err("该条目无可复制的文件路径".to_string());
    }
    // 先占位，避免监控线程把主动复制的文件重新捕获。
    crate::clipboard::note_self_copy_files(&monitor, &paths);
    crate::clipboard::set_clipboard_file_list(&paths)
}

/// 从存储中还原文件路径列表：优先取 content_blob 中的 JSON 数组；
/// 缺失或解析失败时回退到按 ", " 拆分 content_text（兼容旧数据）。
pub(crate) fn paths_from_storage(blob: Option<&[u8]>, text: &str) -> Vec<PathBuf> {
    if let Some(b) = blob {
        if let Ok(paths) = serde_json::from_slice::<Vec<String>>(b) {
            return paths.into_iter().map(PathBuf::from).collect();
        }
    }
    text.split(", ")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// 另存为：把图片 / 文件保存到用户指定位置。
/// - image：`content_blob`（PNG 字节）写入 `target` 完整路径。
/// - file：解析 `content_blob` 的本地路径列表；单文件则复制到 `target`（完整路径）；
///   多文件或 `target` 为已存在目录时，复制每个文件到 `target` 目录下（文件名冲突加 -1/-2 后缀）。
#[tauri::command]
pub fn save_item_as(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    id: i64,
    target: String,
    kind: String,
) -> Result<String, String> {
    ensure_unlocked(&state)?;
    state.touch_activity();
    let (blob, text) = db::read_item_raw(&db, id, "history")
        .ok_or_else(|| "该条目不存在".to_string())?;
    if kind == "image" {
        let bytes = blob.ok_or_else(|| "该图片无二进制数据".to_string())?;
        std::fs::write(&target, &bytes).map_err(|e| format!("保存图片失败: {e}"))?;
        Ok(format!("图片已保存到 {target}"))
    } else if kind == "file" {
        let paths = paths_from_storage(blob.as_deref(), &text);
        if paths.is_empty() {
            return Err("该条目无可保存的文件路径".to_string());
        }
        let target_path = std::path::Path::new(&target);
        let is_dir_target = target_path.is_dir();
        if paths.len() == 1 && !is_dir_target {
            // `std::fs::copy` 只能复制常规文件；源可能是文件夹（复制目录时剪贴板路径即目录），
            // 此时需递归复制整个目录，否则报 "neither a regular file nor a symlink" 错误。
            let src = &paths[0];
            let meta = std::fs::symlink_metadata(src)
                .map_err(|e| format!("保存失败: 源不存在或无法访问: {e}"))?;
            if meta.is_dir() {
                copy_dir_all(src, target_path).map_err(|e| format!("保存文件夹失败: {e}"))?;
                Ok(format!("文件夹已保存到 {target}"))
            } else {
                std::fs::copy(src, target_path).map_err(|e| format!("保存文件失败: {e}"))?;
                Ok(format!("文件已保存到 {target}"))
            }
        } else {
            std::fs::create_dir_all(target_path).map_err(|e| format!("创建目录失败: {e}"))?;
            let mut count = 0usize;
            for src in &paths {
                let name = src
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                let dest = unique_save_path(target_path, &name);
                // 源可能不存在（如 LAN 共享未落盘），跳过而非中断；目录则递归复制。
                let meta = match std::fs::symlink_metadata(src) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let ok = if meta.is_dir() {
                    copy_dir_all(src, &dest).is_ok()
                } else {
                    std::fs::copy(src, &dest).is_ok()
                };
                if ok {
                    count += 1;
                }
            }
            if count == 0 {
                return Err("没有文件被保存".to_string());
            }
            Ok(format!("已保存 {count} 个文件/文件夹到 {target}"))
        }
    } else {
        Err("仅图片与文件支持另存".to_string())
    }
}

/// 递归复制目录（含子目录与文件）到 `dest`；`dest` 不存在则创建。
/// 用于「另存为」保存剪贴板中的文件夹（目录条目）。
fn copy_dir_all(src: &std::path::Path, dest: &std::path::Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        let new_dest = dest.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_all(&path, &new_dest)?;
        } else if ft.is_symlink() {
            // 符号链接按链接本身复制，避免跟随后报错或越界。
            let _ = std::fs::remove_file(&new_dest);
            std::fs::copy(&path, &new_dest)?;
        } else {
            std::fs::copy(&path, new_dest)?;
        }
    }
    Ok(())
}

/// 计算不冲突的保存目标路径：文件名已存在时追加 `-1` / `-2` 后缀。
fn unique_save_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let base = dir.join(name);
    if !base.exists() {
        return base;
    }
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = base.extension().map(|e| e.to_string_lossy().into_owned());
    let mut i = 1u32;
    loop {
        let candidate = if let Some(e) = &ext {
            dir.join(format!("{stem}-{i}.{e}"))
        } else {
            dir.join(format!("{stem}-{i}"))
        };
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

/// 读取条目的二进制内容（图片为 PNG 字节，已解密），用于详情面板预览。
/// 文本 / 链接 / 代码类条目无二进制，返回错误。
#[tauri::command]
pub fn get_item_blob(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    id: i64,
) -> Result<Vec<u8>, String> {
    ensure_unlocked(&state)?;
    let (blob, _) = db::read_item_raw(&db, id, "history")
        .ok_or_else(|| "该条目不存在".to_string())?;
    blob.ok_or_else(|| "该条目无二进制内容（仅图片支持预览）".to_string())
}

/// 读取回收站条目的二进制内容（图片为 PNG 字节，已解密），用于回收站详情面板预览。
/// 与 `get_item_blob` 类似，但查询的是 `trash` 表。
#[tauri::command]
pub fn get_trash_blob(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    id: i64,
) -> Result<Vec<u8>, String> {
    ensure_unlocked(&state)?;
    let (blob, _) = db::read_item_raw(&db, id, "trash")
        .ok_or_else(|| "该条目不存在".to_string())?;
    blob.ok_or_else(|| "该条目无二进制内容（仅图片支持预览）".to_string())
}

/// 关于系统：返回运行平台与处理器架构（应用版本 / Tauri 版本由前端 app API 获取）。
#[derive(serde::Serialize)]
pub struct SystemInfo {
    pub platform: String,
    pub arch: String,
}

#[tauri::command]
pub fn get_system_info() -> SystemInfo {
    let platform = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
    .to_string();
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "Apple Silicon (aarch64)",
        other => other,
    }
    .to_string();
    SystemInfo { platform, arch }
}

/// 读取启动阶段写入的「是否首次运行」标志。
/// 首次运行的窗口显示与标记写入已在 `setup` 阶段同步完成；前端据此决定是否自动进入设置页。
#[tauri::command]
pub fn was_first_run(flag: State<'_, Arc<AtomicBool>>) -> bool {
    flag.load(Ordering::SeqCst)
}

// ===== P0 · 应用锁 / 主密码 =====

/// 首次设置主密码：仅写入校验信息（盐 + 哈希）。
///
/// 设计调整：数据库内容加密由程序内部的固定密钥负责（启动阶段已载入内存），
/// 与主密码无关；主密码只作为「应用锁」凭据。因此设置主密码**不派生也不写入任何
/// 数据库加密密钥**，也不重加密已有数据。Touch ID 解锁令牌由 `set_touch_id` 另行写入。
#[tauri::command]
pub fn setup_master_password(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    pwd: String,
) -> Result<(), String> {
    if pwd.len() < 6 {
        return Err("主密码至少 6 位".into());
    }
    let conn = db.conn.lock().expect("db lock poisoned");
    if db::has_master_password(&conn) {
        return Err("主密码已设置，请使用「修改主密码」".into());
    }
    let salt = crypto::random_salt();
    let salt_b64 = STANDARD.encode(salt);
    let verifier = crypto::hash_password(&pwd, &salt);
    db::set_master_password(&conn, &salt_b64, &verifier)
        .map_err(|e| e.to_string())?;
    drop(conn);

    // 内部数据库密钥已在启动阶段载入内存（见 lib.rs），此处无需再从主密码派生。
    // 明文历史迁移同样在启动阶段完成（mig_enc_v1）。
    state.set_locked(false);
    Ok(())
}

/// 主密码解锁：校验通过后解除「应用锁」。
///
/// 设计调整：内部数据库密钥始终在内存，主密码仅用于解锁 UI / 托盘展示，
/// 不再派生或载入任何加解密密钥。
#[tauri::command]
pub fn unlock(
    app: AppHandle,
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    pwd: String,
) -> Result<bool, String> {
    let conn = db.conn.lock().expect("poisoned");
    let verifier = db::get_pw_verifier(&conn);
    if verifier.is_empty() {
        return Err("尚未设置主密码".into());
    }
    if !crypto::verify_password(&pwd, &verifier) {
        return Ok(false);
    }
    drop(conn);

    // 内部密钥已常驻内存，此处仅解除应用锁；随后刷新托盘菜单。
    state.set_locked(false);
    let _ = app.emit("refresh-tray", ());
    Ok(true)
}

/// Touch ID 解锁：通过系统 `LocalAuthentication` 验证当前登录用户后解除「应用锁」。
/// 非 macOS 平台回退错误（请用主密码）。
///
/// 实现说明：macOS 26+ 的 LAContext 已移除同步版 `evaluatePolicy:localizedReason:error:`，
/// 且裸二进制经 objc2 直接调用会抛 `unrecognized selector` 崩溃。故由
/// `keychain::authenticate_user` 启动 Swift 子进程（独立进程、Apple 签名）完成
/// Touch ID / 登录密码验证——有 Touch ID 时弹生物识别，否则回退设备登录密码。
/// 不依赖钥匙串中任何存储的秘密。
#[tauri::command]
pub fn unlock_touch_id(
    app: AppHandle,
    db: State<'_, DbState>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let conn = db.conn.lock().expect("poisoned");
    if !db::has_master_password(&conn) {
        return Err("尚未设置主密码".into());
    }
    drop(conn);

    // 验证当前登录用户（Touch ID / 登录密码），由 Swift 子进程独立完成。
    keychain::authenticate_user("解锁 ClipStack")?;

    // 内部密钥已常驻内存，此处仅解除应用锁；随后刷新托盘菜单。
    state.set_locked(false);
    let _ = app.emit("refresh-tray", ());
    Ok(true)
}

/// 锁定：仅置「应用锁」标记，使 UI / 托盘历史不可读。
///
/// 设计调整：不再清空内存中的内部数据库密钥——数据库加密由程序内部密钥负责、
/// 与主密码无关，锁定只保护界面与托盘展示，不影响落库数据的加密状态。
#[tauri::command]
pub fn lock(app: AppHandle, _db: State<'_, DbState>, state: State<'_, AppState>) -> Result<(), String> {
    state.set_locked(true);
    // 锁定后刷新托盘菜单，用「已锁定」占位项替换历史，避免托盘泄露。
    let _ = app.emit("refresh-tray", ());
    // 「立即锁定」专用：锁定后由后端直接隐藏主窗体（Rust 端 hide 不受前端
    // capability 权限约束，最可靠）。解锁对话框仅在下次从托盘重新 show 时出现。
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    Ok(())
}

/// 上报一次用户活动：重置「闲置自动锁定」计时，避免活跃使用时被误锁。
/// 由前端在用户与界面交互（点击 / 按键 / 窗口聚焦）时调用，做了限流。
#[tauri::command]
pub fn touch_activity(state: State<'_, AppState>) {
    state.touch_activity();
}

/// 修改主密码：仅更新校验信息。
///
/// 设计调整：主密码仅控制「应用锁」，与数据库加密密钥无关，故修改主密码
/// **不重加密数据、不动内部密钥、不写钥匙串**。
#[tauri::command]
pub fn change_master_password(
    db: State<'_, DbState>,
    state: State<'_, AppState>,
    old_pwd: String,
    new_pwd: String,
) -> Result<(), String> {
    if new_pwd.len() < 6 {
        return Err("新主密码至少 6 位".into());
    }
    let conn = db.conn.lock().expect("poisoned");
    let verifier = db::get_pw_verifier(&conn);
    if !crypto::verify_password(&old_pwd, &verifier) {
        return Err("原主密码错误".into());
    }
    let salt = crypto::random_salt();
    let salt_b64 = STANDARD.encode(salt);
    let new_verifier = crypto::hash_password(&new_pwd, &salt);
    db::set_master_password(&conn, &salt_b64, &new_verifier)
        .map_err(|e| e.to_string())?;
    drop(conn);

    // 内部密钥与落库数据均不受主密码影响，无需任何重加密或钥匙串操作。
    state.set_locked(false);
    Ok(())
}

/// 清除主密码：无需输入旧密码，直接移除「应用锁」凭据与 Touch ID 设置。
///
/// 设计调整：数据库内容由内部密钥加密、与主密码无关，清除主密码**不解密**已加密数据、
/// 不删除内部密钥、不触碰落库内容。清除后应用回到「无应用锁」状态（数据仍以内部密钥加密）。
#[tauri::command]
pub fn clear_master_password(
    app: AppHandle,
    db: State<'_, DbState>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let conn = db.conn.lock().expect("poisoned");
    if !db::has_master_password(&conn) {
        return Err("尚未设置主密码".into());
    }
    // 移除主密码校验信息并关闭 Touch ID 设置（Touch ID 是主密码的增强，无主密码即无意义）。
    db::clear_master_password(&conn).map_err(|e| e.to_string())?;
    drop(conn);

    // 清理旧版遗留的 Touch ID 令牌钥匙串项（若有）。
    let _ = keychain::delete_unlock_key();
    // 内部密钥仍常驻内存，无需清空；解除应用锁。
    state.set_locked(false);
    // 刷新托盘菜单：解除锁定后从「已锁定」占位切回最近历史展示。
    let _ = app.emit("refresh-tray", ());
    Ok(())
}

/// 启用 / 关闭 Touch ID 解锁：仅切换 `use_touch_id` 开关。
///
/// 解锁本身由系统 `LocalAuthentication`（LAContext）直接验证当前登录用户——
/// 有 Touch ID 时弹 Touch ID，否则回退登录密码；钥匙串中不再存放任何解锁令牌。
/// 关闭时清理旧版遗留的 BiometryCurrentSet 钥匙串项（若有），避免无用项残留。
/// 内部数据库加密密钥（clipstack.enc）不受影响。
#[tauri::command]
pub fn set_touch_id(db: State<'_, DbState>, enabled: bool) -> Result<(), String> {
    {
        let conn = db.lock();
        db::update_setting(&conn, "use_touch_id", if enabled { "1" } else { "0" })
            .map_err(|e| e.to_string())?;
    }
    if !enabled {
        // 清理旧版遗留的 Touch ID 令牌钥匙串项（受 BiometryCurrentSet 保护）。
        let _ = keychain::delete_unlock_key();
    }
    Ok(())
}

/// 当前是否处于锁定态。
#[tauri::command]
pub fn is_locked(state: State<'_, AppState>) -> bool {
    state.is_locked()
}

/// 是否已设置主密码（前端据此决定显示「设置密码」还是「解锁」）。
#[tauri::command]
pub fn has_master_password(db: State<'_, DbState>) -> bool {
    let conn = db.conn.lock().expect("db lock poisoned");
    db::has_master_password(&conn)
}

/// P1b：按 `retention_days` 清理超期历史（未置顶）与回收站内容，返回删除条数。
/// 锁定态亦可执行（仅按时间删除，不读取/泄露内容）。
#[tauri::command]
pub fn purge_expired(db: State<'_, DbState>) -> Result<usize, String> {
    let conn = db.conn.lock().expect("db lock poisoned");
    let days = db::get_int_setting(&conn, "retention_days", 0);
    db::purge_expired(&conn, days).map_err(|e| e.to_string())
}

/// 检测 Touch ID 解锁是否可用——即系统是否安装了 Xcode Command Line Tools
/// （Touch ID 解锁通过 `/usr/bin/swift` 子进程调用 LAContext，swift 需要 CLT 才能运行）。
/// macOS 上 `xcode-select -p` 成功即表示 CLT 已安装；非 macOS 始终返回 false。
#[tauri::command]
pub fn check_touch_id_available() -> bool {
    #[cfg(not(target_os = "macos"))]
    {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("xcode-select")
            .arg("-p")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_from_storage_prefers_json_blob() {
        let json = serde_json::to_vec(&vec!["/a.txt", "/b/c.png"]).unwrap();
        let paths = paths_from_storage(Some(&json), "/ignored, fallback");
        assert_eq!(
            paths,
            vec![PathBuf::from("/a.txt"), PathBuf::from("/b/c.png")]
        );
    }

    #[test]
    fn paths_from_storage_falls_back_to_comma_split() {
        // 无 blob 时回退到按 ", " 拆分 content_text（兼容旧数据）。
        let paths = paths_from_storage(None, "/x.txt, /y.txt");
        assert_eq!(paths, vec![PathBuf::from("/x.txt"), PathBuf::from("/y.txt")]);
    }

    #[test]
    fn paths_from_storage_bad_blob_falls_back() {
        // blob 非合法 JSON 时回退到 ", " 拆分，不报错。
        let paths = paths_from_storage(Some(b"not json"), "/only.txt");
        assert_eq!(paths, vec![PathBuf::from("/only.txt")]);
    }

    #[test]
    fn paths_from_storage_empty_yields_empty() {
        let paths = paths_from_storage(None, "");
        assert!(paths.is_empty());
    }
}

// ===== 局域网共享（L2/L3，见 docs/clipstack-lan-sync-design.md）=====

/// 设置局域网共享配置并重启发现（组/密钥变更需重新注册 mDNS 指纹）。
/// 采用平铺入参（与 lan_set_share_out 等命令一致），Tauri v2 直接按参数名传参，
/// 避免单一结构体入参需 `input` 包裹带来的前后端不一致问题。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn lan_set_config(
    state: State<'_, LanManager>,
    group: String,
    key: String,
    name: String,
    share_out: bool,
    file_limit_mb: u64,
    share_types: Vec<String>,
    manual_peers: Vec<String>,
    port: u16,
) -> Result<(), String> {
    let mut cfg = state.config().await;
    cfg.share_group = group;
    // 空密钥表示「保持现有密钥」：避免 UI 未重新输入密钥时误清空共享密钥。
    if !key.is_empty() {
        cfg.share_key = key;
    }
    cfg.device_name = name;
    cfg.share_out = share_out;
    cfg.file_limit_mb = file_limit_mb.max(1);
    // 仅允许白名单内的类型值，避免前端传入脏数据（如 "video"）。
    let mut ts: Vec<String> = share_types
        .into_iter()
        .filter(|t| matches!(t.as_str(), "text" | "image" | "file"))
        .collect();
    ts.sort();
    ts.dedup();
    cfg.share_types = ts;
    cfg.manual_peers = manual_peers;
    // 端口：0 视为「恢复默认」(LAN_PORT)；u16 天然限制在 1..=65535。
    cfg.listen_port = if port == 0 {
        crate::lan::LAN_PORT
    } else {
        port
    };
    state.set_config(cfg).await;
    Ok(())
}

/// 向所有已连对端广播一条测试文本（L2 验证用；L3 由监控线程直接调用 broadcast）。
#[tauri::command]
pub async fn lan_send_test(state: State<'_, LanManager>, text: String) -> Result<usize, String> {
    let item = crate::lan::text_item(&text);
    Ok(state.broadcast_clip(item).await)
}

/// 当前组内在线设备列表。
#[tauri::command]
pub async fn lan_get_peers(state: State<'_, LanManager>) -> Result<Vec<PeerInfo>, String> {
    Ok(state.peers().await)
}

/// 当前局域网配置视图（不含明文密钥）。
#[tauri::command]
pub async fn lan_get_config(state: State<'_, LanManager>) -> Result<LanConfigView, String> {
    let cfg = state.config().await;
    Ok(LanConfigView {
        device_id: cfg.device_id,
        group: cfg.share_group,
        name: cfg.device_name,
        share_out: cfg.share_out,
        file_limit_mb: cfg.file_limit_mb,
        share_types: cfg.share_types,
        has_key: !cfg.share_key.is_empty(),
        manual_peers: cfg.manual_peers,
        port: cfg.listen_port,
        local_ip: crate::lan::local_ipv4()
            .map(|ip| ip.to_string())
            .unwrap_or_default(),
    })
}

/// 局域网配置视图（不泄露明文密钥）。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanConfigView {
    pub device_id: String,
    pub group: String,
    pub name: String,
    pub share_out: bool,
    pub file_limit_mb: u64,
    pub share_types: Vec<String>,
    pub has_key: bool,
    pub manual_peers: Vec<String>,
    pub port: u16,
    pub local_ip: String,
}

/// 按需返回当前明文共享密钥（仅当用户点击「显示」图标时调用，
/// 不随 lan_get_config 自动返回，避免配置加载即泄露密钥）。
#[tauri::command]
pub async fn lan_get_key(state: State<'_, LanManager>) -> Result<String, String> {
    let cfg = state.config().await;
    Ok(cfg.share_key)
}

// ===== 局域网共享 · 配置管理（L3）=====

/// 局域网共享配置视图（不泄露明文密钥）。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanProfileView {
    pub id: String,
    pub group: String,
    pub mode: String,
    pub is_active: bool,
    pub has_key: bool,
}

/// 列出全部共享配置。
#[tauri::command]
pub async fn lan_list_profiles(db: State<'_, DbState>) -> Result<Vec<LanProfileView>, String> {
    let conn = db.conn.lock().unwrap();
    let profiles = db::list_sync_profiles(&conn).map_err(|e| e.to_string())?;
    Ok(profiles
        .into_iter()
        .map(|p| LanProfileView {
            id: p.id,
            group: p.share_group,
            mode: p.mode,
            is_active: p.is_active,
            has_key: true,
        })
        .collect())
}

/// 新建共享配置（密钥经内部密钥包装为 wrapped_key 落库，不裸存）。
/// `activate=true` 时同时激活并重启发现。
#[tauri::command]
pub async fn lan_upsert_profile(
    state: State<'_, LanManager>,
    db: State<'_, DbState>,
    group: String,
    key: String,
    name: String,
    activate: bool,
) -> Result<(), String> {
    let id = Uuid::new_v4().to_string();
    let wrapped = wrap_profile_key(&db, &key).ok_or("key wrap failed")?;
    let created = now_ms();
    {
        let conn = db.conn.lock().unwrap();
        db::upsert_sync_profile(&conn, &id, "lan", &group, &wrapped, activate, created)
            .map_err(|e| e.to_string())?;
        if activate {
            db::set_active_profile(&conn, &id).map_err(|e| e.to_string())?;
        }
    }
    if activate {
        let plaintext = unwrap_profile_key(&db, &wrapped).ok_or("key unwrap failed")?;
        let mut cfg = state.config().await;
        cfg.share_group = group;
        cfg.share_key = plaintext;
        cfg.device_name = name;
        state.set_config(cfg).await;
    }
    Ok(())
}

/// 激活某共享配置：载入明文密钥并重启发现。
#[tauri::command]
pub async fn lan_set_active_profile(
    state: State<'_, LanManager>,
    db: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    let (wrapped, group) = {
        let conn = db.conn.lock().unwrap();
        db::set_active_profile(&conn, &id).map_err(|e| e.to_string())?;
        db::get_profile_creds(&conn, &id).map_err(|e| e.to_string())?
    };
    let plaintext = unwrap_profile_key(&db, &wrapped).ok_or("key unwrap failed")?;
    let mut cfg = state.config().await;
    cfg.share_group = group;
    cfg.share_key = plaintext;
    state.set_config(cfg).await;
    Ok(())
}

/// 删除共享配置；若删除的是激活项，停止发现并复位为未配置。
#[tauri::command]
pub async fn lan_delete_profile(
    state: State<'_, LanManager>,
    db: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    let was_active = {
        let conn = db.conn.lock().unwrap();
        let active = db::is_profile_active(&conn, &id);
        db::delete_sync_profile(&conn, &id).map_err(|e| e.to_string())?;
        active
    };
    if was_active {
        state.stop().await;
        let mut cfg = state.config().await;
        cfg.share_group = String::new();
        cfg.share_key = String::new();
        cfg.share_out = false;
        state.set_config(cfg).await;
    }
    Ok(())
}

/// 切换发布开关（share_out）。
#[tauri::command]
pub async fn lan_set_share_out(state: State<'_, LanManager>, enabled: bool) -> Result<(), String> {
    state.set_share_out(enabled).await;
    Ok(())
}

/// 统计共享文件根目录（~/.clipstack/share）下的文件数量与总大小（递归）。
#[tauri::command]
pub async fn lan_share_stats(app: AppHandle) -> Result<(u64, u64), String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let root = crate::lan::share_root(&home);
    let mut count: u64 = 0;
    let mut size: u64 = 0;
    if let Ok(mut stack) = std::fs::read_dir(&root) {
        while let Some(Ok(entry)) = stack.next() {
            let p = entry.path();
            if let Ok(meta) = p.metadata() {
                if meta.is_file() {
                    count += 1;
                    size += meta.len();
                } else if meta.is_dir() {
                    // 月份子目录，递归累加其中的文件。
                    if let Ok(mut sub) = std::fs::read_dir(&p) {
                        while let Some(Ok(child)) = sub.next() {
                            let cp = child.path();
                            if let Ok(cmeta) = cp.metadata() {
                                if cmeta.is_file() {
                                    count += 1;
                                    size += cmeta.len();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok((count, size))
}

/// 返回共享文件夹（~/.clipstack/share）的绝对路径，供前端展示。
#[tauri::command]
pub async fn lan_share_folder_path(app: AppHandle) -> Result<String, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    Ok(crate::lan::share_root(&home).to_string_lossy().to_string())
}

/// 在文件管理器中打开共享文件夹（~/.clipstack/share），不存在则先创建。
#[tauri::command]
pub async fn lan_open_share_folder(app: AppHandle) -> Result<(), String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let root = crate::lan::share_root(&home);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.to_string_lossy().to_string();
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(&path).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("explorer").arg(&path).status();
    #[cfg(target_os = "linux")]
    let status = std::process::Command::new("xdg-open").arg(&path).status();
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let status: Result<std::process::ExitStatus, std::io::Error> =
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "unsupported platform"));
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("open failed, exit code {s}")),
        Err(e) => Err(format!("open failed: {e}")),
    }
}

/// 清空共享文件夹（~/.clipstack/share）内的全部文件与子目录，返回删除的文件数。
#[tauri::command]
pub async fn lan_clear_share_files(app: AppHandle) -> Result<u64, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let root = crate::lan::share_root(&home);
    let mut removed: u64 = 0;
    if !root.exists() {
        return Ok(0);
    }
    let mut stack = std::fs::read_dir(&root).map_err(|e| e.to_string())?;
    while let Some(Ok(entry)) = stack.next() {
        let p = entry.path();
        if let Ok(meta) = p.metadata() {
            if meta.is_file() {
                if std::fs::remove_file(&p).is_ok() {
                    removed += 1;
                }
            } else if meta.is_dir() {
                // 递归删除整目录并累加其中文件数。
                removed += remove_dir_all_count(&p);
                let _ = std::fs::remove_dir(&p);
            }
        }
    }
    Ok(removed)
}

/// 递归删除目录并返回删除的文件数（尽力而为，不要求每个文件都成功）。
fn remove_dir_all_count(dir: &std::path::Path) -> u64 {
    let mut count: u64 = 0;
    if let Ok(mut stack) = std::fs::read_dir(dir) {
        while let Some(Ok(entry)) = stack.next() {
            let p = entry.path();
            if let Ok(meta) = p.metadata() {
                if meta.is_file() {
                    if std::fs::remove_file(&p).is_ok() {
                        count += 1;
                    }
                } else if meta.is_dir() {
                    count += remove_dir_all_count(&p);
                    let _ = std::fs::remove_dir(&p);
                }
            }
        }
    }
    count
}

/// 内部：用内部数据库密钥加密共享密钥（包装为 wrapped_key 落库）。
fn wrap_profile_key(db: &DbState, plain: &str) -> Option<String> {
    let key = db.key.lock().unwrap();
    let k = key.as_ref()?;
    let sealed = crypto::encrypt(k, plain.as_bytes());
    Some(STANDARD.encode(sealed))
}

/// 内部：解开 wrapped_key 还原明文共享密钥。
fn unwrap_profile_key(db: &DbState, wrapped: &str) -> Option<String> {
    let sealed = STANDARD.decode(wrapped).ok()?;
    let key = db.key.lock().unwrap();
    let k = key.as_ref()?;
    let plain = crypto::decrypt(k, &sealed)?;
    String::from_utf8(plain).ok()
}
