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
use crate::AppState;
use crate::i18n::{tray_about, tray_empty, tray_help, tray_lock, tray_locked, tray_open_main, tray_quit, tray_settings, Lang, MenuLang};
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
    state: &AppState,
) -> Result<TrayIcon, Box<dyn std::error::Error>> {
    // 托盘图标：macOS 使用单色模板图标以自动适配明暗菜单栏；
    // Windows / Linux 使用彩色图标。
    #[cfg(target_os = "macos")]
    let icon = Image::from_bytes(include_bytes!("../icons/tray-icon-template.png"))
        .expect("failed to load tray icon template");
    #[cfg(not(target_os = "macos"))]
    let icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"))
        .expect("failed to load tray icon");
    let menu = build_menu(app, db, state)?;
    let db_for_event = db.clone();
    let monitor_for_event = monitor.clone();
    let state_for_event = state.clone();
    let tray = TrayIconBuilder::with_id("clipstack-tray")
        .icon(icon)
        .tooltip("ClipStack")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| {
            handle_menu_event(app, event, &db_for_event, &monitor_for_event, &state_for_event)
        })
        .build(app)?;

    // macOS 菜单栏图标标记为模板，系统会根据菜单栏深浅主题自动反色。
    #[cfg(target_os = "macos")]
    tray.set_icon_as_template(true)?;

    Ok(tray)
}

