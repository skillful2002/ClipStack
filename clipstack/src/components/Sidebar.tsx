// P3 · 左侧边栏：搜索、分类导航、回收站 / 设置入口。

import { useMemo } from "react";
import { useHistory, type Category } from "../store/history";
import type { ContentType } from "../types";
import { TYPE_META } from "../lib/format";
import { useT } from "../lib/i18n";
import { NAV_SHORTCUTS } from "../lib/shortcuts";
import {
  SearchIcon,
  TypeIcon,
  SettingsIcon,
  TrashBinIcon,
  AboutIcon,
  HelpIcon,
  AllIcon,
  MainWindowIcon,
} from "./icons";

const CATEGORIES: { key: Category; type?: ContentType }[] = [
  { key: "all" },
  { key: "text", type: "text" },
  { key: "link", type: "link" },
  { key: "code", type: "code" },
  { key: "image", type: "image" },
  { key: "file", type: "file" },
];

export function Sidebar() {
  const t = useT();
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

  const categoryLabel = (cat: (typeof CATEGORIES)[number]): string =>
    cat.key === "all" ? t("sidebar.all") : t(`type.${cat.type}`);

  return (
    <aside className="sidebar">
      <div className="sidebar-search">
        <SearchIcon size={15} />
        <input
          id="clipstack-search"
          type="text"
          placeholder={t("sidebar.searchPlaceholder")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label={t("sidebar.searchAria")}
        />
        <span className="search-shortcut">{NAV_SHORTCUTS.search}</span>
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
              {cat.type ? <TypeIcon type={cat.type} size={16} /> : <AllIcon size={16} />}
            </span>
            <span className="nav-label">{categoryLabel(cat)}</span>
            <span className="nav-shortcut">{NAV_SHORTCUTS[cat.key]}</span>
            <span className="nav-count">{counts[cat.key] ?? 0}</span>
          </button>
        ))}
      </nav>

      <div className="sidebar-bottom">
        <button
          className={`nav-item${view === "main" ? " active" : ""}`}
          onClick={() => {
            setCategory("all");
            setView("main");
          }}
        >
          <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
            <MainWindowIcon size={16} />
          </span>
          <span className="nav-label">{t("sidebar.main")}</span>
        </button>
        <button
          className={`nav-item${view === "settings" ? " active" : ""}`}
          onClick={() => setView("settings")}
        >
          <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
            <SettingsIcon size={16} />
          </span>
          <span className="nav-label">{t("sidebar.settings")}</span>
          <span className="nav-shortcut">{NAV_SHORTCUTS.settings}</span>
        </button>
        <button
          className={`nav-item${view === "trash" ? " active" : ""}`}
          onClick={() => setView("trash")}
        >
          <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
            <TrashBinIcon size={16} />
          </span>
          <span className="nav-label">{t("sidebar.trash")}</span>
          <span className="nav-shortcut">{NAV_SHORTCUTS.trash}</span>
        </button>
        <button
          className={`nav-item${view === "help" ? " active" : ""}`}
          onClick={() => setView("help")}
        >
          <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
            <HelpIcon size={16} />
          </span>
          <span className="nav-label">{t("sidebar.help")}</span>
        </button>
        <button
          className={`nav-item${view === "about" ? " active" : ""}`}
          onClick={() => setView("about")}
        >
          <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
            <AboutIcon size={16} />
          </span>
          <span className="nav-label">{t("sidebar.about")}</span>
        </button>
      </div>
    </aside>
  );
}
