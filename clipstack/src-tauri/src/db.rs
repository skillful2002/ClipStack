// P2 · 持久化层（SQLite via rusqlite，bundled 编译内嵌 SQLite，免系统依赖）
//
// 设计：
//   - `AppDb` 封装 `Mutex<Connection>`，可被 Tauri State 托管（Send + Sync）。
//   - 所有 CRUD 都以 `&Connection` 自由函数实现，便于在单测中用内存库直接验证。
//   - 表结构见 `migrate`；去重（hash）与容量上限（`enforce_capacity`）在此层保证。

use std::sync::{Mutex, MutexGuard};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rusqlite::{params, Connection};
use tauri::{AppHandle, Manager};

use crate::clipboard::is_sensitive;
use crate::crypto::{self, Key};
use crate::models::{ContentType, HistoryItem, NewItem, Setting};

/// 敏感内容被掩码时展示的占位符（与托盘脱敏占位一致）。
pub const SENSITIVE_MASK: &str = "••••••";

/// 读取时按当前 `mask_sensitive` 设置实时计算敏感掩码。
///
/// 关键设计：掩码判定在「读取时」而非「写入时」进行，使「掩码敏感内容」开关切换对
/// **所有（含历史）条目立即生效**，无需等待重新捕获或迁移旧数据。原文仍解密返回
/// （复制不受影响），仅预览被遮挡。
/// 返回 `(原文, 预览, 是否掩码)`。
fn mask_sensitive_read(
    key: Option<&Key>,
    content_type: ContentType,
    raw: &str,
    mask_on: bool,
) -> (String, String, bool) {
    let plain = open_text(key, raw);
    // 读取时实时重算「是否敏感」：不依赖捕获时落库的旧值，历史条目也能随规则更新即时修正。
    let detected = matches!(
        content_type,
        ContentType::Text | ContentType::Link | ContentType::Code
    ) && is_sensitive(&plain);
    // 掩码仅在开关开启且命中敏感时生效；detection 本身与开关无关，直接作为 is_sensitive 返回。
    let preview = if mask_on && detected {
        SENSITIVE_MASK.to_string()
    } else {
        plain.clone()
    };
    (plain, preview, detected)
}

/// 历史条目容量上限：超出后自动硬删最旧部分（不进回收站，避免回收站无限增长）。
pub const MAX_HISTORY: i64 = 5000;
/// `get_history` 默认读取上限。
pub const DEFAULT_LIMIT: i64 = 500;

/// 受 Tauri 托管的数据库连接。
pub struct AppDb {
    pub conn: Mutex<Connection>,
    /// 主密钥（仅解锁后在内存中持有；锁定时清空）。`None` 表示未解锁 / 尚未设置主密码
    /// （此时落库为明文，兼容「尚未启用安全」与「锁定态跳过捕获」）。
    pub key: Mutex<Option<Key>>,
}

/// Tauri State 类型（Arc 便于在监控线程间共享）。
pub type DbState = std::sync::Arc<AppDb>;

impl AppDb {
    /// 锁定并返回连接（封装，避免调用处重复 map_err）。
    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().expect("db lock poisoned")
    }
}

// ===== P0 内容加解密辅助（仅 content_text / content_blob 两列）=====
//
// `key = None` 时透传明文（兼容尚未启用安全 / 迁移前数据），使单测与旧数据路径零改动。

/// 明文字符串 → 密文（base64 存 TEXT 列）。
fn seal_text(key: Option<&Key>, s: &str) -> String {
    match key {
        Some(k) => STANDARD.encode(crypto::encrypt(k, s.as_bytes())),
        None => s.to_string(),
    }
}

/// 明文二进制 → 密文（原始字节存 BLOB 列）。
fn seal_blob(key: Option<&Key>, b: Option<&[u8]>) -> Option<Vec<u8>> {
    match (key, b) {
        (Some(k), Some(b)) => Some(crypto::encrypt(k, b)),
        _ => b.map(|b| b.to_vec()),
    }
}

/// 密文（base64 TEXT 列）→ 明文；非 base64 / 解密失败（明文遗留）则原样返回。
fn open_text(key: Option<&Key>, s: &str) -> String {
    match key {
        Some(k) => match STANDARD.decode(s) {
            Ok(ct) => crypto::decrypt(k, &ct)
                .map(|p| String::from_utf8_lossy(&p).into_owned())
                .unwrap_or_else(|| s.to_string()),
            Err(_) => s.to_string(),
        },
        None => s.to_string(),
    }
}

/// 密文（BLOB 列）→ 明文二进制；非密文遗留则原样返回。
fn open_blob(key: Option<&Key>, b: Option<&[u8]>) -> Option<Vec<u8>> {
    match (key, b) {
        (Some(k), Some(b)) => crypto::decrypt(k, b),
        _ => b.map(|b| b.to_vec()),
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
    migrate_lan(&conn)?;
    // 一次性迁移：纠正早期被误判为「文本」的文件条目（见函数注释）。
    let _ = migrate_files_from_text(&conn);
    Ok(AppDb {
        conn: Mutex::new(conn),
        key: Mutex::new(None),
    })
}

/// 若表 `t` 尚不存在列 `col`，则追加（幂等，兼容已部署旧库升级）。
/// SQLite 不支持 `ADD COLUMN IF NOT EXISTS`，这里先查 `pragma_table_info` 再决定。
fn add_column_if_missing(conn: &Connection, t: &str, col: &str, def: &str) -> rusqlite::Result<()> {
    let exists: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{t}') WHERE name = '{col}'"),
        [],
        |r| r.get(0),
    )?;
    if exists == 0 {
        conn.execute(
            &format!("ALTER TABLE {t} ADD COLUMN {col} {def}"),
            [],
        )?;
    }
    Ok(())
}

