//! 顶部原生菜单栏（macOS 菜单栏 App / Edit / Window / Help）。
//!
//! Tauri 在 macOS 上默认用 `muda` 生成顶部菜单，并把预定义项（Copy / Quit / About …）
//! 写死为英文、不跟随系统语言。这里自定义菜单栏，对预定义项传入 `i18n::native_menu`
//! 提供的本地化文案覆盖默认英文。`PredefinedMenuItem` 仅覆盖标题文本，selector
//! （copy: / quit: / hide: …）保持不变，因此 Cmd+C 等编辑 / 窗口快捷键在 webview 中
//! 仍由 responder chain 正常处理。

use tauri::{
    AppHandle,
    menu::{Menu, MenuItemBuilder, PredefinedMenuItem, Submenu},
};
use crate::i18n::{native_menu, Lang};

/// 构建顶部菜单栏（menubar 容器），文案跟随 `lang`。
pub fn build_app_menu(app: &AppHandle, lang: Lang) -> tauri::Result<Menu<tauri::Wry>> {
    let s = native_menu(lang);

    // ===== App 菜单（ClipStack）=====
    // 注意：每个子菜单的分隔符必须是独立实例——同一菜单项不能挂到两个父菜单上。
    let about = PredefinedMenuItem::about(app, Some(s.app_about), None)?;
    let services = PredefinedMenuItem::services(app, Some(s.app_services))?;
    let hide = PredefinedMenuItem::hide(app, Some(s.app_hide))?;
    let hide_others = PredefinedMenuItem::hide_others(app, Some(s.app_hide_others))?;
    let show_all = PredefinedMenuItem::show_all(app, Some(s.app_show_all))?;
    let quit = PredefinedMenuItem::quit(app, Some(s.app_quit))?;
    let sep_a1 = PredefinedMenuItem::separator(app)?;
    let sep_a2 = PredefinedMenuItem::separator(app)?;
    let sep_a3 = PredefinedMenuItem::separator(app)?;
    let app_sub = Submenu::with_items(
        app,
        "ClipStack", // 应用名固定，不翻译
        true,
        &[
            &about,
            &sep_a1,
            &services,
            &sep_a2,
            &hide,
            &hide_others,
            &show_all,
            &sep_a3,
            &quit,
        ],
    )?;

    // ===== Edit 菜单 =====
    let undo = PredefinedMenuItem::undo(app, Some(s.edit_undo))?;
    let redo = PredefinedMenuItem::redo(app, Some(s.edit_redo))?;
    let cut = PredefinedMenuItem::cut(app, Some(s.edit_cut))?;
    let copy = PredefinedMenuItem::copy(app, Some(s.edit_copy))?;
    let paste = PredefinedMenuItem::paste(app, Some(s.edit_paste))?;
    let select_all = PredefinedMenuItem::select_all(app, Some(s.edit_select_all))?;
    let sep_e1 = PredefinedMenuItem::separator(app)?;
    let sep_e2 = PredefinedMenuItem::separator(app)?;
    let edit_sub = Submenu::with_items(
        app,
        s.edit_title,
        true,
        &[&undo, &redo, &sep_e1, &cut, &copy, &paste, &sep_e2, &select_all],
    )?;

    // ===== Window 菜单 =====
    let minimize = PredefinedMenuItem::minimize(app, Some(s.win_minimize))?;
    let zoom = PredefinedMenuItem::maximize(app, Some(s.win_zoom))?;
    let close = PredefinedMenuItem::close_window(app, Some(s.win_close))?;
    let bring_all = PredefinedMenuItem::bring_all_to_front(app, Some(s.win_bring_all))?;
    let sep_w = PredefinedMenuItem::separator(app)?;
    let win_sub = Submenu::with_items(
        app,
        s.win_title,
        true,
        &[&minimize, &zoom, &sep_w, &close, &bring_all],
    )?;

    // ===== Help 菜单 =====
    // 自定义「ClipStack 帮助」项：点击跳转到前端帮助页（与托盘「帮助」行为一致）。
    let help_item = MenuItemBuilder::with_id("help", s.help_item).build(app)?;
    let help_sub = Submenu::with_items(app, s.help_title, true, &[&help_item])?;

    // ===== 组装菜单栏 =====
    let menu = Menu::new(app)?;
    menu.append(&app_sub)?;
    menu.append(&edit_sub)?;
    menu.append(&win_sub)?;
    menu.append(&help_sub)?;
    Ok(menu)
}
