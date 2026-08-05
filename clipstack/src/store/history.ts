// P3 · 全局状态仓库（Zustand）。
//
// 负责：历史列表、选中项、分类 / 时间 / 搜索过滤、视图切换（主界面 / 设置 / 回收站）。
// 与后端的同步通过 src/lib/tauri.ts 的命令 / 事件完成。

import { create } from "zustand";
import type { ContentType, HistoryItem } from "../types";
import * as api from "../lib/tauri";

/** 侧边栏分类：全部 + 五种内容类型。 */
export type Category = ContentType | "all";
/** 主列表时间筛选。 */
export type TimeFilter = "all" | "today" | "yesterday" | "week";
/** 主区域视图。 */
export type View = "main" | "settings" | "trash";

interface HistoryState {
  items: HistoryItem[];
  selectedId: number | null;
  category: Category;
  timeFilter: TimeFilter;
  search: string;
  view: View;
  loading: boolean;
  error: string | null;
  /** 轻量提示（复制成功 / 错误等），由 App 渲染并在数秒后清除。 */
  toast: string | null;

  /** 回收站条目（与主页 items 隔离，避免 id 冲突）。 */
  trashItems: HistoryItem[];
  selectedTrashId: number | null;

  load: () => Promise<void>;
  /** 重新拉取主列表（恢复后保持选中项不丢失）。 */
  reload: () => Promise<void>;
  loadTrash: () => Promise<void>;
  /** 直接替换回收站列表（如清空后）。 */
  setTrashItems: (items: HistoryItem[]) => void;
  setToast: (msg: string | null) => void;
  select: (id: number | null) => void;
  selectTrash: (id: number | null) => void;
  setCategory: (c: Category) => void;
  setTimeFilter: (t: TimeFilter) => void;
  setSearch: (s: string) => void;
  setView: (v: View) => void;
  /** 实时事件：新条目到达（同 id 替换，否则前置）。 */
  prepend: (item: HistoryItem) => void;
  /** 删除：从列表移除并修正选中项。 */
  remove: (id: number) => void;
  /** 回收站：从本地列表移除并修正选中项。 */
  removeTrash: (id: number) => void;
  /** 置顶 / 收藏切换后回写状态（保持置顶优先排序）。 */
  applyToggle: (id: number, field: "isPinned" | "isFavorite", value: boolean) => void;
}

/** 置顶优先、时间倒序。 */
const sortItems = (items: HistoryItem[]): HistoryItem[] =>
  [...items].sort((a, b) => {
    if (a.isPinned !== b.isPinned) return a.isPinned ? -1 : 1;
    return b.createdAt - a.createdAt;
  });

export const useHistory = create<HistoryState>((set, get) => ({
  items: [],
  selectedId: null,
  category: "all",
  timeFilter: "all",
  search: "",
  view: "main",
  loading: false,
  error: null,
  toast: null,
  trashItems: [],
  selectedTrashId: null,

  load: async () => {
    set({ loading: true, error: null });
    try {
      // 读取历史上限设置（默认 1000），用于首次拉取条数。
      let limit = 1000;
      try {
        const settings = await api.getSettings();
        const mh = Number(settings.find((s) => s.key === "max_history")?.value);
        if (mh > 0) limit = mh;
      } catch {
        /* 设置读取失败时回退默认 */
      }
      const items = await api.getHistory(limit);
      set({
        items: sortItems(items),
        loading: false,
        selectedId: items[0]?.id ?? null,
      });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  // 重新拉取主列表，保留当前选中项（若存在），用于恢复条目后刷新。
  reload: async () => {
    set({ loading: true, error: null });
    try {
      let limit = 1000;
      try {
        const settings = await api.getSettings();
        const mh = Number(settings.find((s) => s.key === "max_history")?.value);
        if (mh > 0) limit = mh;
      } catch {
        /* 设置读取失败时回退默认 */
      }
      const items = await api.getHistory(limit);
      const prev = get().selectedId;
      set({
        items: sortItems(items),
        loading: false,
        selectedId:
          prev != null && items.some((i) => i.id === prev)
            ? prev
            : (items[0]?.id ?? null),
      });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  // 读取回收站（按删除时间倒序）。
  loadTrash: async () => {
    set({ loading: true, error: null });
    try {
      const items = await api.getTrash();
      set({
        trashItems: items,
        loading: false,
        selectedTrashId: items[0]?.id ?? null,
      });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  select: (id) => set({ selectedId: id }),
  selectTrash: (id) => set({ selectedTrashId: id }),
  setTrashItems: (items) =>
    set({ trashItems: items, selectedTrashId: items[0]?.id ?? null }),
  setCategory: (category) => set({ category }),
  setTimeFilter: (timeFilter) => set({ timeFilter }),
  setSearch: (search) => set({ search }),
  setView: (view) => set({ view }),
  setToast: (toast) => set({ toast }),

  prepend: (item) => {
    const exists = get().items.some((i) => i.id === item.id);
    const items = exists
      ? get().items.map((i) => (i.id === item.id ? item : i))
      : [item, ...get().items];
    set({ items: sortItems(items), selectedId: get().selectedId ?? item.id });
  },

  remove: (id) => {
    const items = get().items.filter((i) => i.id !== id);
    set({
      items: sortItems(items),
      selectedId:
        get().selectedId === id ? (items[0]?.id ?? null) : get().selectedId,
    });
  },

  removeTrash: (id) => {
    const trashItems = get().trashItems.filter((i) => i.id !== id);
    set({
      trashItems,
      selectedTrashId:
        get().selectedTrashId === id
          ? (trashItems[0]?.id ?? null)
          : get().selectedTrashId,
    });
  },

  applyToggle: (id, field, value) => {
    const items = get().items.map((i) =>
      i.id === id ? { ...i, [field]: value } : i,
    );
    set({ items: sortItems(items) });
  },
}));

/** 纯函数：按分类 / 时间 / 搜索过滤（不改变顺序，由调用方渲染）。 */
export function filterItems(
  items: HistoryItem[],
  category: Category,
  timeFilter: TimeFilter,
  search: string,
): HistoryItem[] {
  const startOfToday = new Date();
  startOfToday.setHours(0, 0, 0, 0);
  const startOfYesterday = startOfToday.getTime() - 86_400_000;
  const startOfWeek = startOfToday.getTime() - 7 * 86_400_000;
  const q = search.trim().toLowerCase();

  return items.filter((it) => {
    if (category !== "all" && it.contentType !== category) return false;

    if (timeFilter !== "all") {
      const t = it.createdAt;
      if (timeFilter === "today" && t < startOfToday.getTime()) return false;
      if (
        timeFilter === "yesterday" &&
        (t < startOfYesterday || t >= startOfToday.getTime())
      )
        return false;
      if (timeFilter === "week" && t < startOfWeek) return false;
    }

    if (q) {
      const hay = `${it.preview} ${it.contentText} ${it.sourceApp}`.toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
}
