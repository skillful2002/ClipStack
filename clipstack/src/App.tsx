// P3 · 应用根组件：装配三栏布局，启动时加载历史并订阅实时事件。

import { useEffect, useRef } from "react";
import { emit } from "@tauri-apps/api/event";
import { useHistory, type Category } from "./store/history";
import {
  onClipboardChanged,
  onShowView,
  onTrayCopied,
  getSettings,
  wasFirstRun,
} from "./lib/tauri";
import { useItemActions } from "./lib/actions";
import { applyTheme, watchSystemTheme, type Theme } from "./lib/theme";
import { useI18nStore, getResolvedLang, translate, type Language } from "./lib/i18n";
import { Sidebar } from "./components/Sidebar";
import { HistoryList } from "./components/HistoryList";
import { DetailPanel } from "./components/DetailPanel";
import { SettingsView } from "./components/SettingsView";
import { TrashView } from "./components/TrashView";
import { TrashDetail } from "./components/TrashDetail";
import { AboutView } from "./components/AboutView";
import "./styles/app.css";

export default function App() {
  const load = useHistory((s) => s.load);
  const prepend = useHistory((s) => s.prepend);
  const view = useHistory((s) => s.view);
  const loadTrash = useHistory((s) => s.loadTrash);
  const toast = useHistory((s) => s.toast);
  const setToast = useHistory((s) => s.setToast);
  const setView = useHistory((s) => s.setView);
  const select = useHistory((s) => s.select);
  const { copy, pin, fav, del } = useItemActions();

  // 启动：加载历史、应用已保存主题、订阅系统主题变化、订阅剪贴板变更。
  const themeUnlistenRef = useRef<() => void>(() => {});
  useEffect(() => {
    void load();
    void (async () => {
      try {
        const settings = await getSettings();
        const t = settings.find((s) => s.key === "theme")?.value as Theme | undefined;
        // 默认跟随系统（无已保存主题时）。
        await applyTheme(t ?? "system");
        // 语言：默认跟随系统（无已保存语言时）。
        const lang = settings.find((s) => s.key === "language")?.value as Language | undefined;
        if (lang) useI18nStore.getState().setLang(lang);
      } catch {
        /* 读取失败时退化为跟随系统 */
        await applyTheme("system");
      }
      // 订阅系统主题变化：仅在「跟随系统」时实时跟随 OS 切换。
      try {
        themeUnlistenRef.current = await watchSystemTheme();
      } catch {
        /* 不支持时静默 */
      }
      // 首启处理：窗口的显示/隐藏与首启标记写入已在 Rust setup 阶段同步完成；
      // 此处仅读取标志，首次运行时自动进入设置页引导配置。
      try {
        const isFirst = await wasFirstRun();
        if (isFirst) setView("settings");
      } catch {
        /* 读取失败时静默，不影响启动 */
      }
    })();
    const unlisten = onClipboardChanged((item) => prepend(item));
    return () => {
      void unlisten.then((fn) => fn());
      themeUnlistenRef.current();
    };
  }, [load, prepend]);

  // P4：托盘菜单 / 全局快捷键触发的视图切换与复制回执。
  useEffect(() => {
    const p1 = onShowView((v) =>
      setView(v === "settings" ? "settings" : v === "about" ? "about" : "main"),
    );
    const p2 = onTrayCopied((id) => {
      select(id);
      setToast(translate(getResolvedLang(), "toast.copied"));
    });
    return () => {
      void p1.then((fn) => fn());
      void p2.then((fn) => fn());
    };
  }, [setView, select, setToast]);

  // P5：⌘/ / Ctrl+/ 聚焦搜索框（⌘K 常被其它程序全局占用且优先级更高，改用 ⌘/）。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "/") {
        e.preventDefault();
        document.getElementById("clipstack-search")?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 快捷键：分类切换（⌘1-⌘6）、设置（⌘,）、回收站（⌘⇧T），以及主界面条目操作
  // （⏎ 复制 / P 置顶 / F 收藏 / ⌫·Del 删除）。输入框或按钮聚焦时跳过单键动作，避免误触。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const typing =
        !!el &&
        (el.tagName === "INPUT" ||
          el.tagName === "TEXTAREA" ||
          el.isContentEditable);
      const k = e.key.toLowerCase();

      // 修饰键组合：输入框中也允许，便于随时切换分类 / 视图。
      if (e.metaKey || e.ctrlKey) {
        if (k >= "1" && k <= "6") {
          e.preventDefault();
          const map: Record<string, Category> = {
            "1": "all", "2": "text", "3": "link",
            "4": "code", "5": "image", "6": "file",
          };
          const st = useHistory.getState();
          st.setCategory(map[k]);
          st.setView("main");
          return;
        }
        if (k === ",") {
          e.preventDefault();
          useHistory.getState().setView("settings");
          return;
        }
        if (e.shiftKey && k === "t") {
          e.preventDefault();
          useHistory.getState().setView("trash");
          return;
        }
        return; // 其它组合（如 ⌘/ 聚焦搜索）交由其它监听处理
      }

      // 单键动作：仅主界面、且未聚焦输入框 / 按钮时生效。
      if (typing || (el && el.tagName === "BUTTON")) return;
      const st = useHistory.getState();
      if (st.view !== "main") return;
      const item = st.items.find((i) => i.id === st.selectedId);
      if (!item) return;
      if (e.key === "Enter") {
        e.preventDefault();
        void copy(item);
      } else if (k === "p") {
        e.preventDefault();
        void pin(item);
      } else if (k === "f") {
        e.preventDefault();
        void fav(item);
      } else if (e.key === "Backspace" || e.key === "Delete") {
        e.preventDefault();
        void del(item);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [copy, pin, fav, del]);

  // 进入回收站视图时拉取已删除条目。
  useEffect(() => {
    if (view === "trash") void loadTrash();
  }, [view, loadTrash]);

  // 语言变化（启动加载 / 设置切换）时，将已解析语言推送给后端，
  // 使托盘菜单文案与界面同步国际化。
  const lang = useI18nStore((s) => s.lang);
  useEffect(() => {
    const resolved = getResolvedLang();
    void emit("language-changed", resolved).catch(() => {});
  }, [lang]);

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
        {view === "about" && <AboutView />}
        {view === "trash" && (
          <>
            <TrashView />
            <TrashDetail />
          </>
        )}
      </main>
      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}
