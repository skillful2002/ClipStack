// P2 · 持久化层（SQLite via rusqlite，bundled 编译内嵌 SQLite，免系统依赖）
//
// 设计：
//   - `AppDb` 封装 `Mutex<Connection>`，可被 Tauri State 托管（Send + Sync）。
//   - 所有 CRUD 都以 `&Connection` 自由函数实现，便于在单测中用内存库直接验证。
//   - 表结构见 `migrate`；去重（hash）与容量上限（`enforce_capacity`）在此层保证。

use std::sync::{Mutex, MutexGuard};

use rusqlite::{params, Connection};
use tauri::{AppHandle, Manager};

use crate::models::{HistoryItem, NewItem, Setting};

/// 历史条目容量上限：超出后自动硬删最旧部分（不进回收站，避免回收站无限增长）。
pub const MAX_HISTORY: i64 = 5000;
/// `get_history` 默认读取上限。
pub const DEFAULT_LIMIT: i64 = 500;

/// 受 Tauri 托管的数据库连接。
pub struct AppDb {
    pub conn: Mutex<Connection>,
}

/// Tauri State 类型（Arc 便于在监控线程间共享）。
pub type DbState = std::sync::Arc<AppDb>;

impl AppDb {
    /// 锁定并返回连接（封装，避免调用处重复 map_err）。
    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().expect("db lock poisoned")
    }
}

/// 解析应用数据目录并打开（不存在则创建）clipstack.db，执行迁移。
/// 数据统一存放于用户主目录下的 `.clipstack` 文件夹（跨 macOS / Windows / Linux 一致），
/// 不再依赖各平台的应用数据目录（如 macOS 的 ~/Library/Application Support/<identifier>）。
pub fn open(app: &AppHandle) -> Result<AppDb, Box<dyn std::error::Error>> {
    let home = app.path().home_dir()?;
    let dir = home.join(".clipstack");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("clipstack.db");
    let conn = Connection::open(&path)?;
    migrate(&conn)?;
    // 一次性迁移：纠正早期被误判为「文本」的文件条目（见函数注释）。
    let _ = migrate_files_from_text(&conn);
    Ok(AppDb {
        conn: Mutex::new(conn),
    })
}

/// 建表 + 索引（幂等）。
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS history (
            id           INTEGER PRIMARY KEY,
            content_type TEXT    NOT NULL,
            content_text TEXT    NOT NULL DEFAULT '',
            content_blob BLOB,
            source_app   TEXT    NOT NULL DEFAULT '',
            size_bytes   INTEGER NOT NULL DEFAULT 0,
            hash         TEXT    NOT NULL,
            is_pinned    INTEGER NOT NULL DEFAULT 0,
            is_favorite  INTEGER NOT NULL DEFAULT 0,
            created_at   INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_history_created ON history(created_at);
        CREATE INDEX IF NOT EXISTS idx_history_type   ON history(content_type);
        CREATE INDEX IF NOT EXISTS idx_history_hash   ON history(hash);

        CREATE TABLE IF NOT EXISTS trash (
            id           INTEGER PRIMARY KEY,
            content_type TEXT    NOT NULL,
            content_text TEXT    NOT NULL DEFAULT '',
            content_blob BLOB,
            source_app   TEXT    NOT NULL DEFAULT '',
            size_bytes   INTEGER NOT NULL DEFAULT 0,
            hash         TEXT    NOT NULL,
            is_pinned    INTEGER NOT NULL DEFAULT 0,
            is_favorite  INTEGER NOT NULL DEFAULT 0,
            created_at   INTEGER NOT NULL,
            deleted_at   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ignored_apps (
            id   INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
        );
        "#,
    )
}

