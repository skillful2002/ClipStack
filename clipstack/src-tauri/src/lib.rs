// P0：拉起最小窗口。
// P1：挂载剪贴板捕获引擎（monitor 线程 + 忽略列表状态 + 落库）。
// P2：托管 SQLite 连接（DbState），注册读写命令，启动时从 DB 加载忽略列表。
// P4：安装托盘菜单 + 全局快捷键；关闭主窗口改为隐藏（常驻托盘）。

mod clipboard;
mod commands;
mod crypto;
mod db;
mod i18n;
mod keychain;
mod models;
mod tray;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Listener, Manager};
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

// ===== P0 · 应用锁状态 =====
//
// 托管为 Tauri State。锁定态时内存无密钥（`db.key == None`），监控线程跳过捕获、
// 内容类命令返回 `Err("locked")`。`AppState` 内部以 `Arc` 包裹，便于后台自动锁线程
// 与窗口事件闭包共享同一份原子状态（Clone 仅复制 Arc，不复制底层）。

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    /// 锁定态标记。
    locked: AtomicBool,
    /// 最近一次用户活动时刻（用于「闲置自动锁」计时）。
    last_activity: Mutex<Instant>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            inner: Arc::new(AppStateInner {
                locked: AtomicBool::new(false),
                last_activity: Mutex::new(Instant::now()),
            }),
        }
    }

    /// 当前是否锁定。
    pub fn is_locked(&self) -> bool {
        self.inner.locked.load(Ordering::SeqCst)
    }

    /// 设置锁定态；解锁时顺带刷新活动时刻，避免解锁后立刻被闲置计时再次锁定。
    pub fn set_locked(&self, v: bool) {
        self.inner.locked.store(v, Ordering::SeqCst);
        if !v {
            self.touch_activity();
        }
    }

    /// 记录一次用户活动（内容类命令调用），重置闲置计时。
    pub fn touch_activity(&self) {
        *self.inner.last_activity.lock().expect("activity lock poisoned") = Instant::now();
    }

    /// 距上次活动经过的秒数。
    pub fn idle_secs(&self) -> u64 {
        self.inner
            .last_activity
            .lock()
            .expect("activity lock poisoned")
            .elapsed()
            .as_secs()
    }
}

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

            // 启动阶段载入/生成「内部数据库加密密钥」（与主密码无关，始终存在）：
            // 用于数据库内容加密，使无论是否设置主密码、是否锁定，落库数据都保持加密。
            // 载入失败（如钥匙串暂不可用）时退回明文兼容，不阻断启动。
            match keychain::load_or_create_internal_key() {
                Ok(internal) => {
                    {
                        let mut kg = db_state.key.lock().expect("key lock poisoned");
                        *kg = Some(crypto::Key(internal));
                    }
                    // 一次性迁移：将启用安全前的明文历史用内部密钥加密；
                    // 已加密行自动跳过（避免双重加密）。锁顺序遵循「先 key 后 conn」。
                    let kg = db_state.key.lock().expect("key lock poisoned");
                    let conn = db_state.conn.lock().expect("db lock poisoned");
                    if let Some(k) = kg.as_ref() {
                        let _ = db::migrate_plaintext_to_encrypted(&conn, k);
                    }
                }
                Err(e) => {
                    eprintln!("[clipstack] 内部数据库密钥载入失败，数据将以明文兼容模式运行: {e}");
                }
            }

            // 应用锁状态：启动始终处于「解锁」状态（用户要求：程序启动时不要锁定）。
            // 主密码仅作为「手动锁定 / 闲置超时锁定」之后的解锁凭据——启动时直接可用、历史立即可读。
            // 锁只保护界面，不影响内部加密密钥。
            let app_state = AppState::new();
            app_state.set_locked(false);
            app.manage(app_state.clone());

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
            let tray = tray::build_tray(app.handle(), &db_state, &monitor_state, &app_state)?;
            let tray_clone = tray.clone();
            let db_for_refresh = db_state.clone();
            // 每次捕获到新内容时刷新托盘菜单，保持最近列表常新。
            let state_for_refresh = app_state.clone();
            app.listen("clipboard-changed", move |_event| {
                tray::refresh_menu(
                    &tray_clone,
                    tray_clone.app_handle(),
                    &db_for_refresh,
                    &state_for_refresh,
                );
            });
            // 托盘历史条数设置变更时立即刷新托盘菜单。
            let tray_for_settings = tray.clone();
            let db_for_settings = db_state.clone();
            let state_for_settings_tray = app_state.clone();
            app.listen("tray-settings-changed", move |_event| {
                tray::refresh_menu(
                    &tray_for_settings,
                    tray_for_settings.app_handle(),
                    &db_for_settings,
                    &state_for_settings_tray,
                );
            });

            // 前端切换语言时接收已解析语言，更新托盘菜单文案并重建菜单。
            let tray_for_lang = tray.clone();
            let db_for_lang = db_state.clone();
            let app_handle_for_lang = app.handle().clone();
            let state_for_lang_tray = app_state.clone();
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
                            &state_for_lang_tray,
                        );
                    }
                }
            });

            // 解锁 / 锁定后刷新托盘菜单：解锁时用最近历史替换「已锁定」占位项，
            // 锁定时反之。该监听运行于主线程，调用 set_menu 安全（TrayIcon 的菜单
            // 变更需在主线程执行）。
            let tray_for_lock = tray.clone();
            let db_for_lock = db_state.clone();
            let state_for_lock = app_state.clone();
            app.listen("refresh-tray", move |_event| {
                tray::refresh_menu(
                    &tray_for_lock,
                    tray_for_lock.app_handle(),
                    &db_for_lock,
                    &state_for_lock,
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
                let app_for_event = app.handle().clone();
                let state_for_event = app_state.clone();
                win.on_window_event(move |event| {
                    match event {
                        tauri::WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            let _ = win_for_event.hide();
                            // 窗口收起后隐藏 Dock 图标（仅 macOS）：未打开界面时不在程序坞显示图标。
                            set_dock_hidden(&app_for_event);
                        }
                        // 窗口重新获得焦点（含从托盘重新打开）：重置闲置计时，
                        // 使「在闲置自动锁定时间范围内再次打开主界面」保持未锁定、无需手动解锁。
                        // 注意：窗口「失去焦点」不再锁定——锁定只由闲置计时触发，
                        // 否则隐藏到托盘再打开会被立即要求解锁，与需求相悖。
                        tauri::WindowEvent::Focused(true) => {
                            state_for_event.touch_activity();
                        }
                        _ => {}
                    }
                });
            }

            // 闲置自动锁：后台线程每 5 秒检查一次。仅当已设置主密码、未锁定、
            // 且距「上次用户活动」超过 `auto_lock_idle_seconds`（>0）时锁定并清空密钥。
            // 阈值实时读取：改设置（含从 0 改为某值）即时生效，无需重启。
            // 缺省 0 = 永久（从不自动锁定）。锁定后同步刷新托盘菜单（锁定态占位项）。
            {
                let db_idle = db_state.clone();
                let state_idle = app_state.clone();
                let app_idle = app.handle().clone();
                std::thread::Builder::new()
                    .name("auto-lock".into())
                    .spawn(move || {
                        loop {
                            std::thread::sleep(Duration::from_secs(5));
                            let idle_timeout = {
                                let c = db_idle.conn.lock().expect("db lock poisoned");
                                db::get_int_setting(&c, "auto_lock_idle_seconds", 0)
                            };
                            if idle_timeout <= 0 {
                                continue;
                            }
                            if state_idle.is_locked() {
                                continue;
                            }
                            let has_pw = {
                                let c = db_idle.conn.lock().expect("db lock poisoned");
                                db::has_master_password(&c)
                            };
                            if has_pw && state_idle.idle_secs() >= idle_timeout as u64 {
                                state_idle.set_locked(true);
                                // 注意：仅置应用锁标记，不清空内部数据库密钥
                                // （数据库加密由内部密钥负责，锁定只保护界面与托盘展示）。
                                // 通知前端切到锁定界面，并刷新托盘菜单（锁定态占位项）。
                                let _ = app_idle.emit("app-lock-changed", true);
                                let _ = app_idle.emit("refresh-tray", ());
                            }
                        }
                    })
                    .ok();
            }

            // 留存过期清理：每 10 分钟按 `retention_days` 清理超期历史（未置顶）与回收站内容。
            // days<=0 视为永久保留，线程直接跳过，不触碰库。
            {
                let db_ret = db_state.clone();
                std::thread::Builder::new()
                    .name("retention-sweep".into())
                    .spawn(move || {
                        loop {
                            std::thread::sleep(Duration::from_secs(600));
                            let days = {
                                let c = db_ret.conn.lock().expect("db lock poisoned");
                                db::get_int_setting(&c, "retention_days", 0)
                            };
                            if days > 0 {
                                let c = db_ret.conn.lock().expect("db lock poisoned");
                                let _ = db::purge_expired(&c, days);
                            }
                        }
                    })
                    .ok();
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
            commands::delete_items,
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
            commands::copy_file,
            commands::was_first_run,
            // ===== P0 · 应用锁 / 主密码 =====
            commands::setup_master_password,
            commands::unlock,
            commands::unlock_touch_id,
            commands::lock,
            commands::touch_activity,
            commands::change_master_password,
            commands::clear_master_password,
            commands::is_locked,
            commands::has_master_password,
            commands::set_touch_id,
            // ===== P1b · 留存过期（含 trash）=====
            commands::purge_expired,
            // ===== Touch ID 可用性检测 =====
            commands::check_touch_id_available,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ClipStack");
}