/// 建表 + 索引（幂等）。
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    // P2 · 安全删除：DELETE 时立即用零覆写被删页（防止内容残留在空闲页）。
    conn.execute_batch("PRAGMA secure_delete = ON;")?;

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
            is_sensitive INTEGER NOT NULL DEFAULT 0,
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
            is_sensitive INTEGER NOT NULL DEFAULT 0,
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
    )?;

    // P1-验收 · 兼容已部署旧库：旧表可能不含 `is_sensitive` 列，这里幂等补列。
    // 全新库因上方 CREATE TABLE 已含该列，此处为 no-op。
    add_column_if_missing(conn, "history", "is_sensitive", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(conn, "trash", "is_sensitive", "INTEGER NOT NULL DEFAULT 0")?;
    // L4 · 来源标识列：本地捕获为空、共享条目填对端设备名。幂等补列，使 `get_history`
    // 等统一 SELECT 在旧库（仅 migrate）与新库均可用。
    add_column_if_missing(conn, "history", "origin_device", "TEXT NOT NULL DEFAULT ''")?;
    add_column_if_missing(conn, "history", "is_remote", "INTEGER NOT NULL DEFAULT 0")?;
    // 回收站同样返回 HistoryItem（含来源标识），故 trash 也补这两列（共享条目一般不进回收站，恒为本地）。
    add_column_if_missing(conn, "trash", "origin_device", "TEXT NOT NULL DEFAULT ''")?;
    add_column_if_missing(conn, "trash", "is_remote", "INTEGER NOT NULL DEFAULT 0")?;
    Ok(())
}

/// L3 · 局域网共享：新建 `sync_profiles` 表与 `history` 增量列（幂等）。
///
/// 设计文档假设该表已存在并写「ALTER TABLE sync_profiles ADD ...」；
/// 实际代码库此前并无此表，故此处用 `CREATE TABLE IF NOT EXISTS` 负责创建，
/// 其余 `history` 增量列沿用 `add_column_if_missing` 兼容旧库升级。
pub fn migrate_lan(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS sync_profiles (
            id          TEXT PRIMARY KEY,
            mode        TEXT NOT NULL DEFAULT 'lan',   -- 'lan' | 'relay'
            share_group TEXT,
            wrapped_key TEXT,                           -- 经钥匙串包装的 share_key，不裸存
            is_active   INTEGER NOT NULL DEFAULT 0,     -- 多配置单激活，互斥
            created_at  INTEGER NOT NULL
        );
        "#,
    )?;
    add_column_if_missing(conn, "history", "sync_id", "TEXT")?;
    add_column_if_missing(conn, "history", "origin_device", "TEXT")?;
    add_column_if_missing(conn, "history", "lamport", "INTEGER")?;
    add_column_if_missing(conn, "history", "profile_id", "TEXT")?;
    add_column_if_missing(conn, "history", "is_remote", "INTEGER")?; // 1=来自共享，不触发回环
    Ok(())
}

/// L3 · `sync_profiles` 行视图（不含敏感性字段 `wrapped_key`）。
#[derive(Debug, Clone)]
pub struct SyncProfile {
    pub id: String,
    pub mode: String,
    pub share_group: String,
    pub is_active: bool,
    #[allow(dead_code)]
    pub created_at: i64,
}

