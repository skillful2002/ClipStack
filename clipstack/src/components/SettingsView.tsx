// P3 · 设置视图（P6 增强）：主题、语言、历史上限、开机自启、忽略应用管理。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import * as api from "../lib/tauri";
import { applyTheme, type Theme } from "../lib/theme";
import { SecuritySettings } from "./SecuritySettings";
import { SaveTypesSettings } from "./SaveTypesSettings";
import { RetentionSettings } from "./RetentionSettings";
import { LanSettings } from "./LanSettings";
import { useHistory } from "../store/history";
import { useT, useI18nStore, LANGUAGE_OPTIONS, type Language } from "../lib/i18n";

const THEMES: { key: Theme }[] = [
  { key: "light" },
  { key: "dark" },
  { key: "system" },
];

export function SettingsView() {
  const t = useT();
  const setLang = useI18nStore((s) => s.setLang);

  const [apps, setApps] = useState<string[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(true);

  const [systemApps, setSystemApps] = useState<string[]>([]);
  const [sysLoading, setSysLoading] = useState(true);
  const [selectedSys, setSelectedSys] = useState("");

  const [theme, setTheme] = useState<Theme>("system");
  const [language, setLanguage] = useState<Language>("system");
  const [maxHistory, setMaxHistory] = useState(1000);
  const [trayHistory, setTrayHistory] = useState(30);
  const [startup, setStartup] = useState(false);
  const [startupBusy, setStartupBusy] = useState(false);

  const setToast = useHistory((s) => s.setToast);

  const refreshApps = async () => {
    try {
      setApps(await api.getIgnoredApps());
    } catch (e) {
      setToast(t("toast.readFailed", { error: String(e) }));
    } finally {
      setLoading(false);
    }
  };

  // 加载系统中已安装应用，供「从系统选择」下拉使用。
  const loadSystemApps = async () => {
    try {
      setSystemApps(await api.listInstalledApps());
    } catch {
      setSystemApps([]);
    } finally {
      setSysLoading(false);
    }
  };

  // 初始化：从设置表读取主题 / 语言 / 上限，从 autostart 插件读取自启状态。
  useEffect(() => {
    void refreshApps();
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
        if (mh > 0) setMaxHistory(mh);
        const thc = Number(settings.find((s) => s.key === "tray_history_count")?.value);
        if (thc > 0) setTrayHistory(thc);
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
    void loadSystemApps();
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

  const onMaxHistoryChange = async (v: number) => {
    const clamped = Math.max(50, Math.min(50000, v || 1000));
    setMaxHistory(clamped);
    try {
      await api.updateSetting("max_history", String(clamped));
      setToast(t("toast.maxHistorySaved"));
    } catch (e) {
      setToast(t("toast.saveFailed", { error: String(e) }));
    }
  };

  const onTrayHistoryChange = async (v: number) => {
    const clamped = Math.max(1, Math.min(100, v || 30));
    setTrayHistory(clamped);
    try {
      await api.updateSetting("tray_history_count", String(clamped));
      setToast(t("toast.trayHistorySaved"));
    } catch (e) {
      setToast(t("toast.saveFailed", { error: String(e) }));
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

  const addApp = async () => {
    const name = input.trim();
    if (!name) return;
    try {
      await api.addIgnoredApp(name);
      setInput("");
      await refreshApps();
      setToast(t("toast.ignoredAdded", { name }));
    } catch (e) {
      setToast(t("toast.addFailed", { error: String(e) }));
    }
  };

  // 从「系统已安装应用」下拉添加忽略项（使用系统原始显示名，保留中文/大小写）。
  const addSelectedApp = async () => {
    const name = selectedSys.trim();
    if (!name) return;
    try {
      await api.addIgnoredApp(name);
      setSelectedSys("");
      await refreshApps();
      setToast(t("toast.ignoredAdded", { name }));
    } catch (e) {
      setToast(t("toast.addFailed", { error: String(e) }));
    }
  };

  const removeApp = async (name: string) => {
    try {
      await api.removeIgnoredApp(name);
      await refreshApps();
      setToast(t("toast.ignoredRemoved", { name }));
    } catch (e) {
      setToast(t("toast.removeFailed", { error: String(e) }));
    }
  };

  const renderLangLabel = (key: Language): string =>
    key === "system" ? t("language.system") : (LANGUAGE_OPTIONS.find((o) => o.key === key)?.native ?? key);

  return (
    <section className="settings-pane">
      <h2 className="settings-title">{t("settings.title")}</h2>

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
            min={50}
            max={50000}
            value={maxHistory}
            onChange={(e) => void onMaxHistoryChange(Number(e.target.value))}
          />
        </div>
        <p className="settings-hint">{t("settings.maxHistoryHint")}</p>
        <div className="settings-row">
          <span>{t("settings.trayHistory")}</span>
          <input
            type="number"
            min={1}
            max={100}
            value={trayHistory}
            onChange={(e) => void onTrayHistoryChange(Number(e.target.value))}
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

      <div className="settings-card">
        <div className="settings-card-title">{t("settings.ignoredApps")}</div>
        <p className="settings-hint">{t("settings.ignoredAppsHint")}</p>

        <div className="settings-add">
          <input
            type="text"
            placeholder={t("settings.appInputPlaceholder")}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void addApp();
            }}
          />
          <button onClick={() => void addApp()}>{t("settings.add")}</button>
        </div>

        <div className="settings-add">
          <select
            className="app-select"
            value={selectedSys}
            disabled={sysLoading}
            onChange={(e) => setSelectedSys(e.target.value)}
          >
            <option value="">
              {sysLoading
                ? t("settings.sysLoading")
                : systemApps.length === 0
                ? t("settings.sysUnavailable")
                : t("settings.sysSelect")}
            </option>
            {systemApps
              .filter((n) => !apps.includes(n))
              .map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
          </select>
          <button onClick={() => void addSelectedApp()} disabled={!selectedSys}>
            {t("settings.addSelected")}
          </button>
        </div>

        <div className="settings-list">
          {loading ? (
            <div className="settings-empty">{t("settings.loading")}</div>
          ) : apps.length === 0 ? (
            <div className="settings-empty">{t("settings.noIgnored")}</div>
          ) : (
            apps.map((a) => (
              <div key={a} className="settings-list-item">
                <span className="item-name">{a}</span>
                <button
                  className="item-remove"
                  title={t("settings.removeIgnoredTitle")}
                  onClick={() => void removeApp(a)}
                >
                  ×
                </button>
              </div>
            ))
          )}
        </div>
        <p className="settings-note">{t("settings.ignoredNote")}</p>
      </div>

      <SecuritySettings />

      <LanSettings />
    </section>
  );
}
