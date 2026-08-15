//! 后端 i18n（托盘菜单文案）。
//!
//! 设计：前端把「已解析」的语言（如 "zh-CN"/"en"）通过 `language-changed` 事件推给后端，
//! 后端存入 `MenuLang` 状态并在重建托盘菜单时使用；同时兜底读取 `language` 设置——
//! 若用户显式选择了具体语言，则从设置直接取值（启动首帧即正确），无需等待前端事件。
//!
//! 与前端 `src/lib/i18n/index.ts` 的 `detect` / `resolveFromBrowser` 保持一致：
//! 具体语言（zh-CN/zh-TW/en/ja/de/fr）直接使用；`system`/未知由前端解析后推送。

use std::sync::Mutex;

/// 已解析的具体语言（不含 "system"）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    ZhCn,
    ZhTw,
    En,
    Ja,
    De,
    Fr,
}

impl Lang {
    /// 从前端存储的 `language` 设置字符串解析具体语言；非具体语言（含 system/未知）返回 None。
    pub fn from_setting(s: &str) -> Option<Lang> {
        match s {
            "zh-CN" => Some(Lang::ZhCn),
            "zh-TW" => Some(Lang::ZhTw),
            "en" => Some(Lang::En),
            "ja" => Some(Lang::Ja),
            "de" => Some(Lang::De),
            "fr" => Some(Lang::Fr),
            _ => None,
        }
    }

    /// 解析（可能含 system 的）语言设置：具体语言直接用；system/未知 → 回退到
    /// `fallback`（前端推送的已解析语言，由调用方从 `MenuLang` 状态读取）。
    pub fn resolve(setting: &str, fallback: Lang) -> Lang {
        Lang::from_setting(setting).unwrap_or(fallback)
    }
}

/// 托盘菜单当前使用的语言状态（由前端 `language-changed` 事件更新；缺省英文）。
pub struct MenuLang(pub Mutex<Lang>);

impl Default for MenuLang {
    fn default() -> Self {
        MenuLang(Mutex::new(Lang::En))
    }
}

/// 托盘菜单「主界面」标签。
pub fn tray_open_main(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "主界面",
        Lang::ZhTw => "主介面",
        Lang::En => "Main Window",
        Lang::Ja => "メインウィンドウ",
        Lang::De => "Hauptfenster",
        Lang::Fr => "Fenêtre principale",
    }
}

/// 托盘菜单「锁定」标签（仅在已设主密码且当前未锁定时显示）。
pub fn tray_lock(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "锁定",
        Lang::ZhTw => "鎖定",
        Lang::En => "Lock",
        Lang::Ja => "ロック",
        Lang::De => "Sperren",
        Lang::Fr => "Verrouiller",
    }
}

/// 托盘菜单「设置」标签。
pub fn tray_settings(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "设置",
        Lang::ZhTw => "設定",
        Lang::En => "Settings",
        Lang::Ja => "設定",
        Lang::De => "Einstellungen",
        Lang::Fr => "Paramètres",
    }
}

/// 托盘菜单「关于系统」标签。
pub fn tray_about(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "关于系统",
        Lang::ZhTw => "關於系統",
        Lang::En => "About",
        Lang::Ja => "バージョン情報",
        Lang::De => "Über",
        Lang::Fr => "À propos",
    }
}

/// 托盘菜单「帮助」标签。
pub fn tray_help(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "帮助",
        Lang::ZhTw => "說明",
        Lang::En => "Help",
        Lang::Ja => "ヘルプ",
        Lang::De => "Hilfe",
        Lang::Fr => "Aide",
    }
}

/// 托盘菜单「共享」开关标签（带状态圆点图标）。
pub fn tray_share(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "共享",
        Lang::ZhTw => "共享",
        Lang::En => "Share",
        Lang::Ja => "共有",
        Lang::De => "Teilen",
        Lang::Fr => "Partager",
    }
}

/// 托盘「共享」开启时的状态提示（显示状态并提示点击切换）。
pub fn tray_share_on(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "已开启 · 点击关闭",
        Lang::ZhTw => "已開啟 · 點擊關閉",
        Lang::En => "On · click to stop",
        Lang::Ja => "オン · クリックで停止",
        Lang::De => "Ein · zum Stoppen klicken",
        Lang::Fr => "Activé · cliquer pour arrêter",
    }
}

