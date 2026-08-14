// 安全 · 忽略的应用：管理不被捕获剪贴板内容的应用名单。

import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import { useHistory } from "../store/history";
import { useT } from "../lib/i18n";

export function IgnoredAppsSettings() {
  const t = useT();
  const setToast = useHistory((s) => s.setToast);

  const [apps, setApps] = useState<string[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(true);

  const [systemApps, setSystemApps] = useState<string[]>([]);
  const [sysLoading, setSysLoading] = useState(true);
  const [selectedSys, setSelectedSys] = useState("");

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

  useEffect(() => {
    void refreshApps();
    void loadSystemApps();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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

  return (
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
        <button className="sec-btn outline" onClick={() => void addApp()}>{t("settings.add")}</button>
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
        <button className="sec-btn outline" onClick={() => void addSelectedApp()} disabled={!selectedSys}>
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
  );
}
