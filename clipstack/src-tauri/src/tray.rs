// P4 · 托盘 / 菜单栏（macOS 菜单栏图标、Windows 托盘图标）
//
// 托盘菜单展示最近若干条历史，点击即复制并广播；另有「打开主界面 / 设置 / 关于系统 / 退出」。
// 每次捕获到新内容（clipboard-changed）时重建菜单，保证最近列表常新。
// 关闭主窗口时改为隐藏（交由 App 的 window event 处理），使应用常驻托盘。

use tauri::{
    AppHandle, Emitter, Manager,
    image::Image,
    menu::{IconMenuItemBuilder, Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
};
use std::sync::{Arc, Mutex};

use crate::clipboard::MonitorState;
use crate::db::{self, DbState};
use crate::i18n::{tray_about, tray_empty, tray_open_main, tray_quit, tray_settings, Lang, MenuLang};
use crate::models::ContentType;
use crate::set_dock_visible;

/// 托盘菜单展示历史记录条数的设置键与缺省值。
const TRAY_HISTORY_KEY: &str = "tray_history_count";
pub const DEFAULT_TRAY_HISTORY: i64 = 30;

/// 构建并安装托盘图标 + 菜单；返回 TrayIcon 句柄（供后续刷新菜单）。
pub fn build_tray(
    app: &AppHandle,
    db: &DbState,
    monitor: &Arc<Mutex<MonitorState>>,
) -> Result<TrayIcon, Box<dyn std::error::Error>> {
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("window icon must exist");
    let menu = build_menu(app, db)?;
    let db_for_event = db.clone();
    let monitor_for_event = monitor.clone();
    let tray = TrayIconBuilder::with_id("clipstack-tray")
        .icon(icon)
        .tooltip("ClipStack")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| {
            handle_menu_event(app, event, &db_for_event, &monitor_for_event)
        })
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
    let limit = db::get_int_setting(&conn, TRAY_HISTORY_KEY, DEFAULT_TRAY_HISTORY);
    let recent = db::get_recent_tray(&conn, limit)?;
    // 解析菜单语言：显式选择具体语言时从设置直接取值（首帧即正确）；
    // system/未知则使用前端推送的已解析语言（MenuLang 状态，缺省英文）。
    let setting = db::get_string_setting(&conn, "language", "system");
    let lang = Lang::resolve(
        &setting,
        *app.state::<MenuLang>().0.lock().expect("menu lang lock poisoned"),
    );
    drop(conn);

    if recent.is_empty() {
        let empty = MenuItem::with_id(app, "empty", tray_empty(lang), false, None::<&str>)?;
        menu.append(&empty)?;
    } else {
        for it in &recent {
            let label = truncate(&it.preview, 40);
            let id = format!("copy:{}", it.id);
            let icon = type_tray_icon(it.content_type)?;
            let item = IconMenuItemBuilder::with_id(id, label).icon(icon).build(app)?;
            menu.append(&item)?;
        }
    }

    // 菜单项图标（内嵌避免发布时资源路径依赖）。
    let open_icon = Image::from_bytes(include_bytes!("../icons/menu-open.png"))?;
    let settings_icon = Image::from_bytes(include_bytes!("../icons/menu-settings.png"))?;
    let about_icon = Image::from_bytes(include_bytes!("../icons/menu-about.png"))?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(
        &IconMenuItemBuilder::with_id("open_main", tray_open_main(lang))
            .icon(open_icon)
            .build(app)?,
    )?;
    menu.append(
        &IconMenuItemBuilder::with_id("settings", tray_settings(lang))
            .icon(settings_icon)
            .build(app)?,
    )?;
    // 「设置」与「关于系统」之间以横线分隔，两组功能区分更清晰。
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(
        &IconMenuItemBuilder::with_id("about", tray_about(lang))
            .icon(about_icon)
            .build(app)?,
    )?;
    // 「关于系统」与「退出」之间以横线分隔，与上方「打开主界面 / 设置」分组呼应。
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&PredefinedMenuItem::quit(app, Some(tray_quit(lang)))?)?;
    Ok(menu)
}