/// 新增或去重置顶：同 hash 已在 history 中 → 更新 created_at 等并置顶返回原 id；
/// 否则插入新行。插入后执行容量上限清理。返回行 id。
pub fn insert_or_bump(conn: &Connection, item: &NewItem) -> rusqlite::Result<i64> {
    if let Ok(existing) =
        conn.query_row("SELECT id FROM history WHERE hash = ?", [item.hash.as_str()], |r| {
            r.get::<_, i64>(0)
        })
    {
        conn.execute(
            "UPDATE history SET created_at = ?, content_text = ?, content_blob = ?, size_bytes = ?, source_app = ? WHERE id = ?",
            params![
                item.created_at,
                item.content_text,
                item.content_blob,
                item.size_bytes,
                item.source_app,
                existing
            ],
        )?;
        let _ = enforce_capacity(conn, MAX_HISTORY);
        return Ok(existing);
    }

    conn.execute(
        "INSERT INTO history (content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at)
         VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?)",
        params![
            item.content_type.as_str(),
            item.content_text,
            item.content_blob,
            item.source_app,
            item.size_bytes,
            item.hash,
            item.created_at
        ],
    )?;
    let id = conn.last_insert_rowid();
    let _ = enforce_capacity(conn, MAX_HISTORY);
    Ok(id)
}

/// 读取历史：默认按「置顶优先、再按时间倒序」，限制条数。
pub fn get_history(conn: &Connection, limit: i64, pin_first: bool) -> rusqlite::Result<Vec<HistoryItem>> {
    let order = if pin_first {
        "is_pinned DESC, created_at DESC"
    } else {
        "created_at DESC"
    };
    let sql = format!(
        "SELECT id, content_type, content_text, source_app, size_bytes, hash, is_pinned, is_favorite, created_at \
         FROM history ORDER BY {order} LIMIT ?"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit], |r| {
        Ok(HistoryItem {
            id: r.get(0)?,
            content_type: parse_content_type(r.get::<_, String>(1)?),
            content_text: r.get(2)?,
            preview: r.get::<_, String>(2)?,
            source_app: r.get(3)?,
            size_bytes: r.get(4)?,
            hash: r.get(5)?,
            is_pinned: r.get::<_, i64>(6)? != 0,
            is_favorite: r.get::<_, i64>(7)? != 0,
            created_at: r.get(8)?,
            deleted_at: None,
        })
    })?;
    rows.collect()
}

/// 读取最近若干条（置顶优先、时间倒序），供托盘菜单展示。
pub fn get_recent(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<HistoryItem>> {
    get_history(conn, limit, true)
}

/// 按 id 读取单条历史（托盘点击复制、命令读取原文等场景）。
pub fn get_item(conn: &Connection, id: i64) -> rusqlite::Result<HistoryItem> {
    let mut stmt = conn.prepare(
        "SELECT id, content_type, content_text, source_app, size_bytes, hash, is_pinned, is_favorite, created_at \
         FROM history WHERE id = ?",
    )?;
    stmt.query_row([id], |r| {
        Ok(HistoryItem {
            id: r.get(0)?,
            content_type: parse_content_type(r.get::<_, String>(1)?),
            content_text: r.get(2)?,
            preview: r.get::<_, String>(2)?,
            source_app: r.get(3)?,
            size_bytes: r.get(4)?,
            hash: r.get(5)?,
            is_pinned: r.get::<_, i64>(6)? != 0,
            is_favorite: r.get::<_, i64>(7)? != 0,
            created_at: r.get(8)?,
            deleted_at: None,
        })
    })
}

/// 删除：从 history 移到 trash（保留完整快照 + deleted_at）。
pub fn delete_item(conn: &mut Connection, id: i64) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let deleted_at = now_ms();
    tx.execute(
        "INSERT INTO trash (id, content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, deleted_at) \
         SELECT id, content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, ? \
         FROM history WHERE id = ?",
        params![deleted_at, id],
    )?;
    tx.execute("DELETE FROM history WHERE id = ?", [id])?;
    tx.commit()
}

/// 读取回收站：按删除时间倒序。
pub fn get_trash(conn: &Connection) -> rusqlite::Result<Vec<HistoryItem>> {
    let mut stmt = conn.prepare(
        "SELECT id, content_type, content_text, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, deleted_at \
         FROM trash ORDER BY deleted_at DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(HistoryItem {
            id: r.get(0)?,
            content_type: parse_content_type(r.get::<_, String>(1)?),
            content_text: r.get(2)?,
            preview: r.get::<_, String>(2)?,
            source_app: r.get(3)?,
            size_bytes: r.get(4)?,
            hash: r.get(5)?,
            is_pinned: r.get::<_, i64>(6)? != 0,
            is_favorite: r.get::<_, i64>(7)? != 0,
            created_at: r.get(8)?,
            deleted_at: Some(r.get(9)?),
        })
    })?;
    rows.collect()
}

