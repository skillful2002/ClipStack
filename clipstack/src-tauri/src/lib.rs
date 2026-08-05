// P0：拉起最小窗口。
// P1：挂载剪贴板捕获引擎（monitor 线程 + 忽略列表状态 + 落库）。
// P2：托管 SQLite 连接（DbState），注册读写命令，启动时从 DB 加载忽略列表。

mod clipboard;
mod commands;
mod db;
mod models;

use std::sync::{Arc, Mutex};

use tauri::Manager;

use clipboard::MonitorState;
use db::{AppDb, DbState};

pub fn run() {
    let monitor_state: Arc<Mutex<MonitorState>> = Arc::new(Mutex::new(MonitorState::default()));

    tauri::Builder::default()
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_item,
            commands::get_history,
            commands::delete_item,
            commands::toggle_pin,
            commands::toggle_favorite,
            commands::update_setting,
            commands::get_settings,
            commands::add_ignored_app,
            commands::get_ignored_apps,
            commands::copy_item,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ClipStack");
}