/// 托盘「共享」关闭时的状态提示（显示状态并提示点击切换）。
pub fn tray_share_off(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "已关闭 · 点击开启",
        Lang::ZhTw => "已關閉 · 點擊開啟",
        Lang::En => "Off · click to start",
        Lang::Ja => "オフ · クリックで開始",
        Lang::De => "Aus · zum Starten klicken",
        Lang::Fr => "Désactivé · cliquer pour démarrer",
    }
}

/// 托盘菜单「退出」标签（PredefinedMenuItem::quit 的自定义文案）。
pub fn tray_quit(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "退出 ClipStack",
        Lang::ZhTw => "退出 ClipStack",
        Lang::En => "Quit ClipStack",
        Lang::Ja => "ClipStack を終了",
        Lang::De => "ClipStack beenden",
        Lang::Fr => "Quitter ClipStack",
    }
}

/// 托盘菜单「已锁定」占位项（主密码已设置且当前处于锁定态时显示，点击解锁）。
pub fn tray_locked(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "🔒 已锁定（点击解锁）",
        Lang::ZhTw => "🔒 已鎖定（點擊解鎖）",
        Lang::En => "🔒 Locked (click to unlock)",
        Lang::Ja => "🔒 ロック中（クリックで解除）",
        Lang::De => "🔒 Gesperrt (zum Entsperren klicken)",
        Lang::Fr => "🔒 Verrouillé (cliquez pour déverrouiller)",
    }
}

/// 托盘菜单空状态（暂无历史记录）标签。
pub fn tray_empty(lang: Lang) -> &'static str {
    match lang {
        Lang::ZhCn => "（暂无历史记录）",
        Lang::ZhTw => "（暫無歷史記錄）",
        Lang::En => "(No history yet)",
        Lang::Ja => "（履歴はまだありません）",
        Lang::De => "(Noch keine Verläufe)",
        Lang::Fr => "(Aucun historique)",
    }
}

/// 顶部原生菜单栏（macOS 菜单栏 App / Edit / Window / Help）的全部文案，跟随 `lang`。
///
/// 背景：Tauri 在 macOS 上用 `muda` 自动生成顶部菜单，但 `muda` 把预定义项
/// （Copy / Quit / About …）写死为英文、不跟随系统语言；本结构体提供本地化文案，
/// 由 `menu.rs` 传入 `PredefinedMenuItem` 覆盖默认英文，使菜单栏跟随应用语言
/// （默认 system = 跟随系统）。
#[derive(Clone, Debug)]
pub struct NativeMenu {
    // App 菜单（标题固定为应用名 "ClipStack"，不翻译）
    pub app_about: &'static str,
    pub app_services: &'static str,
    pub app_hide: &'static str,
    pub app_hide_others: &'static str,
    pub app_show_all: &'static str,
    pub app_quit: &'static str,
    // Edit 菜单
    pub edit_title: &'static str,
    pub edit_undo: &'static str,
    pub edit_redo: &'static str,
    pub edit_cut: &'static str,
    pub edit_copy: &'static str,
    pub edit_paste: &'static str,
    pub edit_select_all: &'static str,
    // Window 菜单
    pub win_title: &'static str,
    pub win_minimize: &'static str,
    pub win_zoom: &'static str,
    pub win_close: &'static str,
    pub win_bring_all: &'static str,
    // Help 菜单
    pub help_title: &'static str,
    pub help_item: &'static str,
}

