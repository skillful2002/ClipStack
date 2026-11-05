// P3 · 设置视图（P6 增强）：主题、语言、历史上限、开机自启、忽略应用管理。

import { useEffect, useState, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import * as api from "../lib/tauri";
import { applyTheme, type Theme } from "../lib/theme";
import {
  DEFAULT_MAX_HISTORY,
  DEFAULT_TRAY_HISTORY,
  MAX_MAX_HISTORY,
  MIN_MAX_HISTORY,
  MAX_TRAY_HISTORY,
  MIN_TRAY_HISTORY,
  clampMaxHistory,
  clampTrayHistory,
} from "../lib/limits";
import { SecuritySettings } from "./SecuritySettings";
import { SaveTypesSettings } from "./SaveTypesSettings";
import { RetentionSettings } from "./RetentionSettings";
import { LanSettings } from "./LanSettings";
import { IgnoredAppsSettings } from "./IgnoredAppsSettings";
import { useHistory } from "../store/history";
import { useT, useI18nStore, LANGUAGE_OPTIONS, type Language } from "../lib/i18n";

type SettingsTab = "general" | "security" | "sharing";

const THEMES: { key: Theme }[] = [
  { key: "light" },
  { key: "dark" },
  { key: "system" },
];

export function SettingsView() {
  const t = useT();
  const setLang = useI18nStore((s) => s.setLang);

  const [tab, setTab] = useState<SettingsTab>("general");

  const [theme, setTheme] = useState<Theme>("system");
  const [language, setLanguage] = useState<Language>("system");
  const [maxHistory, setMaxHistory] = useState(DEFAULT_MAX_HISTORY);
  const [trayHistory, setTrayHistory] = useState(DEFAULT_TRAY_HISTORY);
  // 输入框草稿：只在失焦 / 回车时提交一次。历史上限保存后后端会立即按新值清理，
  // 若逐字符保存，输入 500 的中间态「5」会被夹成 50 并触发一次不可恢复的批量删除。
  const [maxDraft, setMaxDraft] = useState(String(DEFAULT_MAX_HISTORY));
  const [trayDraft, setTrayDraft] = useState(String(DEFAULT_TRAY_HISTORY));
  const [startup, setStartup] = useState(false);
  const [startupBusy, setStartupBusy] = useState(false);

  const setToast = useHistory((s) => s.setToast);

  // 初始化：从设置表读取主题 / 语言 / 上限，从 autostart 插件读取自启状态。
  useEffect(() => {
    void (async () => {
      try {
        const settings = await api.getSettings();
        const th = settings.find((s) => s.key === "theme")?.value as Theme | undefined;
        if (th) {
          setTheme(th);
          applyTheme(th);
        }
        const lg = settings.find((s) => s.key === "language")?.value as Language | undefined;
        if (lg) {
          setLanguage(lg);
          setLang(lg);
        }
        const mh = Number(settings.find((s) => s.key === "max_history")?.value);
        if (mh > 0) {
          const v = clampMaxHistory(mh);
          setMaxHistory(v);
          setMaxDraft(String(v));
        }
        const thc = Number(settings.find((s) => s.key === "tray_history_count")?.value);
        if (thc > 0) {
          const v = clampTrayHistory(thc);
          setTrayHistory(v);
          setTrayDraft(String(v));
        }
      } catch (e) {
        setToast(t("toast.settingsReadFailed", { error: String(e) }));
      }
      try {
        setStartup((await invoke<boolean>("plugin:autostart|is_enabled")) ?? false);
      } catch {
        /* 插件不可用时静默 */
      }
    })();
    // 系统主题的实时跟随已由 App 启动时注册的 watchSystemTheme 统一处理，
    // 此处无需再监听 matchMedia，避免重复订阅与视图关闭后失效的问题。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onThemeChange = async (th: Theme) => {
    setTheme(th);
    void applyTheme(th);
    try {
      await api.updateSetting("theme", th);
    } catch (e) {
      setToast(t("toast.themeSaveFailed", { error: String(e) }));
    }
  };

  const onLanguageChange = async (lg: Language) => {
    setLanguage(lg);
    setLang(lg);
    try {
      await api.updateSetting("language", lg);
    } catch (e) {
      setToast(t("toast.saveFailed", { error: String(e) }));
    }
  };

  // 历史上限 / 托盘条数：只在失焦或回车时提交一次（保存后后端立即清理超量记录，
  // 逐字符保存会让中间态小值触发误删）。非法输入回退到已保存值，不写库。
  const commitMaxHistory = async () => {
    const raw = Number(maxDraft);
    // 空 / 非数字 / 非正数视为「放弃修改」，回退已保存值，不写库、不触发清理。
    const next =
      maxDraft.trim() === "" || !Number.isFinite(raw) || raw <= 0
        ? maxHistory
        : clampMaxHistory(raw);
    setMaxDraft(String(next));
    if (next === maxHistory) return;
    setMaxHistory(next);
    try {
      await api.updateSetting("max_history", String(next));
      setToast(t("toast.maxHistorySaved"));
    } catch (e) {
      setMaxDraft(String(maxHistory));
      setToast(t("toast.saveFailed", { error: String(e) }));
    }
  };

  const commitTrayHistory = async () => {
    const raw = Number(trayDraft);
    const next =
      trayDraft.trim() === "" || !Number.isFinite(raw) || raw <= 0
        ? trayHistory
        : clampTrayHistory(raw);
    setTrayDraft(String(next));
    if (next === trayHistory) return;
    setTrayHistory(next);
    try {
      await api.updateSetting("tray_history_count", String(next));
      setToast(t("toast.trayHistorySaved"));
    } catch (e) {
      setTrayDraft(String(trayHistory));
      setToast(t("toast.saveFailed", { error: String(e) }));
    }
  };

  /** 回车 = 提交并失焦；Esc = 放弃草稿并失焦。 */
  const onNumberKeyDown = (e: KeyboardEvent<HTMLInputElement>, revert: () => void) => {
    if (e.key === "Enter") {
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      revert();
      e.currentTarget.blur();
    }
  };

  const toggleStartup = async (next: boolean) => {
    setStartupBusy(true);
    try {
      if (next) await invoke("plugin:autostart|enable");
      else await invoke("plugin:autostart|disable");
      setStartup(next);
    } catch (e) {
      setToast(t("toast.startupFailed", { error: String(e) }));
    } finally {
      setStartupBusy(false);
    }
  };

  const renderLangLabel = (key: Language): string =>
    key === "system" ? t("language.system") : (LANGUAGE_OPTIONS.find((o) => o.key === key)?.native ?? key);

  return (
    <section className="settings-pane">
      <h2 className="settings-title">{t("settings.title")}</h2>

      <div className="settings-tabs">
        <button
          className={tab === "general" ? "settings-tab active" : "settings-tab"}
          onClick={() => setTab("general")}
        >
          {t("settings.tabGeneral")}
        </button>
        <button
          className={tab === "security" ? "settings-tab active" : "settings-tab"}
          onClick={() => setTab("security")}
        >
          {t("settings.tabSecurity")}
        </button>
        <button
          className={tab === "sharing" ? "settings-tab active" : "settings-tab"}
          onClick={() => setTab("sharing")}
        >
          {t("settings.tabSharing")}
        </button>
      </div>

      <div className="settings-tab-content">
        {tab === "general" && (
        <>
          <div className="settings-card">
            <div className="settings-card-title">{t("settings.appearance")}</div>
            <div className="settings-row">
              <span>{t("settings.theme")}</span>
              <div className="segmented">
                {THEMES.map((th) => (
                  <button
                    key={th.key}
                    className={theme === th.key ? "active" : ""}
                    onClick={() => void onThemeChange(th.key)}
                  >
                    {t(`theme.${th.key}`)}
                  </button>
                ))}
              </div>
            </div>
            <div className="settings-row">
              <span>{t("settings.language")}</span>
              <select
                className="app-select"
                value={language}
                onChange={(e) => void onLanguageChange(e.target.value as Language)}
              >
                {LANGUAGE_OPTIONS.map((o) => (
                  <option key={o.key} value={o.key}>
                    {renderLangLabel(o.key)}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="settings-card">
            <div className="settings-card-title">{t("settings.storage")}</div>
            <div className="settings-row">
              <span>{t("settings.maxHistory")}</span>
              <input
                type="number"
                min={MIN_MAX_HISTORY}
                max={MAX_MAX_HISTORY}
                value={maxDraft}
                onChange={(e) => setMaxDraft(e.target.value)}
                onBlur={() => void commitMaxHistory()}
                onKeyDown={(e) => onNumberKeyDown(e, () => setMaxDraft(String(maxHistory)))}
              />
            </div>
            <p className="settings-hint">{t("settings.maxHistoryHint")}</p>
            <div className="settings-row">
              <span>{t("settings.trayHistory")}</span>
              <input
                type="number"
                min={MIN_TRAY_HISTORY}
                max={MAX_TRAY_HISTORY}
                value={trayDraft}
                onChange={(e) => setTrayDraft(e.target.value)}
                onBlur={() => void commitTrayHistory()}
                onKeyDown={(e) => onNumberKeyDown(e, () => setTrayDraft(String(trayHistory)))}
              />
            </div>
            <p className="settings-hint">{t("settings.trayHistoryHint")}</p>

            <RetentionSettings />
            <SaveTypesSettings />
          </div>

          <div className="settings-card">
            <div className="settings-card-title">{t("settings.startup")}</div>
            <div className="settings-row">
              <span>{t("settings.startupLabel")}</span>
              <label className="switch">
                <input
                  type="checkbox"
                  checked={startup}
                  disabled={startupBusy}
                  onChange={(e) => void toggleStartup(e.target.checked)}
                />
                <span className="slider" />
              </label>
            </div>
          </div>
        </>
      )}

      {tab === "security" && (
        <>
          <IgnoredAppsSettings />
          <SecuritySettings />
        </>
      )}

      {tab === "sharing" && <LanSettings />}
      </div>
    </section>
  );
}
