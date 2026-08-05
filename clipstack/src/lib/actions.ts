// P3 · 条目操作 hook（复制 / 置顶 / 收藏 / 删除），供列表行与详情面板共用。

import { useHistory } from "../store/history";
import * as api from "./tauri";
import type { HistoryItem } from "../types";

export function useItemActions() {
  const remove = useHistory((s) => s.remove);
  const removeTrash = useHistory((s) => s.removeTrash);
  const setTrashItems = useHistory((s) => s.setTrashItems);
  const reload = useHistory((s) => s.reload);
  const applyToggle = useHistory((s) => s.applyToggle);
  const setToast = useHistory((s) => s.setToast);

  const copy = async (item: HistoryItem) => {
    try {
      await api.copyItem(item.contentType, item.contentText);
      setToast("已复制到剪贴板");
    } catch (e) {
      setToast(`复制失败：${String(e)}`);
    }
  };

  const pin = async (item: HistoryItem) => {
    try {
      const v = await api.togglePin(item.id);
      applyToggle(item.id, "isPinned", v);
    } catch (e) {
      setToast(`操作失败：${String(e)}`);
    }
  };

  const fav = async (item: HistoryItem) => {
    try {
      const v = await api.toggleFavorite(item.id);
      applyToggle(item.id, "isFavorite", v);
    } catch (e) {
      setToast(`操作失败：${String(e)}`);
    }
  };

  const del = async (item: HistoryItem) => {
    try {
      await api.deleteItem(item.id);
      remove(item.id);
      setToast("已移至回收站");
    } catch (e) {
      setToast(`删除失败：${String(e)}`);
    }
  };

  const restore = async (item: HistoryItem) => {
    try {
      await api.restoreItem(item.id);
      removeTrash(item.id);
      // 恢复后条目回到主列表，刷新主列表使其立即可见（保留当前选中项）。
      void reload();
      setToast("已恢复");
    } catch (e) {
      setToast(`恢复失败：${String(e)}`);
    }
  };

  const purge = async (item: HistoryItem) => {
    try {
      await api.purgeItem(item.id);
      removeTrash(item.id);
      setToast("已彻底删除");
    } catch (e) {
      setToast(`删除失败：${String(e)}`);
    }
  };

  const emptyTrash = async () => {
    try {
      await api.emptyTrash();
      setTrashItems([]);
      setToast("回收站已清空");
    } catch (e) {
      setToast(`清空失败：${String(e)}`);
    }
  };

  return { copy, pin, fav, del, restore, purge, emptyTrash };
}
