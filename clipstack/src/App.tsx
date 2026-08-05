// P3 · 应用根组件：装配三栏布局，启动时加载历史并订阅实时事件。

import { useEffect } from "react";
import { useHistory } from "./store/history";
import { onClipboardChanged } from "./lib/tauri";
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

  // 加载历史 + 订阅剪贴板变更（实时追加到列表顶部）。
  useEffect(() => {
    void load();
    const unlisten = onClipboardChanged((item) => prepend(item));
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [load, prepend]);

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
