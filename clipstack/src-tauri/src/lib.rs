// P0：拉起最小窗口。
// P1：挂载剪贴板捕获引擎（monitor 线程 + 忽略列表状态 + 落库）。
// P2：托管 SQLite 连接（DbState），注册读写命令，启动时从 DB 加载忽略列表。
// P4：安装托盘菜单 + 全局快捷键；关闭主窗口改为隐藏（常驻托盘）。

mod clipboard;
mod commands;
mod db;
mod i18n;
mod models;
mod tray;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use tauri::{AppHandle, Listener, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use clipboard::MonitorState;
use db::{AppDb, DbState};

/// macOS：以「配件（Accessory）」模式运行 —— 不在 Dock 显示图标、不成为前台应用（仅托盘常驻）。
/// 窗口关闭收进托盘时使用，满足「未打开界面时不显示 Dock 图标」。
#[cfg(target_os = "macos")]
pub fn set_dock_hidden(app: &AppHandle) {
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

/// macOS：恢复「常规（Regular）」模式 —— 在 Dock 显示图标并成为前台应用。窗口打开时使用。
#[cfg(target_os = "macos")]
pub fn set_dock_visible(app: &AppHandle) {
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
}

/// 非 macOS 平台无 Dock 概念，提供同名 no-op 以便调用处无需每处写 cfg 判断。
#[cfg(not(target_os = "macos"))]
pub fn set_dock_hidden(_app: &AppHandle) {}

/// 非 macOS 平台无 Dock 概念，提供同名 no-op。
#[cfg(not(target_os = "macos"))]
pub fn set_dock_visible(_app: &AppHandle) {}

pub fn run() {
    let monitor_state: Arc<Mutex<MonitorState>> = Arc::new(Mutex::new(MonitorState::default()));

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(monitor_state.clone())
        .setup(move |app| {
            // 打开（或创建）SQLite 数据库。
            let app_db: AppDb = db::open(app.handle())?;
            let db_state: DbState = Arc::new(app_db);
            app.manage(db_state.clone());

            // 首次运行判定与窗口显隐：在 setup 阶段同步完成，确保窗口在事件循环启动前就已显示，
            // 避免原先「前端异步加载后再 show」带来的时机过晚与 macOS 激活策略抖动（窗口失焦沉底）。
            // first_run_flag 供前端读取以决定是否自动进入设置页。
            let first_run_flag = Arc::new(AtomicBool::new(false));
            app.manage(first_run_flag.clone());
            let is_first = {
                let conn = db_state.conn.lock().expect("db lock poisoned");
                db::get_string_setting(&conn, "first_launch_done", "0") != "1"
            };
            if is_first {
                // 首次运行：恢复常规模式（Dock 可见）并显示主窗口，引导用户完成初始配置。
                set_dock_visible(app.handle());
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let conn = db_state.conn.lock().expect("db lock poisoned");
                let _ = db::update_setting(&conn, "first_launch_done", "1");
                first_run_flag.store(true, Ordering::SeqCst);
            } else {
                // 非首次运行：仅托盘常驻，隐藏 Dock 图标，窗口保持隐藏。
                set_dock_hidden(app.handle());
            }

            // 托盘菜单语言状态（前端通过 language-changed 事件推送已解析语言）。
            app.manage(i18n::MenuLang::default());

            // 启动时把持久化的忽略应用载入内存过滤集合，使其即时生效。
            {
                let conn = db_state.conn.lock().expect("db lock poisoned");
                if let Ok(ignored) = db::get_ignored_apps(&conn) {
                    for name in ignored {
                        clipboard::ignore_app(&monitor_state, &name);
                    }
                }
            }

            // 启动监控线程并传入 DB 句柄，捕获即落库。
            clipboard::start_monitor(app.handle().clone(), monitor_state.clone(), db_state.clone());

            // 后台预热「已安装应用」列表缓存：在启动阶段于独立线程提前完成一次 mdls 扫描，
            // 使首次打开设置界面时直接命中缓存、不再因扫描而卡顿。
            let _ = std::thread::Builder::new()
                .name("installed-apps-warmup".into())
                .spawn(|| {
                    let _ = clipboard::list_installed_apps();
                });

            // 托盘菜单（最近历史 + 固定项）。
            let tray = tray::build_tray(app.handle(), &db_state, &monitor_state)?;
            let tray_clone = tray.clone();
            let db_for_refresh = db_state.clone();
            // 每次捕获到新内容时刷新托盘菜单，保持最近列表常新。
            app.listen("clipboard-changed", move |_event| {
                tray::refresh_menu(&tray_clone, tray_clone.app_handle(), &db_for_refresh);
            });
            // 托盘历史条数设置变更时立即刷新托盘菜单。
            let tray_for_settings = tray.clone();
            let db_for_settings = db_state.clone();
            app.listen("tray-settings-changed", move |_event| {
                tray::refresh_menu(
                    &tray_for_settings,
                    tray_for_settings.app_handle(),
                    &db_for_settings,
                );
            });

            // 前端切换语言时接收已解析语言，更新托盘菜单文案并重建菜单。
            let tray_for_lang = tray.clone();
            let db_for_lang = db_state.clone();
            let app_handle_for_lang = app.handle().clone();
            app.listen("language-changed", move |event| {
                if let Ok(lang_str) = serde_json::from_str::<String>(event.payload()) {
                    if let Some(l) = i18n::Lang::from_setting(&lang_str) {
                        if let Some(state) = app_handle_for_lang.try_state::<i18n::MenuLang>() {
                            *state.0.lock().expect("menu lang lock poisoned") = l;
                        }
                        tray::refresh_menu(
                            &tray_for_lang,
                            tray_for_lang.app_handle(),
                            &db_for_lang,
                        );
                    }
                }
            });

            // 全局快捷键：Cmd/Ctrl + Shift + V 切换主界面可见性。
            app.global_shortcut().on_shortcut("CmdOrCtrl+Shift+V", |app, _shortcut, event| {
                if event.state() == ShortcutState::Pressed {
                    if let Some(w) = app.get_webview_window("main") {
                        match w.is_visible() {
                            Ok(true) => {
                                let _ = w.hide();
                            }
                            _ => {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                }
            })?;

            // 关闭主窗口改为隐藏（常驻托盘），而非退出进程。
            if let Some(w) = app.get_webview_window("main") {
                let win = w.clone();
                let win_for_event = win.clone();
                let app_for_event = app.handle().clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_for_event.hide();
                        // 窗口收起后隐藏 Dock 图标（仅 macOS）：未打开界面时不在程序坞显示图标。
                        set_dock_hidden(&app_for_event);
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_item,
            commands::get_history,
            commands::delete_item,
            commands::get_trash,
            commands::restore_item,
            commands::purge_item,
            commands::empty_trash,
            commands::clear_history,
            commands::toggle_pin,
            commands::toggle_favorite,
            commands::update_setting,
            commands::get_settings,
            commands::add_ignored_app,
            commands::get_ignored_apps,
            commands::remove_ignored_app,
            commands::list_installed_apps,
            commands::copy_item,
            commands::get_item_blob,
            commands::get_trash_blob,
            commands::get_system_info,
            commands::copy_image,
            commands::was_first_run,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ClipStack");
}