/// 恢复：从 trash 移回 history（重置置顶/收藏，created_at 取当前时间以置顶最新）。
/// 不保留原 id，由 history 自增分配，避免与现有行冲突。
pub fn restore_item(conn: &mut Connection, id: i64) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let now = now_ms();
    tx.execute(
        "INSERT INTO history (content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at) \
         SELECT content_type, content_text, content_blob, source_app, size_bytes, hash, 0, 0, ? \
         FROM trash WHERE id = ?",
        params![now, id],
    )?;
    tx.execute("DELETE FROM trash WHERE id = ?", [id])?;
    tx.commit()
}

/// 彻底删除：从 trash 永久移除单条。
pub fn purge_item(conn: &mut Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM trash WHERE id = ?", [id])?;
    Ok(())
}

/// 清空回收站：删除全部。
pub fn empty_trash(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM trash", [])?;
    Ok(())
}

/// 清空全部历史：将 history 全部软删入回收站（与单条 `delete_item` 一致，可回收站恢复），
/// 随后清空 history 表。trash 的 id 由自增分配（`INSERT` 不指定 id 列），避免与 history
/// 原 id 重叠触发主键冲突。
pub fn clear_history(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let deleted_at = now_ms();
    tx.execute(
        "INSERT INTO trash (content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, deleted_at) \
         SELECT content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, ? \
         FROM history",
        params![deleted_at],
    )?;
    tx.execute("DELETE FROM history", [])?;
    tx.commit()
}

/// 切换置顶，返回切换后的状态（id 不存在返回 false）。
pub fn toggle_pin(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("UPDATE history SET is_pinned = 1 - is_pinned WHERE id = ?", [id])?;
    if n == 0 {
        return Ok(false);
    }
    conn.query_row("SELECT is_pinned FROM history WHERE id = ?", [id], |r| {
        Ok(r.get::<_, i64>(0)? != 0)
    })
}

/// 切换收藏，返回切换后的状态（id 不存在返回 false）。
pub fn toggle_favorite(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("UPDATE history SET is_favorite = 1 - is_favorite WHERE id = ?", [id])?;
    if n == 0 {
        return Ok(false);
    }
    conn.query_row("SELECT is_favorite FROM history WHERE id = ?", [id], |r| {
        Ok(r.get::<_, i64>(0)? != 0)
    })
}

/// 写入 / 覆盖设置项。
pub fn update_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// 读取全部设置项。
pub fn get_settings(conn: &Connection) -> rusqlite::Result<Vec<Setting>> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt.query_map([], |r| {
        Ok(Setting {
            key: r.get(0)?,
            value: r.get(1)?,
        })
    })?;
    rows.collect()
}

/// 读取字符串设置项；键不存在时返回 default。
pub fn get_string_setting(conn: &Connection, key: &str, default: &str) -> String {
    match conn.query_row(
        "SELECT value FROM settings WHERE key = ?",
        [key],
        |r| r.get::<_, String>(0),
    ) {
        Ok(v) => v,
        Err(_) => default.to_string(),
    }
}

/// 读取整型设置项；键不存在或值无法解析时返回 default。
pub fn get_int_setting(conn: &Connection, key: &str, default: i64) -> i64 {
    match conn.query_row(
        "SELECT value FROM settings WHERE key = ?",
        [key],
        |r| r.get::<_, String>(0),
    ) {
        Ok(v) => v.parse::<i64>().unwrap_or(default),
        Err(_) => default,
    }
}

/// 写入忽略应用：保留系统原始显示名（含中文/原始大小写），
/// 大小写不敏感去重（先删同名其它大小写形式，再插入当前显示名），避免重复条目。
pub fn insert_ignored_app(conn: &Connection, name: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM ignored_apps WHERE lower(name) = lower(?)", [name])?;
    conn.execute(
        "INSERT OR IGNORE INTO ignored_apps (name) VALUES (?)",
        [name],
    )?;
    Ok(())
}

/// 读取全部忽略应用名（系统原始显示名）。
pub fn get_ignored_apps(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM ignored_apps")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// 从忽略列表移除应用（大小写不敏感匹配）。
pub fn delete_ignored_app(conn: &Connection, name: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM ignored_apps WHERE lower(name) = lower(?)",
        [name],
    )?;
    Ok(())
}