fn handle_menu_event(
    app: &AppHandle,
    event: tauri::menu::MenuEvent,
    db: &DbState,
    monitor: &Arc<Mutex<MonitorState>>,
) {
    let id = event.id().as_ref().to_string();
    if let Some(rest) = id.strip_prefix("copy:") {
        if let Ok(id_num) = rest.parse::<i64>() {
            let conn = db.conn.lock().expect("db lock poisoned");
            if let Ok(item) = db::get_item(&conn, id_num) {
                drop(conn);
                match item.content_type {
                    crate::models::ContentType::Image => {
                        // 图片：从数据库读取 PNG 二进制，解码后写回剪贴板。
                        let conn2 = db.conn.lock().expect("db lock poisoned");
                        let blob: Option<Vec<u8>> = conn2
                            .query_row(
                                "SELECT content_blob FROM history WHERE id = ?",
                                [id_num],
                                |r| r.get(0),
                            )
                            .ok();
                        drop(conn2);
                        if let Some(png_bytes) = blob {
                            crate::clipboard::note_self_copy_image(monitor, &png_bytes);
                            match crate::clipboard::set_clipboard_image(&png_bytes) {
                                Ok(()) => {
                                    let _ = app.emit("tray-copied", serde_json::json!({ "id": id_num }));
                                }
                                Err(e) => eprintln!("[clipstack] tray image copy failed: {e}"),
                            }
                        }
                    }
                    crate::models::ContentType::File => {
                        // 文件：从数据库读取路径列表（JSON），写回剪贴板文件列表。
                        let conn2 = db.conn.lock().expect("db lock poisoned");
                        let blob: Option<Vec<u8>> = conn2
                            .query_row(
                                "SELECT content_blob FROM history WHERE id = ?",
                                [id_num],
                                |r| r.get(0),
                            )
                            .ok();
                        drop(conn2);
                        let paths = crate::commands::paths_from_storage(blob.as_deref(), &item.content_text);
                        crate::clipboard::note_self_copy_files(monitor, &paths);
                        match crate::clipboard::set_clipboard_file_list(&paths) {
                            Ok(()) => {
                                let _ = app.emit("tray-copied", serde_json::json!({ "id": id_num }));
                            }
                            Err(e) => eprintln!("[clipstack] tray file copy failed: {e}"),
                        }
                    }
                    _ => {
                        // 文本 / 链接 / 代码：直接写回文本。
                        crate::clipboard::note_self_copy(monitor, &item.content_text);
                        match crate::clipboard::set_clipboard_text(&item.content_text) {
                            Ok(()) => {
                                let _ = app.emit("tray-copied", serde_json::json!({ "id": id_num }));
                            }
                            Err(e) => eprintln!("[clipstack] tray copy failed: {e}"),
                        }
                    }
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
            // 窗口打开时恢复 Dock 图标（仅 macOS）。
            set_dock_visible(app);
            let _ = app.emit("show-view", "all");
        }
        "settings" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            set_dock_visible(app);
            let _ = app.emit("show-view", "settings");
        }
        "about" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            set_dock_visible(app);
            let _ = app.emit("show-view", "about");
        }
        _ => {}
    }
}

/// 为托盘历史条目加载对应内容类型的图标（与首界面分类图标同字形）。
fn type_tray_icon(ct: ContentType) -> Result<Image<'static>, Box<dyn std::error::Error>> {
    const ICON_TEXT: &[u8] = include_bytes!("../icons/menu-type-text.png");
    const ICON_LINK: &[u8] = include_bytes!("../icons/menu-type-link.png");
    const ICON_CODE: &[u8] = include_bytes!("../icons/menu-type-code.png");
    const ICON_IMAGE: &[u8] = include_bytes!("../icons/menu-type-image.png");
    let bytes = match ct {
        ContentType::Link => ICON_LINK,
        ContentType::Code => ICON_CODE,
        ContentType::Image => ICON_IMAGE,
        _ => ICON_TEXT, // text；file 在托盘历史中已排除，fallback 为文本图标
    };
    Ok(Image::from_bytes(bytes)?)
}

/// 截断过长的预览文本，超出部分以省略号结尾。
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let taken: String = s.chars().take(max).collect();
    format!("{taken}…")
}
