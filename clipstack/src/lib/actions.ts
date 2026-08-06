// P3 · 条目操作 hook（复制 / 置顶 / 收藏 / 删除），供列表行与详情面板共用。

import { useHistory } from "../store/history";
import * as api from "./tauri";
import { useT } from "./i18n";
import type { HistoryItem } from "../types";

export function useItemActions() {
  const remove = useHistory((s) => s.remove);
  const removeTrash = useHistory((s) => s.removeTrash);
  const setTrashItems = useHistory((s) => s.setTrashItems);
  const reload = useHistory((s) => s.reload);
  const loadTrash = useHistory((s) => s.loadTrash);
  const applyToggle = useHistory((s) => s.applyToggle);
  const setToast = useHistory((s) => s.setToast);
  const t = useT();

  const copy = async (item: HistoryItem) => {
    // 图片 / 文件因平台 API 限制暂不支持一键复制：直接给出本地化提示，不再调用后端（避免后端硬编码中文报错）。
    if (item.contentType === "image" || item.contentType === "file") {
      setToast(t("toast.copyUnsupported"));
      return;
    }
    try {
      await api.copyItem(item.contentType, item.contentText);
      setToast(t("toast.copied"));
    } catch (e) {
      setToast(t("toast.copyFailed", { error: String(e) }));
    }
  };

  const pin = async (item: HistoryItem) => {
    try {
      const v = await api.togglePin(item.id);
      applyToggle(item.id, "isPinned", v);
    } catch (e) {
      setToast(t("toast.opFailed", { error: String(e) }));
    }
  };

  const fav = async (item: HistoryItem) => {
    try {
      const v = await api.toggleFavorite(item.id);
      applyToggle(item.id, "isFavorite", v);
    } catch (e) {
      setToast(t("toast.opFailed", { error: String(e) }));
    }
  };

  const del = async (item: HistoryItem) => {
    try {
      await api.deleteItem(item.id);
      remove(item.id);
      setToast(t("toast.deleted"));
    } catch (e) {
      setToast(t("toast.deleteFailed", { error: String(e) }));
    }
  };

  const restore = async (item: HistoryItem) => {
    try {
      await api.restoreItem(item.id);
      removeTrash(item.id);
      // 恢复后条目回到主列表，刷新主列表使其立即可见（保留当前选中项）。
      void reload();
      setToast(t("toast.restored"));
    } catch (e) {
      setToast(t("toast.restoreFailed", { error: String(e) }));
    }
  };

  const purge = async (item: HistoryItem) => {
    try {
      await api.purgeItem(item.id);
      removeTrash(item.id);
      setToast(t("toast.purged"));
    } catch (e) {
      setToast(t("toast.purgeFailed", { error: String(e) }));
    }
  };

  const emptyTrash = async () => {
    try {
      await api.emptyTrash();
      setTrashItems([]);
      setToast(t("toast.trashEmptied"));
    } catch (e) {
      setToast(t("toast.emptyFailed", { error: String(e) }));
    }
  };

  const clearAll = async () => {
    try {
      await api.clearAllHistory();
      // 主列表清空并刷新，回收站同步刷新（新条目已软删入回收站）。
      void reload();
      void loadTrash();
      setToast(t("toast.allMovedToTrash"));
    } catch (e) {
      setToast(t("toast.clearFailed", { error: String(e) }));
    }
  };

  return { copy, pin, fav, del, restore, purge, emptyTrash, clearAll };
}
