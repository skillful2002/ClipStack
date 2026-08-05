// P3 · 设置视图：忽略应用管理（来源应用不再被捕获）。

import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import { useHistory } from "../store/history";

export function SettingsView() {
  const [apps, setApps] = useState<string[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(true);
  const setToast = useHistory((s) => s.setToast);

  const refresh = async () => {
    try {
      setApps(await api.getIgnoredApps());
    } catch (e) {
      setToast(`读取失败：${String(e)}`);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const add = async () => {
    const name = input.trim().toLowerCase();
    if (!name) return;
    try {
      await api.addIgnoredApp(name);
      setInput("");
      await refresh();
      setToast(`已忽略：${name}`);
    } catch (e) {
      setToast(`添加失败：${String(e)}`);
    }
  };

  return (
    <section className="settings-pane">
      <h2 className="settings-title">设置</h2>

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
              if (e.key === "Enter") void add();
            }}
          />
          <button onClick={() => void add()}>添加</button>
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