/// 重建托盘菜单（最近历史 + 固定项）。捕获到新内容时调用。
pub fn refresh_menu(tray: &TrayIcon, app: &AppHandle, db: &DbState, state: &AppState) {
    if let Ok(menu) = build_menu(app, db, state) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn build_menu(
    app: &AppHandle,
    db: &DbState,
    state: &AppState,
) -> Result<Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let menu = Menu::new(app)?;
    // 锁定顺序与捕获线程一致：先 key 后 conn，避免死锁。
    let key_guard = db.key.lock().expect("key lock poisoned");
    let conn = db.conn.lock().expect("db lock poisoned");
    // 锁定态（已设主密码且当前锁定）：不展示任何历史明文/密文。
    let locked = state.is_locked() && db::has_master_password(&conn);
    // 是否已设主密码（用于决定是否显示「锁定」菜单——仅解锁态且已设密码时显示）。
    let has_pw = db::has_master_password(&conn);
    let limit = db::get_int_setting(&conn, TRAY_HISTORY_KEY, DEFAULT_TRAY_HISTORY);
    let recent = if locked {
        Vec::new()
    } else {
        db::get_recent_tray(&conn, key_guard.as_ref(), limit)?
    };
    // 解析菜单语言：显式选择具体语言时从设置直接取值（首帧即正确）；
    // system/未知则使用前端推送的已解析语言（MenuLang 状态，缺省英文）。
    let setting = db::get_string_setting(&conn, "language", "system");
    let lang = Lang::resolve(
        &setting,
        *app.state::<MenuLang>().0.lock().expect("menu lang lock poisoned"),
    );
    drop(conn);
    drop(key_guard);

    if locked {
        // 锁定态：仅给一个「点击解锁」占位项，绝不泄露明文或密文。
        let item = MenuItem::with_id(app, "unlock", tray_locked(lang), true, None::<&str>)?;
        menu.append(&item)?;
    } else if recent.is_empty() {
        let empty = MenuItem::with_id(app, "empty", tray_empty(lang), false, None::<&str>)?;
        menu.append(&empty)?;
    } else {
        for (idx, it) in recent.iter().enumerate() {
            let label = truncate(&it.preview, 40);
            let id = format!("copy:{}", it.id);
            let icon = type_tray_icon(it.content_type)?;
            // 前 9 条历史绑定 ⌘1–⌘9，在托盘菜单打开时按数字即可快速复制；
            // 菜单项 accelerator 仅在菜单可见时生效，不会注册成全局热键，与侧边栏 ⌘1–⌘6 不冲突。
            let mut builder = IconMenuItemBuilder::with_id(id, label).icon(icon);
            if idx < 9 {
                builder = builder.accelerator(&format!("CmdOrCtrl+{}", idx + 1));
            }
            let item = builder.build(app)?;
            menu.append(&item)?;
        }
    }

    // 菜单项图标（内嵌避免发布时资源路径依赖）。
    let open_icon = Image::from_bytes(include_bytes!("../icons/menu-open.png"))?;
    let settings_icon = Image::from_bytes(include_bytes!("../icons/menu-settings.png"))?;
    let about_icon = Image::from_bytes(include_bytes!("../icons/menu-about.png"))?;
    let help_icon = Image::from_bytes(include_bytes!("../icons/menu-help.png"))?;
    let lock_icon = Image::from_bytes(include_bytes!("../icons/menu-lock.png"))?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(
        &IconMenuItemBuilder::with_id("open_main", tray_open_main(lang))
            .icon(open_icon)
            .accelerator("CmdOrCtrl+Shift+V")
            .build(app)?,
    )?;
    menu.append(
        &IconMenuItemBuilder::with_id("settings", tray_settings(lang))
            .icon(settings_icon)
            .accelerator("CmdOrCtrl+,")
            .build(app)?,
    )?;
    // 已设主密码时在「设置」下追加「锁定」菜单项（仅解锁态显示，锁定态 history 区已有占位项）。
    if has_pw && !locked {
        menu.append(
            &IconMenuItemBuilder::with_id("tray_lock", tray_lock(lang))
                .icon(lock_icon)
                .build(app)?,
        )?;
    }
    // 「设置」与「关于系统」之间以横线分隔，两组功能区分更清晰。
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    // 「设置」与「关于系统」之间以横线分隔，两组功能区分更清晰。
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(
        &IconMenuItemBuilder::with_id("help", tray_help(lang))
            .icon(help_icon)
            .build(app)?,
    )?;
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
    state: &AppState,
) {
    let id = event.id().as_ref().to_string();
    if let Some(rest) = id.strip_prefix("copy:") {
        if let Ok(id_num) = rest.parse::<i64>() {
            // 锁定态不允许从托盘复制（此时历史项已被「已锁定」项替换，正常不会走到这里）。
            if state.is_locked() {
                return;
            }
            // 取密钥用于解密（key=None 时透传明文，兼容未启用安全）。
            // 锁顺序与 build_menu / 捕获线程一致：先 key 后 conn。
            let key_guard = db.key.lock().expect("key lock poisoned");
            let (item, blob) = {
                let conn = db.conn.lock().expect("db lock poisoned");
                let item = db::get_item(&conn, key_guard.as_ref(), id_num).ok();
                let blob: Option<Vec<u8>> = item.as_ref().and_then(|it| {
                    if matches!(it.content_type, ContentType::Image | ContentType::File) {
                        conn.query_row(
                            "SELECT content_blob FROM history WHERE id = ?",
                            [id_num],
                            |r| r.get(0),
                        )
                        .ok()
                    } else {
                        None
                    }
                });
                (item, blob)
            };
            drop(key_guard);
            if let Some(item) = item {
                match item.content_type {
                    ContentType::Image => {
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
                    ContentType::File => {
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
        "unlock" => {
            // 锁定态占位项被点击：打开主界面，前端据此展示锁屏让用户解锁。
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            set_dock_visible(app);
            let _ = app.emit("show-view", "settings");
        }
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
        "help" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            set_dock_visible(app);
            let _ = app.emit("show-view", "help");
        }
        "about" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            set_dock_visible(app);
            let _ = app.emit("show-view", "about");
        }
        "tray_lock" => {
            // 托盘「锁定」菜单：立即锁定应用。
            state.set_locked(true);
            let _ = app.emit("refresh-tray", ());
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