/// 容量清理：超出上限时硬删最旧 excess 条（不进回收站）。
pub fn enforce_capacity(conn: &Connection, max: i64) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))?;
    if count > max {
        let excess = count - max;
        conn.execute(
            "DELETE FROM history WHERE id IN (SELECT id FROM history ORDER BY created_at ASC LIMIT ?)",
            [excess],
        )?;
    }
    Ok(())
}

/// 已知文件管理器来源（忽略大小写）。这些来源复制的「文本」极可能是文件名而非正文，
/// 是判断一条 `text` 条目是否实为文件复制的强信号。
const FILE_MANAGER_SOURCES: &[&str] = &[
    "访达", "finder", "资源管理器", "explorer", "文件管理器", "nautilus", "dolphin",
];

/// 扫描时跳过的目录名（体积大或无意义的系统 / 依赖目录），避免一次性迁移拖垮启动。
const MIGRATION_SKIP_DIRS: &[&str] = &[
    "node_modules", "target", ".git", "dist", "build", ".next", "out", "vendor", "Library",
    ".cache", ".cargo", ".rustup", ".npm", ".venv", "__pycache__", ".Trash", ".docker",
    ".vagrant", "Pictures", "Movies", "Music",
];

/// 一次性迁移：将早期被误判为「文本」的文件条目纠正为「文件」类型。
///
/// 早期版本 `read_clipboard` 优先取文本，而 macOS 上 Finder 复制文件时 `NSStringPboardType`
/// 仅含**文件名**（不含路径），导致文件以 `content_type='text'` + `content_text=文件名` 入库。
/// 这些条目无法一键粘贴为文件本身，且在类型筛选「文件」与托盘菜单中均表现为文本。
///
/// 处理：对 `content_type='text'` 且「来源为文件管理器」或「content_text 本就是绝对路径列表」的行，
/// 将文件名解析为真实路径（绝对路径直接验证，否则在用户主目录中按文件名查找），写入 `content_blob`
/// （JSON 路径数组）并改为 `content_type='file'`，同时回填完整路径到 `content_text`（用于列表与详情展示）。
/// 无法解析为真实文件的保持原样，绝不误伤纯文本。用设置项 `mig_file_v1` 保证仅执行一次（且无需时跳过）。
pub fn migrate_files_from_text(conn: &Connection) -> rusqlite::Result<()> {
    if get_string_setting(conn, "mig_file_v1", "") == "1" {
        return Ok(());
    }

    // 快速预检：仅当存在可能需要纠正的 text 行时才执行（昂贵的目录扫描）。
    let mut pre = conn.prepare(
        "SELECT source_app, content_text FROM history WHERE content_type = 'text'",
    )?;
    let pre_rows = pre.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut needs = false;
    for r in pre_rows {
        let (source, text) = r?;
        if FILE_MANAGER_SOURCES
            .iter()
            .any(|fm| source.eq_ignore_ascii_case(fm))
            || is_abs_path_list(&text)
        {
            needs = true;
            break;
        }
    }
    drop(pre);
    if !needs {
        let _ = update_setting(conn, "mig_file_v1", "1");
        return Ok(());
    }

    // 预构建「文件名 -> 路径」索引（带容量与深度上限，避免超大目录拖垮启动）。
    let mut name_index: std::collections::HashMap<String, std::path::PathBuf> =
        std::collections::HashMap::new();
    if let Some(home) = std::env::var_os("HOME") {
        build_name_index(&std::path::PathBuf::from(home), &mut name_index, 5, 250_000);
    }

    let mut stmt = conn.prepare(
        "SELECT id, content_text, source_app FROM history WHERE content_type = 'text'",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    })?;
    let items: Vec<(i64, String, String)> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    for (id, content_text, source_app) in items {
        let from_fm = FILE_MANAGER_SOURCES
            .iter()
            .any(|fm| source_app.eq_ignore_ascii_case(fm));
        if !from_fm && !is_abs_path_list(&content_text) {
            continue;
        }
        let tokens: Vec<&str> = content_text
            .split(", ")
            .filter(|s| !s.is_empty())
            .collect();
        if tokens.is_empty() {
            continue;
        }
        let mut resolved: Vec<String> = Vec::with_capacity(tokens.len());
        let mut ok = true;
        for tok in tokens {
            match resolve_token(tok, &name_index) {
                Some(p) => resolved.push(p),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && !resolved.is_empty() {
            let size: i64 = resolved
                .iter()
                .filter_map(|p| std::fs::metadata(p).ok())
                .map(|m| m.len() as i64)
                .sum();
            let joined = resolved.join(", ");
            if let Ok(blob) = serde_json::to_vec(&resolved) {
                conn.execute(
                    "UPDATE history SET content_type = 'file', content_text = ?, content_blob = ?, size_bytes = ? WHERE id = ?",
                    params![joined, blob, size, id],
                )?;
            }
        }
    }

    let _ = update_setting(conn, "mig_file_v1", "1");
    Ok(())
}

/// content_text 是否本身就是「绝对路径列表」（每个 token 都是已存在的绝对路径）。
fn is_abs_path_list(s: &str) -> bool {
    let tokens: Vec<&str> = s.split(", ").filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return false;
    }
    tokens
        .iter()
        .all(|t| std::path::Path::new(t).is_absolute() && std::path::Path::new(t).exists())
}

