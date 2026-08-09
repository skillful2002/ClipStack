// 回收站列表（P8·修复）：展示已删除条目，支持恢复 / 彻底删除 / 清空。

import { useState } from "react";
import { useHistory } from "../store/history";
import { useItemActions } from "../lib/actions";
import { TYPE_META, relativeTime } from "../lib/format";
import { useT } from "../lib/i18n";
import type { HistoryItem } from "../types";
import { TypeIcon, RestoreIcon, TrashIcon } from "./icons";
import { ConfirmDialog } from "./ConfirmDialog";

export function TrashView() {
  const t = useT();
  const trashItems = useHistory((s) => s.trashItems);
  const selectedTrashId = useHistory((s) => s.selectedTrashId);
  const selectTrash = useHistory((s) => s.selectTrash);
  const { restore, purge, emptyTrash } = useItemActions();
  // 清空回收站确认对话框（Tauri WebView 不支持 window.confirm，必须用自定义模态）。
  const [confirmEmpty, setConfirmEmpty] = useState(false);

  return (
    <section className="list-pane">
      <div className="list-toolbar">
        <div className="time-tabs">
          <span className="trash-title">{t("trash.title")}</span>
        </div>
        <div className="trash-toolbar-right">
          <span className="list-count">{t("list.itemsCount", { n: trashItems.length })}</span>
          <button
            className="trash-empty-btn"
            onClick={() => {
              if (trashItems.length === 0) return;
              setConfirmEmpty(true);
            }}
            disabled={trashItems.length === 0}
          >
            {t("trash.emptyButton")}
          </button>
        </div>
      </div>

      <div className="list-body">
        {trashItems.length === 0 ? (
          <div className="list-empty">{t("trash.empty")}</div>
        ) : (
          trashItems.map((it: HistoryItem) => {
            const meta = TYPE_META[it.contentType];
            return (
              <div
                key={it.id}
                className={`item-row${it.id === selectedTrashId ? " selected" : ""}`}
                onClick={() => selectTrash(it.id)}
              >
                <span
                  className="item-type"
                  style={{ color: meta.color, background: `${meta.color}1a` }}
                >
                  <TypeIcon type={it.contentType} size={16} />
                </span>

                <div className="item-main">
                  <div className="item-preview">{it.preview || t("item.emptyContent")}</div>
                  <div className="item-sub">
                    <span className="item-app">{it.sourceApp || t("item.unknownSource")}</span>
                    <span className="item-dot">·</span>
                    <span>{t("trash.deletedAt", { time: relativeTime(it.deletedAt ?? it.createdAt) })}</span>
                  </div>
                </div>

                <div className="item-actions" onClick={(e) => e.stopPropagation()}>
                  <button title={t("action.restore")} onClick={() => restore(it)}>
                    <RestoreIcon size={15} />
                  </button>
                  <button
                    title={t("action.purge")}
                    className="danger"
                    onClick={() => purge(it)}
                  >
                    <TrashIcon size={15} />
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>

      {/* 清空回收站确认对话框 */}
      <ConfirmDialog
        open={confirmEmpty}
        title={t("trash.emptyButton")}
        message={t("trash.confirmEmpty")}
        confirmLabel={t("trash.emptyButton")}
        cancelLabel={t("confirm.cancel")}
        danger
        onConfirm={() => {
          setConfirmEmpty(false);
          void emptyTrash();
        }}
        onCancel={() => setConfirmEmpty(false)}
      />
    </section>
  );
}