/// L3 · 列出全部共享配置（按创建时间升序）。
pub fn list_sync_profiles(conn: &Connection) -> rusqlite::Result<Vec<SyncProfile>> {
    let mut stmt = conn.prepare(
        "SELECT id, mode, share_group, is_active, created_at FROM sync_profiles ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SyncProfile {
            id: r.get(0)?,
            mode: r.get(1)?,
            share_group: r.get(2)?,
            is_active: r.get::<_, i64>(3)? != 0,
            created_at: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// L3 · 新建或更新共享配置（按 id 幂等）。
pub fn upsert_sync_profile(
    conn: &Connection,
    id: &str,
    mode: &str,
    share_group: &str,
    wrapped_key: &str,
    is_active: bool,
    created_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sync_profiles (id, mode, share_group, wrapped_key, is_active, created_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
            mode=excluded.mode, share_group=excluded.share_group, \
            wrapped_key=excluded.wrapped_key, is_active=excluded.is_active",
        params![id, mode, share_group, wrapped_key, is_active as i64, created_at],
    )?;
    Ok(())
}

/// L3 · 激活某配置：先全部置 0，再置该 id 为 1（多配置单激活，互斥）。
pub fn set_active_profile(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE sync_profiles SET is_active = 0", [])?;
    conn.execute("UPDATE sync_profiles SET is_active = 1 WHERE id = ?", [id])?;
    Ok(())
}

/// L3 · 删除共享配置。
pub fn delete_sync_profile(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM sync_profiles WHERE id = ?", [id])?;
    Ok(())
}

/// L3 · 当前激活的共享配置（如有）。
#[allow(dead_code)]
pub fn get_active_profile(conn: &Connection) -> rusqlite::Result<Option<SyncProfile>> {
    let mut stmt = conn.prepare(
        "SELECT id, mode, share_group, is_active, created_at FROM sync_profiles WHERE is_active = 1 LIMIT 1",
    )?;
    let mut rows = stmt.query_map([], |r| {
        Ok(SyncProfile {
            id: r.get(0)?,
            mode: r.get(1)?,
            share_group: r.get(2)?,
            is_active: r.get::<_, i64>(3)? != 0,
            created_at: r.get(4)?,
        })
    })?;
    rows.next().transpose()
}

/// L3 · 读取某配置的（wrapped_key, share_group），用于激活时还原明文密钥。
pub fn get_profile_creds(conn: &Connection, id: &str) -> rusqlite::Result<(String, String)> {
    conn.query_row(
        "SELECT wrapped_key, share_group FROM sync_profiles WHERE id = ?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// L3 · 某配置是否当前激活。
pub fn is_profile_active(conn: &Connection, id: &str) -> bool {
    conn.query_row(
        "SELECT is_active FROM sync_profiles WHERE id = ?",
        [id],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
        != 0
}

/// L3 · 写入来自共享的对端条目（`is_remote=1`）所需的全部入参。
/// 抽为结构体以满足 `clippy::too_many_arguments`，同时让调用点更可读。
pub struct RemoteClipInput<'a> {
    pub key: Option<&'a Key>,
    pub content_type: &'a str,
    pub content_text: &'a str,
    pub content_blob: Option<&'a [u8]>,
    pub source_app: &'a str,
    pub size_bytes: i64,
    pub hash: &'a str,
    pub is_sensitive: bool,
    pub origin_device: &'a str,
    pub sync_id: &'a str,
    pub lamport: i64,
    pub profile_id: &'a str,
}

/// L3 · 写入来自共享的对端条目（`is_remote=1`）。按 `sync_id` 去重，已存在则跳过。
/// 返回新插入行 id（跳过时为 `None`）。内容沿用内部密钥加密，与本地捕获一致。
pub fn insert_remote_clip(conn: &Connection, input: RemoteClipInput<'_>) -> rusqlite::Result<Option<i64>> {
    if conn
        .query_row(
            "SELECT id FROM history WHERE sync_id = ?",
            [input.sync_id],
            |r| r.get::<_, i64>(0),
        )
        .is_ok()
    {
        return Ok(None); // 已存在，跳过（去重）
    }
    let sealed_text = seal_text(input.key, input.content_text);
    let sealed_blob = seal_blob(input.key, input.content_blob);
    conn.execute(
        "INSERT INTO history \
         (content_type, content_text, content_blob, source_app, size_bytes, hash, \
          is_pinned, is_favorite, is_sensitive, created_at, sync_id, origin_device, lamport, profile_id, is_remote) \
         VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?, ?, ?, ?, 1)",
        params![
            input.content_type,
            sealed_text,
            sealed_blob,
            input.source_app,
            input.size_bytes,
            input.hash,
            input.is_sensitive as i64,
            now_ms(),
            input.sync_id,
            input.origin_device,
            input.lamport,
            input.profile_id,
        ],
    )?;
    let _ = enforce_capacity(conn, MAX_HISTORY);
    Ok(Some(conn.last_insert_rowid()))
}

/// 新增或去重置顶：同 hash 已在 history 中 → 更新 created_at 等并置顶返回原 id；
/// 否则插入新行。插入后执行容量上限清理。返回行 id。
pub fn insert_or_bump(
    conn: &Connection,
    key: Option<&Key>,
    item: &NewItem,
) -> rusqlite::Result<i64> {
    let ct = seal_text(key, &item.content_text);
    let cb = seal_blob(key, item.content_blob.as_deref());
    if let Ok(existing) =
        conn.query_row("SELECT id FROM history WHERE hash = ?", [item.hash.as_str()], |r| {
            r.get::<_, i64>(0)
        })
    {
        conn.execute(
            "UPDATE history SET created_at = ?, content_text = ?, content_blob = ?, size_bytes = ?, source_app = ? WHERE id = ?",
            params![
                item.created_at,
                ct,
                cb,
                item.size_bytes,
                item.source_app,
                existing
            ],
        )?;
        let _ = enforce_capacity(conn, MAX_HISTORY);
        return Ok(existing);
    }

    conn.execute(
        "INSERT INTO history (content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, is_sensitive, created_at)
         VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?)",
        params![
            item.content_type.as_str(),
            ct,
            cb,
            item.source_app,
            item.size_bytes,
            item.hash,
            item.is_sensitive as i64,
            item.created_at
        ],
    )?;
    let id = conn.last_insert_rowid();
    let _ = enforce_capacity(conn, MAX_HISTORY);
    Ok(id)
}

/// 读取历史：默认按「置顶优先、再按时间倒序」，限制条数。
pub fn get_history(
    conn: &Connection,
    key: Option<&Key>,
    limit: i64,
    pin_first: bool,
) -> rusqlite::Result<Vec<HistoryItem>> {
    let order = if pin_first {
        "is_pinned DESC, created_at DESC"
    } else {
        "created_at DESC"
    };
    let sql = format!(
        "SELECT id, content_type, content_text, source_app, size_bytes, hash, is_pinned, is_favorite, is_sensitive, created_at, origin_device, is_remote \
         FROM history ORDER BY {order} LIMIT ?"
    );
    // 读取时实时读取「掩码敏感内容」开关，使开关切换对所有（含历史）条目立即生效。
    let mask_on = get_string_setting(conn, "mask_sensitive", "0") != "0";
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit], |r| {
        let raw: String = r.get(2)?;
        let ct = parse_content_type(r.get::<_, String>(1)?);
        let (plain, preview, sensitive) = mask_sensitive_read(key, ct, &raw, mask_on);
        Ok(HistoryItem {
            id: r.get(0)?,
            content_type: ct,
            content_text: plain,
            preview,
            source_app: r.get(3)?,
            size_bytes: r.get(4)?,
            hash: r.get(5)?,
            is_pinned: r.get::<_, i64>(6)? != 0,
            is_favorite: r.get::<_, i64>(7)? != 0,
            is_sensitive: sensitive,
            created_at: r.get(9)?,
            origin_device: r.get(10)?,
            is_remote: r.get::<_, i64>(11)? != 0,
            deleted_at: None,
        })
    })?;
    rows.collect()
}

/// 托盘菜单专用：读取最近若干条，但排除「文件」类型（文件复制在文件管理器中粘贴更直观，
/// 不适合在托盘里以文本/图标形式展示复制）。
pub fn get_recent_tray(
    conn: &Connection,
    key: Option<&Key>,
    limit: i64,
) -> rusqlite::Result<Vec<HistoryItem>> {
    let mut stmt = conn.prepare(
        "SELECT id, content_type, content_text, source_app, size_bytes, hash, is_pinned, is_favorite, is_sensitive, created_at, origin_device, is_remote \
         FROM history WHERE content_type != 'file' ORDER BY is_pinned DESC, created_at DESC LIMIT ?",
    )?;
    // 读取时实时读取「掩码敏感内容」开关，使开关切换对所有（含历史）条目立即生效。
    let mask_on = get_string_setting(conn, "mask_sensitive", "0") != "0";
    let rows = stmt.query_map([limit], |r| {
        let raw: String = r.get(2)?;
        let ct = parse_content_type(r.get::<_, String>(1)?);
        let (plain, preview, sensitive) = mask_sensitive_read(key, ct, &raw, mask_on);
        Ok(HistoryItem {
            id: r.get(0)?,
            content_type: ct,
            content_text: plain,
            preview,
            source_app: r.get(3)?,
            size_bytes: r.get(4)?,
            hash: r.get(5)?,
            is_pinned: r.get::<_, i64>(6)? != 0,
            is_favorite: r.get::<_, i64>(7)? != 0,
            is_sensitive: sensitive,
            created_at: r.get(9)?,
            origin_device: r.get(10)?,
            is_remote: r.get::<_, i64>(11)? != 0,
            deleted_at: None,
        })
    })?;
    rows.collect()
}

/// 按 id 读取单条历史（托盘点击复制、命令读取原文等场景）。
pub fn get_item(conn: &Connection, key: Option<&Key>, id: i64) -> rusqlite::Result<HistoryItem> {
    let mut stmt = conn.prepare(
        "SELECT id, content_type, content_text, source_app, size_bytes, hash, is_pinned, is_favorite, is_sensitive, created_at, origin_device, is_remote \
         FROM history WHERE id = ?",
    )?;
    // 读取时实时读取「掩码敏感内容」开关，使开关切换对所有（含历史）条目立即生效。
    let mask_on = get_string_setting(conn, "mask_sensitive", "0") != "0";
    stmt.query_row([id], |r| {
        let raw: String = r.get(2)?;
        let ct = parse_content_type(r.get::<_, String>(1)?);
        let (plain, preview, sensitive) = mask_sensitive_read(key, ct, &raw, mask_on);
        Ok(HistoryItem {
            id: r.get(0)?,
            content_type: ct,
            content_text: plain,
            preview,
            source_app: r.get(3)?,
            size_bytes: r.get(4)?,
            hash: r.get(5)?,
            is_pinned: r.get::<_, i64>(6)? != 0,
            is_favorite: r.get::<_, i64>(7)? != 0,
            is_sensitive: sensitive,
            created_at: r.get(9)?,
            origin_device: r.get(10)?,
            is_remote: r.get::<_, i64>(11)? != 0,
            deleted_at: None,
        })
    })
}

/// 删除：从 history 移到 trash（保留完整快照 + deleted_at）。
/// 注意：trash 的 id 由自身自增分配，**不**沿用 history.id。否则 history 与 trash 共用
/// 同一 id 命名空间，而 `restore_item` 恢复时给 history 重新分配自增 id，该 id 可能与
/// trash 中已存在的行冲突；后续再删这条 history 行插入 trash 即触发
/// `UNIQUE constraint failed: trash.id`，整笔事务回滚（表现为「清除失败」）。
pub fn delete_item(conn: &mut Connection, id: i64) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let deleted_at = now_ms();
    tx.execute(
        "INSERT INTO trash (content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, deleted_at, is_sensitive) \
         SELECT content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, ?, is_sensitive \
         FROM history WHERE id = ?",
        params![deleted_at, id],
    )?;
    tx.execute("DELETE FROM history WHERE id = ?", [id])?;
    tx.commit()
}