/// 将单个 token 解析为真实存在的文件路径：
/// - 已是绝对且存在的路径 → 直接规范化返回；
/// - 否则在 `name_index`（按文件名预建索引）中查找同名文件 → 返回其规范化路径；
/// - 都找不到 → None。
fn resolve_token(
    tok: &str,
    name_index: &std::collections::HashMap<String, std::path::PathBuf>,
) -> Option<String> {
    let p = std::path::Path::new(tok);
    if p.is_absolute() {
        if p.exists() {
            return std::fs::canonicalize(p)
                .ok()
                .map(|c| c.to_string_lossy().into_owned())
                .or_else(|| Some(tok.to_string()));
        }
        return None;
    }
    let fname = p.file_name()?.to_string_lossy().into_owned();
    if let Some(found) = name_index.get(&fname) {
        if found.exists() {
            return std::fs::canonicalize(found)
                .ok()
                .map(|c| c.to_string_lossy().into_owned())
                .or_else(|| Some(found.to_string_lossy().into_owned()));
        }
    }
    None
}

/// 在 `root` 下构建「文件名 -> 路径」索引（深度优先，跳过重型 / 系统 / 隐藏目录，受容量与深度限制）。
fn build_name_index(
    root: &std::path::Path,
    index: &mut std::collections::HashMap<String, std::path::PathBuf>,
    max_depth: usize,
    cap: usize,
) {
    fn walk(
        dir: &std::path::Path,
        index: &mut std::collections::HashMap<String, std::path::PathBuf>,
        depth: usize,
        max_depth: usize,
        cap: usize,
    ) {
        if depth > max_depth || index.len() >= cap {
            return;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            if index.len() >= cap {
                return;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // 跳过系统 / 依赖 / 隐藏目录，避免一次性迁移扫描过慢。
                if MIGRATION_SKIP_DIRS.contains(&name) || name.starts_with('.') {
                    continue;
                }
                walk(&path, index, depth + 1, max_depth, cap);
            } else if file_type.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    // 仅记录首次出现的同名文件，避免索引被海量同名小文件撑爆。
                    index.entry(name.to_string()).or_insert(path);
                }
            }
        }
    }
    walk(root, index, 0, max_depth, cap);
}

