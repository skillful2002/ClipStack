// P4 · 托盘 / 菜单栏（macOS 菜单栏图标、Windows 托盘图标）
//
// 托盘菜单展示最近若干条历史，点击即复制并广播；另有「打开主界面 / 设置 / 退出」。
// 每次捕获到新内容（clipboard-changed）时重建菜单，保证最近列表常新。
// 关闭主窗口时改为隐藏（交由 App 的 window event 处理），使应用常驻托盘。

use tauri::{
    AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
};

use crate::db::{self, DbState};
use crate::models::ContentType;

/// 托盘菜单展示的最近历史条数。
const RECENT_LIMIT: i64 = 5;

/// 构建并安装托盘图标 + 菜单；返回 TrayIcon 句柄（供后续刷新菜单）。
pub fn build_tray(
    app: &AppHandle,
    db: &DbState,
) -> Result<TrayIcon, Box<dyn std::error::Error>> {
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("window icon must exist");
    let menu = build_menu(app, db)?;
    let db_for_event = db.clone();
    let tray = TrayIconBuilder::with_id("clipstack-tray")
        .icon(icon)
        .tooltip("ClipStack")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| handle_menu_event(app, event, &db_for_event))
        .build(app)?;
    Ok(tray)
}

/// 重建托盘菜单（最近历史 + 固定项）。捕获到新内容时调用。
pub fn refresh_menu(tray: &TrayIcon, app: &AppHandle, db: &DbState) {
    if let Ok(menu) = build_menu(app, db) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn build_menu(
    app: &AppHandle,
    db: &DbState,
) -> Result<Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let menu = Menu::new(app)?;
    let conn = db.conn.lock().expect("db lock poisoned");
    let recent = db::get_recent(&conn, RECENT_LIMIT)?;
    drop(conn);

    if recent.is_empty() {
        let empty = MenuItem::with_id(app, "empty", "（暂无历史记录）", false, None::<&str>)?;
        menu.append(&empty)?;
    } else {
        for it in &recent {
            let label = format!("{} {}", type_prefix(&it.content_type), truncate(&it.preview, 40));
            let id = format!("copy:{}", it.id);
            let item = MenuItem::with_id(app, id, label, true, None::<&str>)?;
            menu.append(&item)?;
        }
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "open_main", "打开主界面", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::quit(app, Some("退出 ClipStack"))?)?;
    Ok(menu)
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent, db: &DbState) {
    let id = event.id().as_ref().to_string();
    if let Some(rest) = id.strip_prefix("copy:") {
        if let Ok(id_num) = rest.parse::<i64>() {
            let conn = db.conn.lock().expect("db lock poisoned");
            if let Ok(item) = db::get_item(&conn, id_num) {
                drop(conn);
                match crate::clipboard::set_clipboard_text(&item.content_text) {
                    Ok(()) => {
                        let _ = app.emit("tray-copied", serde_json::json!({ "id": id_num }));
                    }
                    Err(e) => eprintln!("[clipstack] tray copy failed: {e}"),
                }
            }
        }
        return;
    }
    match id.as_str() {
        "open_main" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            let _ = app.emit("show-view", "all");
        }
        "settings" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            let _ = app.emit("show-view", "settings");
        }
        _ => {}
    }
}

/// 类型短标签（无 emoji，符合文件规范）。
fn type_prefix(ct: &ContentType) -> &'static str {
    match ct {
        ContentType::Text => "[文]",
        ContentType::Link => "[链]",
        ContentType::Code => "[码]",
        ContentType::Image => "[图]",
        ContentType::File => "[件]",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let taken: String = s.chars().take(max).collect();
    format!("{taken}…")
}
