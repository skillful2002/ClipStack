// P0：拉起最小窗口。
// P1：挂载剪贴板捕获引擎（monitor 线程 + 忽略列表状态 + 落库）。
// P2：托管 SQLite 连接（DbState），注册读写命令，启动时从 DB 加载忽略列表。
// P4：安装托盘菜单 + 全局快捷键；关闭主窗口改为隐藏（常驻托盘）。

mod clipboard;
mod commands;
mod db;
mod models;
mod tray;

use std::sync::{Arc, Mutex};

use tauri::{Listener, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use clipboard::MonitorState;
use db::{AppDb, DbState};

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
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_for_event.hide();
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running ClipStack");
}
