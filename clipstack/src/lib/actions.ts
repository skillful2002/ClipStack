// P3 · 条目操作 hook（复制 / 置顶 / 收藏 / 删除 / 另存），供列表行与详情面板共用。

import { useHistory } from "../store/history";
import * as api from "./tauri";
import { useT } from "./i18n";
import { save as dialogSave, open as dialogOpen } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
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
    try {
      if (item.contentType === "image") {
        // 图片：从数据库读取二进制并解码后写回剪贴板。
        await api.copyImage(item.id);
      } else if (item.contentType === "file") {
        // 文件：从数据库读取路径列表，写回系统剪贴板文件列表（可粘贴为文件本身）。
        await api.copyFile(item.id);
      } else {
        // 文本 / 链接 / 代码：直接写回文本。
        await api.copyItem(item.contentType, item.contentText);
      }
      setToast(t("toast.copied"));
    } catch (e) {
      setToast(t("toast.copyFailed", { error: String(e) }));
    }
  };

  const save = async (item: HistoryItem) => {
    try {
      if (item.contentType === "image") {
        // 图片：让用户选择保存路径，由后端从数据库读取 PNG 二进制写入磁盘。
        const def = `clipstack-${item.id}.png`;
        const target = await dialogSave({ defaultPath: def });
        if (!target) return;
        await api.saveItemAs(item.id, target, "image");
        setToast(t("toast.saved"));
      } else if (item.contentType === "file") {
        // 文件：解析内容中的本地路径列表，单文件让用户选文件名，多文件让用户选目录。
        const bytes = await api.getItemBlob(item.id);
        let paths: string[] = [];
        try {
          paths = JSON.parse(new TextDecoder().decode(bytes));
        } catch {
          // 解析失败则回退到 content_text 按 ", " 拆分
        }
        if (!Array.isArray(paths) || paths.length === 0) {
          paths = item.contentText.split(", ").filter(Boolean);
        }
        if (paths.length === 0) {
          setToast(t("toast.saveNoFile"));
          return;
        }
        let target: string | null = null;
        if (paths.length === 1) {
          const name = paths[0].split(/[\\/]/).pop() || "file";
          target = await dialogSave({ defaultPath: name });
        } else {
          target = (await dialogOpen({ directory: true, multiple: false })) as string | null;
        }
        if (!target) return;
        await api.saveItemAs(item.id, target, "file");
        setToast(t("toast.saved"));
      }
    } catch (e) {
      setToast(t("toast.saveFailed", { error: String(e) }));
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

  const clearFiltered = async (ids: number[]) => {
    try {
      const n = await api.deleteItems(ids);
      // 删除命中的条目（软删入回收站），刷新主列表与回收站。
      void reload();
      void loadTrash();
      setToast(t("toast.allMovedToTrash", { n }));
    } catch (e) {
      setToast(t("toast.clearFailed", { error: String(e) }));
    }
  };

  // 用系统默认浏览器打开链接类型条目的 URL。
  const open = async (item: HistoryItem) => {
    try {
      const u = /^[a-z][a-z0-9+.-]*:\/\//i.test(item.contentText)
        ? item.contentText
        : `https://${item.contentText}`;
      await openUrl(u);
    } catch (e) {
      setToast(t("toast.opFailed", { error: String(e) }));
    }
  };

  return { copy, save, pin, fav, del, open, restore, purge, emptyTrash, clearFiltered };
}
