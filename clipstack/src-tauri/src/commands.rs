// P2 · Tauri 命令层
//
// 命名：`动词_名词`（见开发规范）。入参 / 出参为 `models.rs` 的 serde 结构体。
// 错误统一转 `String`（Tauri 要求命令错误可序列化），便于前端处理。

use std::sync::{Arc, Mutex};

use tauri::State;

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
#[tauri::command]
pub fn update_setting(db: State<'_, DbState>, key: String, value: String) -> Result<(), String> {
    let conn = db.lock();
    db::update_setting(&conn, &key, &value).map_err(|e| e.to_string())
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

/// 读取全部忽略应用名（小写）。
#[tauri::command]
pub fn get_ignored_apps(db: State<'_, DbState>) -> Result<Vec<String>, String> {
    let conn = db.lock();
    db::get_ignored_apps(&conn).map_err(|e| e.to_string())
}

/// 一键复制：将条目内容写回系统剪贴板（文本 / 链接 / 代码）。
/// 图片 / 文件因需二进制解析与平台文件 API，P3 暂不支持。
#[tauri::command]
pub fn copy_item(content_type: ContentType, content_text: String) -> Result<(), String> {
    if matches!(content_type, ContentType::Image | ContentType::File) {
        return Err("该类型暂不支持一键复制".into());
    }
    crate::clipboard::set_clipboard_text(&content_text)
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
