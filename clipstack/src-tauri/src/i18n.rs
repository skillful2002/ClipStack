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