/// 返回对应语言的顶部菜单栏全部文案。
pub fn native_menu(lang: Lang) -> NativeMenu {
    match lang {
        Lang::ZhCn => NativeMenu {
            app_about: "关于 ClipStack",
            app_services: "服务",
            app_hide: "隐藏 ClipStack",
            app_hide_others: "隐藏其他",
            app_show_all: "全部显示",
            app_quit: "退出 ClipStack",
            edit_title: "编辑",
            edit_undo: "撤销",
            edit_redo: "重做",
            edit_cut: "剪切",
            edit_copy: "拷贝",
            edit_paste: "粘贴",
            edit_select_all: "全选",
            win_title: "窗口",
            win_minimize: "最小化",
            win_zoom: "缩放",
            win_close: "关闭窗口",
            win_bring_all: "全部置于顶层",
            help_title: "帮助",
            help_item: "ClipStack 帮助",
        },
        Lang::ZhTw => NativeMenu {
            app_about: "關於 ClipStack",
            app_services: "服務",
            app_hide: "隱藏 ClipStack",
            app_hide_others: "隱藏其他",
            app_show_all: "全部顯示",
            app_quit: "退出 ClipStack",
            edit_title: "編輯",
            edit_undo: "復原",
            edit_redo: "重做",
            edit_cut: "剪下",
            edit_copy: "拷貝",
            edit_paste: "貼上",
            edit_select_all: "全部選取",
            win_title: "視窗",
            win_minimize: "最小化",
            win_zoom: "縮放",
            win_close: "關閉視窗",
            win_bring_all: "全部置於最前方",
            help_title: "說明",
            help_item: "ClipStack 說明",
        },
        Lang::En => NativeMenu {
            app_about: "About ClipStack",
            app_services: "Services",
            app_hide: "Hide ClipStack",
            app_hide_others: "Hide Others",
            app_show_all: "Show All",
            app_quit: "Quit ClipStack",
            edit_title: "Edit",
            edit_undo: "Undo",
            edit_redo: "Redo",
            edit_cut: "Cut",
            edit_copy: "Copy",
            edit_paste: "Paste",
            edit_select_all: "Select All",
            win_title: "Window",
            win_minimize: "Minimize",
            win_zoom: "Zoom",
            win_close: "Close Window",
            win_bring_all: "Bring All to Front",
            help_title: "Help",
            help_item: "ClipStack Help",
        },
        Lang::Ja => NativeMenu {
            app_about: "ClipStack について",
            app_services: "サービス",
            app_hide: "ClipStack を隠す",
            app_hide_others: "他を隠す",
            app_show_all: "すべてを表示",
            app_quit: "ClipStack を終了",
            edit_title: "編集",
            edit_undo: "取り消し",
            edit_redo: "やり直し",
            edit_cut: "切り取り",
            edit_copy: "コピー",
            edit_paste: "貼り付け",
            edit_select_all: "すべてを選択",
            win_title: "ウィンドウ",
            win_minimize: "最小化",
            win_zoom: "ズーム",
            win_close: "ウィンドウを閉じる",
            win_bring_all: "すべてを手前に",
            help_title: "ヘルプ",
            help_item: "ClipStack ヘルプ",
        },
        Lang::De => NativeMenu {
            app_about: "Über ClipStack",
            app_services: "Dienste",
            app_hide: "ClipStack ausblenden",
            app_hide_others: "Andere ausblenden",
            app_show_all: "Alle anzeigen",
            app_quit: "ClipStack beenden",
            edit_title: "Bearbeiten",
            edit_undo: "Rückgängig",
            edit_redo: "Wiederholen",
            edit_cut: "Ausschneiden",
            edit_copy: "Kopieren",
            edit_paste: "Einfügen",
            edit_select_all: "Alles auswählen",
            win_title: "Fenster",
            win_minimize: "Minimieren",
            win_zoom: "Vergrößern",
            win_close: "Fenster schließen",
            win_bring_all: "Alle nach vorne",
            help_title: "Hilfe",
            help_item: "ClipStack-Hilfe",
        },
        Lang::Fr => NativeMenu {
            app_about: "À propos de ClipStack",
            app_services: "Services",
            app_hide: "Masquer ClipStack",
            app_hide_others: "Masquer les autres",
            app_show_all: "Tout afficher",
            app_quit: "Quitter ClipStack",
            edit_title: "Édition",
            edit_undo: "Annuler",
            edit_redo: "Rétablir",
            edit_cut: "Couper",
            edit_copy: "Copier",
            edit_paste: "Coller",
            edit_select_all: "Tout sélectionner",
            win_title: "Fenêtre",
            win_minimize: "Réduire",
            win_zoom: "Zoom",
            win_close: "Fermer la fenêtre",
            win_bring_all: "Tout ramener au premier plan",
            help_title: "Aide",
            help_item: "Aide ClipStack",
        },
    }
}