/// 批量删除：将指定 id 的 history 条目软删入回收站（与单条 `delete_item` 完全一致，可回收站恢复）。
/// 用于「按当前查询条件清除」——前端把 `filterItems` 命中的 id 列表传入，由后端精确删除这些行。
/// 空列表直接返回 0，避免无意义的写事务。
pub fn delete_items(conn: &mut Connection, ids: &[i64]) -> rusqlite::Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let deleted_at = now_ms();
    let tx = conn.transaction()?;
    for &id in ids {
        tx.execute(
            "INSERT INTO trash (content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, deleted_at, is_sensitive) \
             SELECT content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, ?, is_sensitive \
             FROM history WHERE id = ?",
            params![deleted_at, id],
        )?;
        tx.execute("DELETE FROM history WHERE id = ?", [id])?;
    }
    tx.commit()?;
    Ok(ids.len())
}
pub fn get_trash(conn: &Connection, key: Option<&Key>) -> rusqlite::Result<Vec<HistoryItem>> {
    let mut stmt = conn.prepare(
        "SELECT id, content_type, content_text, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, deleted_at, is_sensitive, origin_device, is_remote \
         FROM trash ORDER BY deleted_at DESC",
    )?;
    // 读取时实时读取「掩码敏感内容」开关，使开关切换对所有（含历史）条目立即生效。
    let mask_on = get_string_setting(conn, "mask_sensitive", "0") != "0";
    let rows = stmt.query_map([], |r| {
        let raw: String = r.get(2)?;
        let ct = parse_content_type(r.get::<_, String>(1)?);
        let (plain, preview, sensitive) = mask_sensitive_read(key, ct, &raw, mask_on);
        Ok(HistoryItem {
            id: r.get(0)?,
            content_type: ct,
            content_text: plain,
            preview,
            source_app: r.get(3)?,
            size_bytes: r.get(4)?,
            hash: r.get(5)?,
            is_pinned: r.get::<_, i64>(6)? != 0,
            is_favorite: r.get::<_, i64>(7)? != 0,
            is_sensitive: sensitive,
            created_at: r.get(8)?,
            origin_device: r.get(11)?,
            is_remote: r.get::<_, i64>(12)? != 0,
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
        "INSERT INTO history (content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, is_sensitive) \
         SELECT content_type, content_text, content_blob, source_app, size_bytes, hash, 0, 0, ?, is_sensitive \
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
    // P2 · 安全删除：回收站内容已零覆写，VACUUM 回收空闲页。
    let _ = conn.execute("VACUUM", []);
    Ok(())
}

/// 清空全部历史：将 history 全部软删入回收站（与单条 `delete_item` 一致，可回收站恢复），
/// 随后清空 history 表。trash 的 id 由自增分配（`INSERT` 不指定 id 列），避免与 history
/// 原 id 重叠触发主键冲突。
pub fn clear_history(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let deleted_at = now_ms();
    tx.execute(
        "INSERT INTO trash (content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, deleted_at, is_sensitive) \
         SELECT content_type, content_text, content_blob, source_app, size_bytes, hash, is_pinned, is_favorite, created_at, ?, is_sensitive \
         FROM history",
        params![deleted_at],
    )?;
    tx.execute("DELETE FROM history", [])?;
    tx.commit()?;
    // P2 · 安全删除：清空历史后 VACUUM 回收空闲页（DELETE 已零覆写）。
    let _ = conn.execute("VACUUM", []);
    Ok(())
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

/// 读取单条条目的（已解密）二进制与文本：供 `copy_image` / `copy_file` / `get_item_blob` 使用。
/// 同时取 content_blob 与 content_text，按当前 key 解密；调用方无需分别加锁（本函数内统一加锁）。
pub fn read_item_raw(db: &AppDb, id: i64, table: &str) -> Option<(Option<Vec<u8>>, String)> {
    let conn = db.conn.lock().expect("db lock poisoned");
    let key = db.key.lock().expect("key lock poisoned");
    let row: (Option<Vec<u8>>, String) = conn
        .query_row(
            &format!("SELECT content_blob, content_text FROM {table} WHERE id = ?"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let text = open_text(key.as_ref(), &row.1);
    let blob = open_blob(key.as_ref(), row.0.as_deref());
    Some((blob, text))
}

/// 主密码是否已设置（`pw_verifier` 是否存在）。
pub fn has_master_password(conn: &Connection) -> bool {
    get_string_setting(conn, "pw_verifier", "") != ""
}

/// P1a · 读取「保存历史记录类型」开关：文本 / 图片 / 文件三类，默认均启用。
/// 禁用类型在捕获阶段跳过，不写入历史（决策：不提供「一键清理已禁用类型历史」按钮，
/// 仅控制后续是否继续捕获）。
pub fn save_type_enabled(conn: &Connection, ct: ContentType) -> bool {
    let key = match ct {
        ContentType::Text | ContentType::Link | ContentType::Code => "save_text",
        ContentType::Image => "save_image",
        ContentType::File => "save_file",
    };
    get_string_setting(conn, key, "1") != "0"
}

/// 写入主密码派生所需的盐与校验哈希（首次设置主密码时调用）。
pub fn set_master_password(conn: &Connection, salt_hex: &str, verifier: &str) -> rusqlite::Result<()> {
    update_setting(conn, "pw_salt", salt_hex)?;
    update_setting(conn, "pw_verifier", verifier)?;
    Ok(())
}

/// 清除主密码：移除校验盐/哈希，重置加密迁移标志（允许未来重设主密码时重新迁移明文），
/// 并关闭 Touch ID 设置（Touch ID 是主密码的增强，无主密码即无意义）。
pub fn clear_master_password(conn: &Connection) -> rusqlite::Result<()> {
    update_setting(conn, "pw_salt", "")?;
    update_setting(conn, "pw_verifier", "")?;
    update_setting(conn, "mig_enc_v1", "")?;
    update_setting(conn, "use_touch_id", "0")?;
    Ok(())
}

/// 读取主密码校验哈希。
pub fn get_pw_verifier(conn: &Connection) -> String {
    get_string_setting(conn, "pw_verifier", "")
}

/// 迁移：将存量明文 `content_text` / `content_blob` 加密（一次性，由 `mig_enc_v1` 标志保证）。
/// 在启动阶段调用——此时内部数据库密钥已载入内存，全库明文直接全部加密；
/// 之后写入由 `insert_or_bump` 加密。已加密行（可成功解密为不同内容）会被跳过，
/// 避免对既有密文再次加密造成双重加密。
pub fn migrate_plaintext_to_encrypted(conn: &Connection, key: &Key) -> rusqlite::Result<usize> {
    if get_string_setting(conn, "mig_enc_v1", "") == "1" {
        return Ok(0);
    }
    let mut n = 0;
    for table in ["history", "trash"] {
        let mut stmt = conn.prepare(&format!("SELECT id, content_text, content_blob FROM {table}"))?;
        let rows: Vec<(i64, String, Option<Vec<u8>>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (id, text, blob) in rows {
            // 已加密行（用内部密钥可成功解密为不同内容）跳过，避免双重加密。
            if open_text(Some(key), &text) != text {
                continue;
            }
            let enc_text = STANDARD.encode(crypto::encrypt(key, text.as_bytes()));
            let enc_blob = blob.map(|b| crypto::encrypt(key, &b));
            conn.execute(
                &format!("UPDATE {table} SET content_text = ?, content_blob = ? WHERE id = ?"),
                params![enc_text, enc_blob, id],
            )?;
            n += 1;
        }
    }
    update_setting(conn, "mig_enc_v1", "1")?;
    Ok(n)
}

/// 主密码变更时重新加密（历史函数，当前主密码已不再影响数据库加密，仅保留供测试/兼容）。
#[allow(dead_code)]
pub fn reencrypt_all(conn: &Connection, old: &Key, new: &Key) -> rusqlite::Result<usize> {
    let mut n = 0;
    for table in ["history", "trash"] {
        let mut stmt = conn.prepare(&format!("SELECT id, content_text, content_blob FROM {table}"))?;
        let rows: Vec<(i64, String, Option<Vec<u8>>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (id, text, blob) in rows {
            // 解密（旧密钥）；本身为明文遗留（未迁移）则直接当明文处理。
            let plain = match STANDARD.decode(&text) {
                Ok(ct) => crypto::decrypt(old, &ct).unwrap_or_else(|| text.as_bytes().to_vec()),
                Err(_) => text.into_bytes(),
            };
            let new_enc = STANDARD.encode(crypto::encrypt(new, &plain));
            let new_blob = blob
                .and_then(|b| crypto::decrypt(old, &b))
                .map(|p| crypto::encrypt(new, &p));
            conn.execute(
                &format!("UPDATE {table} SET content_text = ?, content_blob = ? WHERE id = ?"),
                params![new_enc, new_blob, id],
            )?;
            n += 1;
        }
    }
    Ok(n)
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

/// P1b · 清理超期数据（含 trash）：
/// - history：未置顶且 `created_at` 超期（按「留存天数」从创建时刻计）；
/// - trash：按 `deleted_at` 超期（按「在回收站停留时长」计，决策 2）。
/// `days <= 0` 视为永久保留，直接返回 0 不删除。
pub fn purge_expired(conn: &Connection, days: i64) -> rusqlite::Result<usize> {
    if days <= 0 {
        return Ok(0);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let cutoff = now - days * 86_400_000;
    let n_hist = conn.execute(
        "DELETE FROM history WHERE is_pinned = 0 AND created_at < ?",
        [cutoff],
    )?;
    // trash 无置顶概念，按「回收站停留时长」(deleted_at) 过期。
    let n_trash = conn.execute("DELETE FROM trash WHERE deleted_at < ?", [cutoff])?;
    // P2 · 安全删除：secure_delete=ON 已在 migrate 开启，DELETE 已零覆写；
    // 此处 VACUUM 回收空闲页，避免超期内容残留在数据库文件空闲区。
    let _ = conn.execute("VACUUM", []);
    Ok((n_hist + n_trash) as usize)
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
            is_sensitive: false,
        }
    }

    #[test]
    fn insert_then_get_reverse_chrono() {
        let c = mem_db();
        insert_or_bump(&c, None, &sample("a", 100)).unwrap();
        insert_or_bump(&c, None, &sample("b", 200)).unwrap();
        let items = get_history(&c, None, 100, true).unwrap();
        assert_eq!(items.len(), 2);
        // 时间倒序：后插入的 b(200) 在前面
        assert_eq!(items[0].hash, "b");
        assert_eq!(items[1].hash, "a");
    }

    /// P1-验收 · 旧库升级：模拟「不含 `is_sensitive` 列的已部署库」，migrate 应能幂等补列，
    /// 且原数据可正常读写、is_sensitive 默认为 0。
    #[test]
    fn migrate_adds_is_sensitive_to_legacy_db() {
        // 用旧结构（无 is_sensitive）建表，插入一条数据。
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE history (id INTEGER PRIMARY KEY, content_type TEXT NOT NULL, \
             content_text TEXT NOT NULL DEFAULT '', content_blob BLOB, source_app TEXT NOT NULL DEFAULT '', \
             size_bytes INTEGER NOT NULL DEFAULT 0, hash TEXT NOT NULL, is_pinned INTEGER NOT NULL DEFAULT 0, \
             is_favorite INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);",
        )
        .unwrap();
        c.execute(
            "INSERT INTO history (content_type, content_text, source_app, size_bytes, hash, created_at) \
             VALUES ('text','legacy','App',5,'h1',1000)",
            [],
        )
        .unwrap();

        // 跑迁移（应补列且不破坏原数据）。
        migrate(&c).unwrap();

        // 列已存在。
        let has_col: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('history') WHERE name = 'is_sensitive'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_col, 1);

        // 原数据可正常读出，is_sensitive 默认 false。
        let items = get_history(&c, None, 10, false).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].hash, "h1");
        assert!(!items[0].is_sensitive);

        // 再次 migrate 幂等（不报错）。
        migrate(&c).unwrap();
    }

    #[test]
    fn get_recent_tray_excludes_file_type() {
        let c = mem_db();
        insert_or_bump(&c, None, &sample("text-a", 100)).unwrap();
        // 直接插入一条「文件」类型，模拟 Finder 复制文件被捕获的场景。
        c.execute(
            "INSERT INTO history (content_type, content_text, content_blob, source_app, size_bytes, hash, created_at) \
             VALUES ('file', '/tmp/clipstack-packaging.md', NULL, 'Finder', 10, 'file-1', 200)",
            [],
        )
        .unwrap();
        let tray = get_recent_tray(&c, None, 100).unwrap();
        // 托盘菜单应排除文件类型，仅保留文本条目。
        assert_eq!(tray.len(), 1);
        assert_eq!(tray[0].hash, "text-a");
        assert_eq!(tray[0].content_type, crate::models::ContentType::Text);
        // 反向校验：get_history 仍包含全部（主界面照常展示文件）。
        assert_eq!(get_history(&c, None, 100, true).unwrap().len(), 2);
    }

    #[test]
    fn delete_moves_to_trash() {
        let mut c = mem_db();
        let id = insert_or_bump(&c, None, &sample("a", 100)).unwrap();
        delete_item(&mut c, id).unwrap();
        assert!(get_history(&c, None, 100, true).unwrap().is_empty());
        let trash_count: i64 = c
            .query_row("SELECT COUNT(*) FROM trash", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trash_count, 1);
    }

    #[test]
    fn delete_items_only_targeted_rows() {
        let mut c = mem_db();
        let keep = insert_or_bump(&c, None, &sample("keep", 100)).unwrap();
        let a = insert_or_bump(&c, None, &sample("a", 200)).unwrap();
        let b = insert_or_bump(&c, None, &sample("b", 300)).unwrap();
        // 仅删除 a、b，保留 keep。
        let n = delete_items(&mut c, &[a, b]).unwrap();
        assert_eq!(n, 2);
        let items = get_history(&c, None, 100, true).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, keep);
        let trash_count: i64 = c
            .query_row("SELECT COUNT(*) FROM trash", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trash_count, 2);
    }

    #[test]
    fn delete_items_empty_is_noop() {
        let mut c = mem_db();
        insert_or_bump(&c, None, &sample("a", 100)).unwrap();
        let n = delete_items(&mut c, &[]).unwrap();
        assert_eq!(n, 0);
        assert_eq!(get_history(&c, None, 100, true).unwrap().len(), 1);
    }

    /// 回归：删 -> 恢复 -> 再删 不得触发 `UNIQUE constraint failed: trash.id`。
    /// 旧实现把 history.id 原样拷入 trash.id，而 restore_item 给 history 重分配自增 id，
    /// 该 id 可能等于 trash 中仍存在的某行 id；后续再删这条 history 行插入 trash 即冲突。
    /// 修复后 trash 始终自增分配 id，与 history 解耦，故不再冲突。
    #[test]
    fn delete_restore_delete_no_trash_id_collision() {
        let mut c = mem_db();
        // 顺序插入得到 history id 1..4。
        for i in 0..4 {
            insert_or_bump(&c, None, &sample(&format!("h{i}"), i)).unwrap();
        }
        // 删除 id=4、id=3：trash 拥有 2 行（旧实现 id 分别为 4、3；新实现为自增）。
        delete_item(&mut c, 4).unwrap();
        delete_item(&mut c, 3).unwrap();
        // 动态取出「原 item 4」所在 trash 行的 id（不依赖 id 是否等于 history id）。
        let trash = get_trash(&c, None).unwrap();
        assert_eq!(trash.len(), 2);
        let t4 = trash
            .iter()
            .find(|t| t.hash == "h3")
            .expect("原 item 4(h3) 应在回收站")
            .id;
        // 恢复该 trash 行：history 重新自增分配 id（旧实现可能复用 trash 中 existing 的 id）。
        restore_item(&mut c, t4).unwrap();
        // 取刚恢复出来的 history 行（id 最大者），再删一次：旧实现会因此冲突报错。
        let hist = get_history(&c, None, 100, true).unwrap();
        let restored_id = hist.iter().map(|h| h.id).max().unwrap();
        delete_item(&mut c, restored_id).unwrap();
        // 最终 trash 应有 2 行：一条从未恢复过的 + 一条刚从恢复再删的（新自增 id，无重复）。
        let trash_final = get_trash(&c, None).unwrap();
        assert_eq!(trash_final.len(), 2);
    }


    #[test]
    fn insert_or_bump_dedups_by_hash() {
        let c = mem_db();
        let id1 = insert_or_bump(&c, None, &sample("same", 100)).unwrap();
        let id2 = insert_or_bump(&c, None, &sample("same", 999)).unwrap();
        assert_eq!(id1, id2, "同 hash 应复用同一行");
        let items = get_history(&c, None, 100, true).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].created_at, 999, "created_at 应更新为最新");
    }

    #[test]
    fn capacity_is_enforced() {
        let c = mem_db();
        for i in 0..(MAX_HISTORY + 10) {
            insert_or_bump(&c, None, &sample(&format!("h{i}"), i)).unwrap();
        }
        let count: i64 = c.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap();
        assert_eq!(count, MAX_HISTORY);
    }

    #[test]
    fn toggle_pin_works() {
        let c = mem_db();
        let id = insert_or_bump(&c, None, &sample("a", 100)).unwrap();
        assert!(toggle_pin(&c, id).unwrap());
        let items = get_history(&c, None, 100, true).unwrap();
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
        let id = insert_or_bump(&c, None, &sample("a", 100)).unwrap();
        delete_item(&mut c, id).unwrap();
        let trash = get_trash(&c, None).unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].hash, "a");
    }

    #[test]
    fn restore_moves_back_to_history() {
        let mut c = mem_db();
        let id = insert_or_bump(&c, None, &sample("a", 100)).unwrap();
        delete_item(&mut c, id).unwrap();
        restore_item(&mut c, id).unwrap();
        assert_eq!(get_history(&c, None, 100, true).unwrap().len(), 1);
        let trash_count: i64 = c
            .query_row("SELECT COUNT(*) FROM trash", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trash_count, 0);
    }

    #[test]
    fn purge_removes_from_trash() {
        let mut c = mem_db();
        let id = insert_or_bump(&c, None, &sample("a", 100)).unwrap();
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
        let a = insert_or_bump(&c, None, &sample("a", 100)).unwrap();
        let b = insert_or_bump(&c, None, &sample("b", 200)).unwrap();
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
        insert_or_bump(&c, None, &sample("a", 100)).unwrap();
        insert_or_bump(&c, None, &sample("b", 200)).unwrap();
        insert_or_bump(&c, None, &sample("c", 300)).unwrap();
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
        let restored = get_trash(&c, None).unwrap();
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

    #[test]
    fn save_type_enabled_respects_settings() {
        use crate::models::ContentType;
        let c = mem_db();
        // 默认三类均启用（Link/Code 归入文本，受 save_text 控制）。
        assert!(save_type_enabled(&c, ContentType::Text));
        assert!(save_type_enabled(&c, ContentType::Link));
        assert!(save_type_enabled(&c, ContentType::Code));
        assert!(save_type_enabled(&c, ContentType::Image));
        assert!(save_type_enabled(&c, ContentType::File));
        // 禁用文本类型：影响 Text/Link/Code，不影响 Image/File。
        update_setting(&c, "save_text", "0").unwrap();
        assert!(!save_type_enabled(&c, ContentType::Text));
        assert!(!save_type_enabled(&c, ContentType::Link));
        assert!(!save_type_enabled(&c, ContentType::Code));
        assert!(save_type_enabled(&c, ContentType::Image));
        assert!(save_type_enabled(&c, ContentType::File));
        // 禁用图片类型。
        update_setting(&c, "save_image", "0").unwrap();
        assert!(!save_type_enabled(&c, ContentType::Image));
        // 恢复文本、禁用文件。
        update_setting(&c, "save_text", "1").unwrap();
        update_setting(&c, "save_file", "0").unwrap();
        assert!(save_type_enabled(&c, ContentType::Text));
        assert!(!save_type_enabled(&c, ContentType::File));
    }

    #[test]
    fn purge_expired_clears_history_and_trash() {
        use rusqlite::params;
        let c = mem_db();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let old = now - 100 * 86_400_000;
        // 未置顶旧条目（应删）+ 置顶旧条目（保留）+ 新条目（保留）
        c.execute(
            "INSERT INTO history (content_type, content_text, source_app, size_bytes, hash, created_at, is_pinned, is_favorite) VALUES ('text','old',?,0,'h_old',?,0,0)",
            params!["app", old],
        )
        .unwrap();
        c.execute(
            "INSERT INTO history (content_type, content_text, source_app, size_bytes, hash, created_at, is_pinned, is_favorite) VALUES ('text','pinned',?,0,'h_pin',?,1,0)",
            params!["app", old],
        )
        .unwrap();
        c.execute(
            "INSERT INTO history (content_type, content_text, source_app, size_bytes, hash, created_at, is_pinned, is_favorite) VALUES ('text','new',?,0,'h_new',?,0,0)",
            params!["app", now],
        )
        .unwrap();
        // trash：旧条目（按 deleted_at 过期，应删）+ 新条目（保留）
        c.execute(
            "INSERT INTO trash (content_type, content_text, source_app, size_bytes, hash, created_at, deleted_at, is_pinned, is_favorite) VALUES ('text','told',?,0,'t_old',?,?,0,0)",
            params!["app", old, old],
        )
        .unwrap();
        c.execute(
            "INSERT INTO trash (content_type, content_text, source_app, size_bytes, hash, created_at, deleted_at, is_pinned, is_favorite) VALUES ('text','tnew',?,0,'t_new',?,?,0,0)",
            params!["app", now, now],
        )
        .unwrap();

        // 30 天留存：删除 1 条超期未置顶历史 + 1 条超期 trash = 2。
        let n = purge_expired(&c, 30).unwrap();
        assert_eq!(n, 2);
        let hist_count: i64 = c.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap();
        assert_eq!(hist_count, 2); // 置顶旧 + 新
        let trash_count: i64 = c.query_row("SELECT COUNT(*) FROM trash", [], |r| r.get(0)).unwrap();
        assert_eq!(trash_count, 1);

        // days<=0 不删除
        assert_eq!(purge_expired(&c, 0).unwrap(), 0);
    }

    // ===== L3 · 局域网共享 DB 层 =====

    /// L3 · 内存库（含 sync_profiles 表与 history 增量列），供共享相关单测使用。
    fn mem_db_lan() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        migrate(&c).unwrap();
        migrate_lan(&c).unwrap();
        c
    }

    fn remote_input<'a>(
        content_text: &'a str,
        hash: &'a str,
        sync_id: &'a str,
        origin: &'a str,
        lamport: i64,
        profile_id: &'a str,
    ) -> RemoteClipInput<'a> {
        RemoteClipInput {
            key: None,
            content_type: "text",
            content_text,
            content_blob: None,
            source_app: origin,
            size_bytes: 10,
            hash,
            is_sensitive: false,
            origin_device: origin,
            sync_id,
            lamport,
            profile_id,
        }
    }

    #[test]
    fn remote_clip_inserts_and_dedups_by_sync_id() {
        let c = mem_db_lan();
        let id1 = insert_remote_clip(&c, remote_input("remote-h1", "h1", "sync-1", "peer-A", 3, "p1"))
            .unwrap();
        assert!(id1.is_some(), "首条应插入");
        // 相同 sync_id → 去重跳过，返回 None。
        let id2 = insert_remote_clip(&c, remote_input("remote-h1", "h1", "sync-1", "peer-A", 3, "p1"))
            .unwrap();
        assert!(id2.is_none(), "同 sync_id 应跳过");
        // 不同 sync_id → 再次插入。
        let id3 = insert_remote_clip(&c, remote_input("remote-h2", "h2", "sync-2", "peer-A", 4, "p1"))
            .unwrap();
        assert!(id3.is_some());
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2, "去重后仅 2 行");
    }

    #[test]
    fn remote_clip_marks_is_remote_and_origin() {
        let c = mem_db_lan();
        insert_remote_clip(&c, remote_input("remote-h1", "h1", "sync-1", "peer-A", 7, "p1")).unwrap();
        let row: (String, String, i64, i64) = c
            .query_row(
                "SELECT origin_device, profile_id, lamport, is_remote FROM history WHERE sync_id = 'sync-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "peer-A");
        assert_eq!(row.1, "p1");
        assert_eq!(row.2, 7);
        assert_eq!(row.3, 1, "is_remote 应为 1");
    }

    #[test]
    fn get_history_exposes_remote_origin() {
        let c = mem_db_lan();
        insert_remote_clip(&c, remote_input("remote-h1", "h1", "sync-1", "peer-A", 7, "p1")).unwrap();
        let items = get_history(&c, None, 100, true).unwrap();
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert!(it.is_remote, "共享条目 is_remote 应为 true");
        assert_eq!(it.origin_device, "peer-A", "应暴露来源设备名");
    }

    #[test]
    fn sync_profiles_crud_and_single_active() {
        let c = mem_db_lan();
        upsert_sync_profile(&c, "p1", "lan", "team", "WKEY1", true, 100).unwrap();
        upsert_sync_profile(&c, "p2", "lan", "home", "WKEY2", false, 200).unwrap();
        // 列表已包含两个配置。
        let list = list_sync_profiles(&c).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "p1");
        // 单激活互斥：激活 p2 后 p1 失活。
        set_active_profile(&c, "p2").unwrap();
        assert!(!is_profile_active(&c, "p1"));
        assert!(is_profile_active(&c, "p2"));
        let active = get_active_profile(&c).unwrap().unwrap();
        assert_eq!(active.id, "p2");
        // 凭证读取（wrapped_key 不对外暴露，但内部可还原）。
        let (wkey, grp) = get_profile_creds(&c, "p2").unwrap();
        assert_eq!(wkey, "WKEY2");
        assert_eq!(grp, "home");
        // 删除 p2。
        delete_sync_profile(&c, "p2").unwrap();
        assert!(!is_profile_active(&c, "p2"));
        assert_eq!(list_sync_profiles(&c).unwrap().len(), 1);
    }

    #[test]
    fn upsert_sync_profile_is_idempotent() {
        let c = mem_db_lan();
        upsert_sync_profile(&c, "p1", "lan", "team", "WKEY1", true, 100).unwrap();
        // 同 id 更新 group/key，不新增行。
        upsert_sync_profile(&c, "p1", "lan", "team2", "WKEYX", false, 100).unwrap();
        let list = list_sync_profiles(&c).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].share_group, "team2");
        // id 不在视图中暴露 wrapped_key；需用 creds 校验。
        assert_eq!(get_profile_creds(&c, "p1").unwrap().0, "WKEYX");
    }
}