/// 当前毫秒时间戳。
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 从存储字符串解析 ContentType，未知回退 Text（历史数据健壮）。
fn parse_content_type(s: String) -> crate::models::ContentType {
    crate::models::ContentType::from_str(&s).unwrap_or(crate::models::ContentType::Text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        migrate(&c).unwrap();
        c
    }

    fn sample(hash: &str, created_at: i64) -> NewItem {
        NewItem {
            content_type: crate::models::ContentType::Text,
            content_text: format!("content-{hash}"),
            content_blob: None,
            source_app: "TestApp".into(),
            size_bytes: 10,
            hash: hash.into(),
            created_at,
        }
    }

    #[test]
    fn insert_then_get_reverse_chrono() {
        let c = mem_db();
        insert_or_bump(&c, &sample("a", 100)).unwrap();
        insert_or_bump(&c, &sample("b", 200)).unwrap();
        let items = get_history(&c, 100, true).unwrap();
        assert_eq!(items.len(), 2);
        // 时间倒序：后插入的 b(200) 在前面
        assert_eq!(items[0].hash, "b");
        assert_eq!(items[1].hash, "a");
    }

    #[test]
    fn delete_moves_to_trash() {
        let mut c = mem_db();
        let id = insert_or_bump(&c, &sample("a", 100)).unwrap();
        delete_item(&mut c, id).unwrap();
        assert!(get_history(&c, 100, true).unwrap().is_empty());
        let trash_count: i64 = c
            .query_row("SELECT COUNT(*) FROM trash", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trash_count, 1);
    }

    #[test]
    fn insert_or_bump_dedups_by_hash() {
        let c = mem_db();
        let id1 = insert_or_bump(&c, &sample("same", 100)).unwrap();
        let id2 = insert_or_bump(&c, &sample("same", 999)).unwrap();
        assert_eq!(id1, id2, "同 hash 应复用同一行");
        let items = get_history(&c, 100, true).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].created_at, 999, "created_at 应更新为最新");
    }

    #[test]
    fn capacity_is_enforced() {
        let c = mem_db();
        for i in 0..(MAX_HISTORY + 10) {
            insert_or_bump(&c, &sample(&format!("h{i}"), i)).unwrap();
        }
        let count: i64 = c.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap();
        assert_eq!(count, MAX_HISTORY);
    }

    #[test]
    fn toggle_pin_works() {
        let c = mem_db();
        let id = insert_or_bump(&c, &sample("a", 100)).unwrap();
        assert!(toggle_pin(&c, id).unwrap());
        let items = get_history(&c, 100, true).unwrap();
        assert!(items[0].is_pinned);
    }

    #[test]
    fn ignored_apps_persist() {
        let c = mem_db();
        insert_ignored_app(&c, "Safari").unwrap();
        let list = get_ignored_apps(&c).unwrap();
        // 保留原始显示名（不再小写化）。
        assert_eq!(list, vec!["Safari".to_string()]);
    }

    #[test]
    fn delete_ignored_app_removes_it() {
        let c = mem_db();
        insert_ignored_app(&c, "Safari").unwrap();
        insert_ignored_app(&c, "Terminal").unwrap();
        delete_ignored_app(&c, "SAFARI").unwrap();
        let list = get_ignored_apps(&c).unwrap();
        // 大小写不敏感匹配；删除后仅剩 Terminal。
        assert_eq!(list, vec!["Terminal".to_string()]);
    }

    #[test]
    fn get_trash_returns_deleted() {
        let mut c = mem_db();
        let id = insert_or_bump(&c, &sample("a", 100)).unwrap();
        delete_item(&mut c, id).unwrap();
        let trash = get_trash(&c).unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].hash, "a");
    }

    #[test]
    fn restore_moves_back_to_history() {
        let mut c = mem_db();
        let id = insert_or_bump(&c, &sample("a", 100)).unwrap();
        delete_item(&mut c, id).unwrap();
        restore_item(&mut c, id).unwrap();
        assert_eq!(get_history(&c, 100, true).unwrap().len(), 1);
        let trash_count: i64 = c
            .query_row("SELECT COUNT(*) FROM trash", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trash_count, 0);
    }

    #[test]
    fn purge_removes_from_trash() {
        let mut c = mem_db();
        let id = insert_or_bump(&c, &sample("a", 100)).unwrap();
        delete_item(&mut c, id).unwrap();
        purge_item(&mut c, id).unwrap();
        let trash_count: i64 = c
            .query_row("SELECT COUNT(*) FROM trash", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trash_count, 0);
    }

    #[test]
    fn empty_trash_clears_all() {
        let mut c = mem_db();
        let a = insert_or_bump(&c, &sample("a", 100)).unwrap();
        let b = insert_or_bump(&c, &sample("b", 200)).unwrap();
        delete_item(&mut c, a).unwrap();
        delete_item(&mut c, b).unwrap();
        empty_trash(&c).unwrap();
        let trash_count: i64 = c
            .query_row("SELECT COUNT(*) FROM trash", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trash_count, 0);
    }

    #[test]
    fn clear_history_moves_all_to_trash() {
        let mut c = mem_db();
        insert_or_bump(&c, &sample("a", 100)).unwrap();
        insert_or_bump(&c, &sample("b", 200)).unwrap();
        insert_or_bump(&c, &sample("c", 300)).unwrap();
        clear_history(&mut c).unwrap();
        // history 被清空
        let hist_count: i64 = c
            .query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(hist_count, 0);
        // 全部进入回收站（trash id 自增分配，不与原 history id 冲突）
        let trash_count: i64 = c
            .query_row("SELECT COUNT(*) FROM trash", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trash_count, 3);
        // 回收站保留原 content，可恢复
        let restored = get_trash(&c).unwrap();
        let hashes: Vec<String> = restored.iter().map(|i| i.hash.clone()).collect();
        assert!(hashes.contains(&"a".to_string()));
        assert!(hashes.contains(&"b".to_string()));
        assert!(hashes.contains(&"c".to_string()));
    }

    #[test]
    fn get_int_setting_returns_value_or_default() {
        let c = mem_db();
        // 缺省：键不存在时返回 default
        assert_eq!(get_int_setting(&c, "tray_history_count", 30), 30);
        // 写入后读取
        update_setting(&c, "tray_history_count", "12").unwrap();
        assert_eq!(get_int_setting(&c, "tray_history_count", 30), 12);
        // 无法解析时回退 default
        update_setting(&c, "tray_history_count", "not-a-number").unwrap();
        assert_eq!(get_int_setting(&c, "tray_history_count", 30), 30);
    }

    #[test]
    fn is_abs_path_list_detects_real_paths() {
        let tmp = std::env::temp_dir().join(format!("clipstack_mig_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let f = tmp.join("a.txt");
        let _ = std::fs::write(&f, b"x");
        assert!(is_abs_path_list(f.to_str().unwrap()));
        // 纯文件名（相对）不应判定为绝对路径列表
        assert!(!is_abs_path_list("just a name.txt"));
        // 空串
        assert!(!is_abs_path_list(""));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_token_finds_existing_file_by_name() {
        let tmp = std::env::temp_dir().join(format!("clipstack_mig2_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let f = tmp.join("wanted.md");
        let _ = std::fs::write(&f, b"x");
        let mut idx = std::collections::HashMap::new();
        build_name_index(&tmp, &mut idx, 3, 1000);
        let resolved = resolve_token("wanted.md", &idx);
        assert!(resolved.is_some());
        assert!(resolved.unwrap().ends_with("wanted.md"));
        // 不存在的文件名解析为 None
        assert!(resolve_token("nope.md", &idx).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn migrate_files_from_text_reclassifies_finder_copies() {
        let c = mem_db();
        // 模拟早期误判：来自 Finder 的文本条目，content_text 仅含文件名。
        c.execute(
            "INSERT INTO history (content_type, content_text, content_blob, source_app, size_bytes, hash, created_at) \
             VALUES ('text', 'demo.md', NULL, '访达', 0, 'h1', 100)",
            [],
        )
        .unwrap();
        // 一条纯文本（非文件管理器来源）应保持原样。
        c.execute(
            "INSERT INTO history (content_type, content_text, content_blob, source_app, size_bytes, hash, created_at) \
             VALUES ('text', 'hello world', NULL, 'TextEdit', 0, 'h2', 200)",
            [],
        )
        .unwrap();

        // 在临时目录放一个同名文件，并把 HOME 临时指向该目录，供迁移按文件名解析。
        let tmp = std::env::temp_dir().join(format!("clipstack_mig3_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::write(tmp.join("demo.md"), b"hello");
        // 重定向 HOME 指向 tmp（含 demo.md），使 Finder 来源的文件名可被解析为真实路径。
        let _ = std::env::set_var("HOME", &tmp);
        migrate_files_from_text(&c).unwrap();
        let _ = std::env::remove_var("HOME");

        let mut types = c
            .prepare("SELECT content_type, content_text FROM history ORDER BY id")
            .unwrap();
        let rows: Vec<(String, String)> = types
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        // Finder 来源的文件名被解析为真实路径 → 纠正为 file 类型。
        assert_eq!(rows[0].0, "file");
        assert!(rows[0].1.ends_with("demo.md"));
        // 纯文本（非文件管理器来源）保持 text，绝不误伤。
        assert_eq!(rows[1].0, "text");
        // 迁移标记已写入
        assert_eq!(get_string_setting(&c, "mig_file_v1", ""), "1");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
