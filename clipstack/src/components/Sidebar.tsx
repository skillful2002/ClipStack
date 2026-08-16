// P3 · 左侧边栏：搜索、分类导航、回收站 / 设置入口。

import { useEffect, useMemo, useRef, useState } from "react";
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
  SourceIcon,
  ClearIcon,
  MoreIcon,
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
  const sourceFilter = useHistory((s) => s.sourceFilter);
  const setSourceFilter = useHistory((s) => s.setSourceFilter);
  const view = useHistory((s) => s.view);
  const setView = useHistory((s) => s.setView);
  const hasUpdate = useHistory((s) => s.updateInfo?.hasUpdate ?? false);
  const searchRef = useRef<HTMLInputElement>(null);

  // 底部「更多」弹出菜单开关；打开期间按 Esc 关闭。
  const [moreOpen, setMoreOpen] = useState(false);
  useEffect(() => {
    if (!moreOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMoreOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [moreOpen]);

  // 「更多」菜单收纳的视图之一处于当前视图时，触发器高亮。
  const moreActive =
    view === "settings" || view === "trash" || view === "help" || view === "about";

  const counts = useMemo(() => {
    const c: Record<string, number> = { all: items.length };
    for (const it of items) c[it.contentType] = (c[it.contentType] ?? 0) + 1;
    return c;
  }, [items]);

  // 最常见的 5 个来源：数量降序；数量相同按最后剪切时间（createdAt）倒序。
  // 按当前所选分类（类型）限定用于统计来源的数据范围：
  // 「全部」用全量 items，否则只用该 content type 的条目。
  const scopedItems = useMemo(
    () => (category === "all" ? items : items.filter((i) => i.contentType === category)),
    [items, category],
  );

  // 空来源（未知）以空串为 key，显示时回落到「未知来源」文案。
  const topSources = useMemo(() => {
    const map = new Map<string, { count: number; last: number }>();
    for (const it of scopedItems) {
      const key = it.sourceApp || "";
      const e = map.get(key) ?? { count: 0, last: 0 };
      e.count += 1;
      e.last = Math.max(e.last, it.createdAt);
      map.set(key, e);
    }
    return [...map.entries()]
      .sort((a, b) => b[1].count - a[1].count || b[1].last - a[1].last)
      .slice(0, 8)
      .map(([key]) => key);
  }, [scopedItems]);

  // 各来源数量，供来源项徽标使用。
  const sourceCounts = useMemo(() => {
    const c: Record<string, number> = { "": 0 };
    for (const it of scopedItems) c[it.sourceApp || ""] = (c[it.sourceApp || ""] ?? 0) + 1;
    return c;
  }, [scopedItems]);

  const categoryLabel = (cat: (typeof CATEGORIES)[number]): string =>
    cat.key === "all" ? t("sidebar.all") : t(`type.${cat.type}`);

  // 来源项的显示文案：空串（未知来源）回落到对应文案。
  const sourceLabel = (key: string): string =>
    key === "" ? t("item.unknownSource") : key;

  return (
    <aside className="sidebar">
      <div className="sidebar-search">
        <SearchIcon size={15} />
        <input
          id="clipstack-search"
          ref={searchRef}
          type="text"
          placeholder={t("sidebar.searchPlaceholder")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label={t("sidebar.searchAria")}
        />
        {search ? (
          <button
            type="button"
            className="search-clear"
            onClick={() => {
              setSearch("");
              searchRef.current?.focus();
            }}
            aria-label={t("sidebar.searchClear")}
            title={t("sidebar.searchClear")}
          >
            <ClearIcon size={13} />
          </button>
        ) : (
          <span className="search-shortcut">{NAV_SHORTCUTS.search}</span>
        )}
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

        {topSources.length > 0 && <div className="nav-divider" />}

        {topSources.map((key) => (
          <button
            key={key}
            className={`nav-item nav-subitem${sourceFilter === key ? " active" : ""}`}
            onClick={() => {
              setSourceFilter(sourceFilter === key ? null : key);
              setView("main");
            }}
            title={sourceLabel(key)}
          >
            <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
              <SourceIcon size={16} />
            </span>
            <span className="nav-label">{sourceLabel(key)}</span>
            <span className="nav-count">{sourceCounts[key] ?? 0}</span>
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
        <div className="sidebar-more">
          <button
            type="button"
            className={`nav-item${moreActive ? " active" : ""}`}
            onClick={() => setMoreOpen((v) => !v)}
            aria-haspopup="menu"
            aria-expanded={moreOpen}
          >
            <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
              <MoreIcon size={16} />
            </span>
            <span className="nav-label">{t("sidebar.more")}</span>
            {hasUpdate && !moreOpen && (
              <span className="nav-badge" aria-label={t("about.newVersionBadge")} />
            )}
          </button>
          {moreOpen && (
            <>
              <div className="more-overlay" onClick={() => setMoreOpen(false)} />
              <div className="more-menu" role="menu">
                <button
                  type="button"
                  role="menuitem"
                  className={`more-menu-item${view === "settings" ? " active" : ""}`}
                  onClick={() => {
                    setView("settings");
                    setMoreOpen(false);
                  }}
                >
                  <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
                    <SettingsIcon size={16} />
                  </span>
                  <span className="nav-label">{t("sidebar.settings")}</span>
                  <span className="nav-shortcut">{NAV_SHORTCUTS.settings}</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className={`more-menu-item${view === "trash" ? " active" : ""}`}
                  onClick={() => {
                    setView("trash");
                    setMoreOpen(false);
                  }}
                >
                  <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
                    <TrashBinIcon size={16} />
                  </span>
                  <span className="nav-label">{t("sidebar.trash")}</span>
                  <span className="nav-shortcut">{NAV_SHORTCUTS.trash}</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className={`more-menu-item${view === "help" ? " active" : ""}`}
                  onClick={() => {
                    setView("help");
                    setMoreOpen(false);
                  }}
                >
                  <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
                    <HelpIcon size={16} />
                  </span>
                  <span className="nav-label">{t("sidebar.help")}</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className={`more-menu-item${view === "about" ? " active" : ""}`}
                  onClick={() => {
                    setView("about");
                    setMoreOpen(false);
                  }}
                >
                  <span className="nav-icon" style={{ color: "var(--cs-text-secondary)" }}>
                    <AboutIcon size={16} />
                  </span>
                  <span className="nav-label">{t("sidebar.about")}</span>
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </aside>
  );
}
