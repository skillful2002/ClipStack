// P3 · 设置视图（P6 增强）：主题、历史上限、开机自启、忽略应用管理。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import * as api from "../lib/tauri";
import { applyTheme, type Theme } from "../lib/theme";
import { useHistory } from "../store/history";

const THEMES: { key: Theme; label: string }[] = [
  { key: "light", label: "浅色" },
  { key: "dark", label: "深色" },
  { key: "system", label: "跟随系统" },
];

export function SettingsView() {
  const [apps, setApps] = useState<string[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(true);

  const [theme, setTheme] = useState<Theme>("light");
  const [maxHistory, setMaxHistory] = useState(1000);
  const [trayHistory, setTrayHistory] = useState(30);
  const [startup, setStartup] = useState(false);
  const [startupBusy, setStartupBusy] = useState(false);

  const setToast = useHistory((s) => s.setToast);

  const refreshApps = async () => {
    try {
      setApps(await api.getIgnoredApps());
    } catch (e) {
      setToast(`读取失败：${String(e)}`);
    } finally {
      setLoading(false);
    }
  };

  // 初始化：从设置表读取主题 / 上限，从 autostart 插件读取自启状态。
  useEffect(() => {
    void refreshApps();
    void (async () => {
      try {
        const settings = await api.getSettings();
        const t = settings.find((s) => s.key === "theme")?.value as Theme | undefined;
        if (t) {
          setTheme(t);
          applyTheme(t);
        }
        const mh = Number(settings.find((s) => s.key === "max_history")?.value);
        if (mh > 0) setMaxHistory(mh);
        const th = Number(settings.find((s) => s.key === "tray_history_count")?.value);
        if (th > 0) setTrayHistory(th);
      } catch (e) {
        setToast(`设置读取失败：${String(e)}`);
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

  const onThemeChange = async (t: Theme) => {
    setTheme(t);
    void applyTheme(t);
    try {
      await api.updateSetting("theme", t);
    } catch (e) {
      setToast(`主题保存失败：${String(e)}`);
    }
  };

  const onMaxHistoryChange = async (v: number) => {
    const clamped = Math.max(50, Math.min(50000, v || 1000));
    setMaxHistory(clamped);
    try {
      await api.updateSetting("max_history", String(clamped));
      setToast("已保存历史上限（下次启动时生效）");
    } catch (e) {
      setToast(`保存失败：${String(e)}`);
    }
  };

  const onTrayHistoryChange = async (v: number) => {
    const clamped = Math.max(1, Math.min(100, v || 30));
    setTrayHistory(clamped);
    try {
      await api.updateSetting("tray_history_count", String(clamped));
      setToast("已保存托盘历史条数（立即生效）");
    } catch (e) {
      setToast(`保存失败：${String(e)}`);
    }
  };

  const toggleStartup = async (next: boolean) => {
    setStartupBusy(true);
    try {
      if (next) await invoke("plugin:autostart|enable");
      else await invoke("plugin:autostart|disable");
      setStartup(next);
    } catch (e) {
      setToast(`自启设置失败：${String(e)}`);
    } finally {
      setStartupBusy(false);
    }
  };

  const addApp = async () => {
    const name = input.trim().toLowerCase();
    if (!name) return;
    try {
      await api.addIgnoredApp(name);
      setInput("");
      await refreshApps();
      setToast(`已忽略：${name}`);
    } catch (e) {
      setToast(`添加失败：${String(e)}`);
    }
  };

  return (
    <section className="settings-pane">
      <h2 className="settings-title">设置</h2>

      <div className="settings-card">
        <div className="settings-card-title">外观</div>
        <div className="settings-row">
          <span>主题</span>
          <div className="segmented">
            {THEMES.map((t) => (
              <button
                key={t.key}
                className={theme === t.key ? "active" : ""}
                onClick={() => void onThemeChange(t.key)}
              >
                {t.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-card-title">存储</div>
        <div className="settings-row">
          <span>历史上限（条）</span>
          <input
            type="number"
            min={50}
            max={50000}
            value={maxHistory}
            onChange={(e) => void onMaxHistoryChange(Number(e.target.value))}
          />
        </div>
        <p className="settings-hint">超出上限的最旧记录会被自动清理；调整将在下次启动时生效。</p>
        <div className="settings-row">
          <span>托盘菜单历史条数</span>
          <input
            type="number"
            min={1}
            max={100}
            value={trayHistory}
            onChange={(e) => void onTrayHistoryChange(Number(e.target.value))}
          />
        </div>
        <p className="settings-hint">托盘图标菜单中显示的最近历史条数（1–100，默认 30），保存后立即生效。</p>
      </div>

      <div className="settings-card">
        <div className="settings-card-title">开机自启</div>
        <div className="settings-row">
          <span>登录后自动启动 ClipStack</span>
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
        <div className="settings-card-title">忽略的应用</div>
        <p className="settings-hint">
          被忽略应用的复制内容不会被 ClipStack 捕获。名称以小写应用名匹配（如
          <code> safari</code>、<code> 终端</code>）。
        </p>

        <div className="settings-add">
          <input
            type="text"
            placeholder="输入应用名后回车添加…"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void addApp();
            }}
          />
          <button onClick={() => void addApp()}>添加</button>
        </div>

        <div className="settings-list">
          {loading ? (
            <div className="settings-empty">加载中…</div>
          ) : apps.length === 0 ? (
            <div className="settings-empty">暂无忽略应用</div>
          ) : (
            apps.map((a) => (
              <div key={a} className="settings-list-item">
                {a}
              </div>
            ))
          )}
        </div>
        <p className="settings-note">
          注：移除忽略项将在后续版本提供，当前可通过数据库直接清理。
        </p>
      </div>
    </section>
  );
}
