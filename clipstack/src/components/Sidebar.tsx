// P3 · 左侧边栏：搜索、分类导航、回收站 / 设置入口。

import { useMemo } from "react";
import { useHistory, type Category } from "../store/history";
import type { ContentType } from "../types";
import { TYPE_META } from "../lib/format";
import {
  SearchIcon,
  TypeIcon,
  SettingsIcon,
  TrashBinIcon,
} from "./icons";

const CATEGORIES: { key: Category; label: string; type?: ContentType }[] = [
  { key: "all", label: "全部" },
  { key: "text", label: TYPE_META.text.label, type: "text" },
  { key: "link", label: TYPE_META.link.label, type: "link" },
  { key: "code", label: TYPE_META.code.label, type: "code" },
  { key: "image", label: TYPE_META.image.label, type: "image" },
  { key: "file", label: TYPE_META.file.label, type: "file" },
];

export function Sidebar() {
  const items = useHistory((s) => s.items);
  const category = useHistory((s) => s.category);
  const setCategory = useHistory((s) => s.setCategory);
  const search = useHistory((s) => s.search);
  const setSearch = useHistory((s) => s.setSearch);
  const view = useHistory((s) => s.view);
  const setView = useHistory((s) => s.setView);

  const counts = useMemo(() => {
    const c: Record<string, number> = { all: items.length };
    for (const it of items) c[it.contentType] = (c[it.contentType] ?? 0) + 1;
    return c;
  }, [items]);

  return (
    <aside className="sidebar">
      <div className="sidebar-search">
        <SearchIcon size={15} />
        <input
          id="clipstack-search"
          type="text"
          placeholder="搜索剪贴板内容…  (⌘K)"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="搜索"
        />
      </div>

      <nav className="sidebar-nav">
        {CATEGORIES.map((cat) => (
          <button
            key={cat.key}
            className={`nav-item${category === cat.key ? " active" : ""}`}
            onClick={() => {
              setCategory(cat.key);
              setView("main");
            }}
          >
            <span className="nav-icon" style={{ color: cat.type ? TYPE_META[cat.type].color : "var(--cs-text-secondary)" }}>
              {cat.type ? <TypeIcon type={cat.type} size={16} /> : <TypeIcon type="text" size={16} />}
            </span>
            <span className="nav-label">{cat.label}</span>
            <span className="nav-count">{counts[cat.key] ?? 0}</span>
          </button>
        ))}
      </nav>

      <div className="sidebar-bottom">
        <button
          className={`nav-item${view === "trash" ? " active" : ""}`}
          onClick={() => setView("trash")}
        >
          <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
            <TrashBinIcon size={16} />
          </span>
          <span className="nav-label">回收站</span>
        </button>
        <button
          className={`nav-item${view === "settings" ? " active" : ""}`}
          onClick={() => setView("settings")}
        >
          <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
            <SettingsIcon size={16} />
          </span>
          <span className="nav-label">设置</span>
        </button>
      </div>
    </aside>
  );
}
