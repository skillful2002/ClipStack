// P2 · Tauri 命令层
//
// 命名：`动词_名词`（见开发规范）。入参 / 出参为 `models.rs` 的 serde 结构体。
// 错误统一转 `String`（Tauri 要求命令错误可序列化），便于前端处理。

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use tauri::{AppHandle, Emitter, State};

use crate::clipboard::MonitorState;
use crate::db::{self, DbState, DEFAULT_LIMIT};
use crate::models::{ContentType, HistoryItem, NewItem, Setting};

/// 新增条目（手动添加 / 前端回写场景）。返回新行 id。
#[tauri::command]
pub fn add_item(db: State<'_, DbState>, item: NewItem) -> Result<i64, String> {
    let conn = db.lock();
    db::insert_or_bump(&conn, &item).map_err(|e| e.to_string())
}

/// 读取历史（默认 500 条、置顶优先、时间倒序）。
#[tauri::command]
pub fn get_history(db: State<'_, DbState>, limit: Option<i64>) -> Result<Vec<HistoryItem>, String> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    let conn = db.lock();
    db::get_history(&conn, limit, true).map_err(|e| e.to_string())
}

/// 删除条目（移入回收站）。
#[tauri::command]
pub fn delete_item(db: State<'_, DbState>, id: i64) -> Result<(), String> {
    let mut conn = db.lock();
    db::delete_item(&mut conn, id).map_err(|e| e.to_string())
}

/// 读取回收站（按删除时间倒序）。
#[tauri::command]
pub fn get_trash(db: State<'_, DbState>) -> Result<Vec<HistoryItem>, String> {
    let conn = db.lock();
    db::get_trash(&conn).map_err(|e| e.to_string())
}

/// 恢复：从回收站移回历史。
#[tauri::command]
pub fn restore_item(db: State<'_, DbState>, id: i64) -> Result<(), String> {
    let mut conn = db.lock();
    db::restore_item(&mut conn, id).map_err(|e| e.to_string())
}

/// 彻底删除：从回收站永久移除。
#[tauri::command]
pub fn purge_item(db: State<'_, DbState>, id: i64) -> Result<(), String> {
    let mut conn = db.lock();
    db::purge_item(&mut conn, id).map_err(|e| e.to_string())
}

/// 清空回收站。
#[tauri::command]
pub fn empty_trash(db: State<'_, DbState>) -> Result<(), String> {
    let conn = db.lock();
    db::empty_trash(&conn).map_err(|e| e.to_string())
}

/// 清空全部历史（软删入回收站，可回收站恢复）。
#[tauri::command]
pub fn clear_history(db: State<'_, DbState>) -> Result<(), String> {
    let mut conn = db.lock();
    db::clear_history(&mut conn).map_err(|e| e.to_string())
}

/// 按 id 批量删除（软删入回收站，可回收站恢复）。用于「按当前查询条件清除」：
/// 前端把 `filterItems` 命中的 id 列表传入，仅删除这些行，不影响其它条目。
#[tauri::command]
pub fn delete_items(db: State<'_, DbState>, ids: Vec<i64>) -> Result<usize, String> {
    let mut conn = db.lock();
    db::delete_items(&mut conn, &ids).map_err(|e| e.to_string())
}

/// 切换置顶，返回切换后状态。
#[tauri::command]
pub fn toggle_pin(db: State<'_, DbState>, id: i64) -> Result<bool, String> {
    let conn = db.lock();
    db::toggle_pin(&conn, id).map_err(|e| e.to_string())
}

/// 切换收藏，返回切换后状态。
#[tauri::command]
pub fn toggle_favorite(db: State<'_, DbState>, id: i64) -> Result<bool, String> {
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
    let refresh_tray = key == "tray_history_count";
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
) -> Result<(), String> {
    if matches!(content_type, ContentType::Image | ContentType::File) {
        return Err("该类型暂不支持一键复制".into());
    }
    // 先占位，避免监控线程把主动复制的内容重新捕获（改写时间 / 重复入列）。
    crate::clipboard::note_self_copy(&monitor, &content_text);
    crate::clipboard::set_clipboard_text(&content_text)
}

/// 一键复制图片：从数据库读取 PNG 二进制，解码后写回系统剪贴板。
#[tauri::command]
pub fn copy_image(
    id: i64,
    db: State<'_, DbState>,
    monitor: State<'_, Arc<Mutex<MonitorState>>>,
) -> Result<(), String> {
    let conn = db.lock();
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT content_blob FROM history WHERE id = ?",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    drop(conn);
    let png_bytes = blob.ok_or_else(|| "该图片无二进制数据".to_string())?;
    // 先占位，避免监控线程把主动复制的图片重新捕获。
    crate::clipboard::note_self_copy_image(&monitor, &png_bytes);
    crate::clipboard::set_clipboard_image(&png_bytes)
}

/// 一键复制文件：从数据库读取路径列表（存于 content_blob，JSON 数组；
/// 旧格式则可能以 ", " 拼在 content_text 中），写回系统剪贴板文件列表，可粘贴为文件本身。
#[tauri::command]
pub fn copy_file(
    id: i64,
    db: State<'_, DbState>,
    monitor: State<'_, Arc<Mutex<MonitorState>>>,
) -> Result<(), String> {
    let conn = db.lock();
    let row: (Option<Vec<u8>>, String) = conn
        .query_row(
            "SELECT content_blob, content_text FROM history WHERE id = ?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    drop(conn);
    let (blob, text) = row;
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

/// 读取条目的二进制内容（图片为 PNG 字节），用于详情面板预览。
/// 文本 / 链接 / 代码类条目无二进制，返回错误。
#[tauri::command]
pub fn get_item_blob(db: State<'_, DbState>, id: i64) -> Result<Vec<u8>, String> {
    let conn = db.lock();
    // query_row 在无匹配行时返回 Err(QueryReturnedNoRows)，经 map_err 转为错误字符串。
    let row: (Option<Vec<u8>>, String) = conn
        .query_row(
            "SELECT content_blob, content_type FROM history WHERE id = ?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let (blob, _ctype) = row;
    blob.ok_or_else(|| "该条目无二进制内容（仅图片支持预览）".to_string())
}

/// 读取回收站条目的二进制内容（图片为 PNG 字节），用于回收站详情面板预览。
/// 与 `get_item_blob` 类似，但查询的是 `trash` 表。
#[tauri::command]
pub fn get_trash_blob(db: State<'_, DbState>, id: i64) -> Result<Vec<u8>, String> {
    let conn = db.lock();
    let row: (Option<Vec<u8>>, String) = conn
        .query_row(
            "SELECT content_blob, content_type FROM trash WHERE id = ?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let (blob, _ctype) = row;
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
