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
}
