// 回收站列表（P8·修复）：展示已删除条目，支持恢复 / 彻底删除 / 清空。

import { useHistory } from "../store/history";
import { useItemActions } from "../lib/actions";
import { TYPE_META, relativeTime } from "../lib/format";
import type { HistoryItem } from "../types";
import { TypeIcon, RestoreIcon, TrashIcon } from "./icons";

export function TrashView() {
  const trashItems = useHistory((s) => s.trashItems);
  const selectedTrashId = useHistory((s) => s.selectedTrashId);
  const selectTrash = useHistory((s) => s.selectTrash);
  const { restore, purge, emptyTrash } = useItemActions();

  return (
    <section className="list-pane">
      <div className="list-toolbar">
        <div className="time-tabs">
          <span className="trash-title">回收站</span>
        </div>
        <div className="trash-toolbar-right">
          <span className="list-count">{trashItems.length} 项</span>
          <button
            className="trash-empty-btn"
            onClick={() => {
              if (trashItems.length === 0) return;
              if (confirm("确定清空回收站？此操作不可恢复。")) void emptyTrash();
            }}
            disabled={trashItems.length === 0}
          >
            清空回收站
          </button>
        </div>
      </div>

      <div className="list-body">
        {trashItems.length === 0 ? (
          <div className="list-empty">回收站为空，删除的剪贴板记录会暂存于此</div>
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
                  <div className="item-preview">{it.preview || "（空内容）"}</div>
                  <div className="item-sub">
                    <span className="item-app">{it.sourceApp || "未知来源"}</span>
                    <span className="item-dot">·</span>
                    <span>删除于 {relativeTime(it.deletedAt ?? it.createdAt)}</span>
                  </div>
                </div>

                <div className="item-actions" onClick={(e) => e.stopPropagation()}>
                  <button title="恢复" onClick={() => restore(it)}>
                    <RestoreIcon size={15} />
                  </button>
                  <button
                    title="彻底删除"
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
    </section>
  );
}
