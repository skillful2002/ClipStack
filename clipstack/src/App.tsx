// P3 · 应用根组件：装配三栏布局，启动时加载历史并订阅实时事件。

import { useEffect } from "react";
import { useHistory } from "./store/history";
import {
  onClipboardChanged,
  onShowView,
  onTrayCopied,
  getSettings,
} from "./lib/tauri";
import { applyTheme, type Theme } from "./lib/theme";
import { Sidebar } from "./components/Sidebar";
import { HistoryList } from "./components/HistoryList";
import { DetailPanel } from "./components/DetailPanel";
import { SettingsView } from "./components/SettingsView";
import { TrashPlaceholder } from "./components/TrashPlaceholder";
import "./styles/app.css";

export default function App() {
  const load = useHistory((s) => s.load);
  const prepend = useHistory((s) => s.prepend);
  const view = useHistory((s) => s.view);
  const toast = useHistory((s) => s.toast);
  const setToast = useHistory((s) => s.setToast);
  const setView = useHistory((s) => s.setView);
  const select = useHistory((s) => s.select);

  // 启动：加载历史、应用已保存主题、订阅剪贴板变更（实时追加到列表顶部）。
  useEffect(() => {
    void load();
    void (async () => {
      try {
        const settings = await getSettings();
        const t = settings.find((s) => s.key === "theme")?.value as Theme | undefined;
        if (t) applyTheme(t);
      } catch {
        /* 无主题设置时保持浅色默认 */
      }
    })();
    const unlisten = onClipboardChanged((item) => prepend(item));
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [load, prepend]);

  // P4：托盘菜单 / 全局快捷键触发的视图切换与复制回执。
  useEffect(() => {
    const p1 = onShowView((v) => setView(v === "settings" ? "settings" : "main"));
    const p2 = onTrayCopied((id) => {
      select(id);
      setToast("已复制到剪贴板");
    });
    return () => {
      void p1.then((fn) => fn());
      void p2.then((fn) => fn());
    };
  }, [setView, select, setToast]);

  // P5：⌘K / Ctrl+K 聚焦搜索框。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        document.getElementById("clipstack-search")?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 提示自动消失。
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 2600);
    return () => clearTimeout(t);
  }, [toast, setToast]);

  return (
    <div className="app">
      <Sidebar />
      <main className="app-main">
        {view === "main" && (
          <>
            <HistoryList />
            <DetailPanel />
          </>
        )}
        {view === "settings" && <SettingsView />}
        {view === "trash" && <TrashPlaceholder />}
      </main>
      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}
