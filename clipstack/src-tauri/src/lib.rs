// P0：拉起最小窗口。P1：挂载剪贴板捕获引擎（monitor 线程 + 忽略列表状态 + command）。

mod clipboard;

use std::sync::{Arc, Mutex};

use clipboard::MonitorState;

pub fn run() {
    let state: Arc<Mutex<MonitorState>> = Arc::new(Mutex::new(MonitorState::default()));

    tauri::Builder::default()
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![clipboard::add_ignored_app])
        .setup(move |app| {
            clipboard::start_monitor(app.handle().clone(), state.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running ClipStack");
}
