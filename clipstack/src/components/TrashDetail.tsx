// 回收站详情面板（P8·修复）：展示选中已删除条目，提供恢复 / 彻底删除。

import { useHistory } from "../store/history";
import { useItemActions } from "../lib/actions";
import { TYPE_META, formatBytes, fullDateTime } from "../lib/format";
import { TypeIcon, RestoreIcon, TrashIcon } from "./icons";

export function TrashDetail() {
  const item = useHistory(
    (s) => s.trashItems.find((i) => i.id === s.selectedTrashId) ?? null,
  );
  const { restore, purge } = useItemActions();

  if (!item) {
    return (
      <aside className="detail-pane">
        <div className="detail-empty">选择左侧条目查看详情</div>
      </aside>
    );
  }

  const meta = TYPE_META[item.contentType];
  const isImage = item.contentType === "image";
  const isFile = item.contentType === "file";

  return (
    <aside className="detail-pane">
      <div className="detail-head">
        <span
          className="detail-type"
          style={{ color: meta.color, background: `${meta.color}1a` }}
        >
          <TypeIcon type={item.contentType} size={16} />
          {meta.label}
        </span>
        <span className="detail-app">{item.sourceApp || "未知来源"}</span>
      </div>

      <div className="detail-preview">
        {isImage ? (
          <div className="preview-image-placeholder">
            图片（回收站内暂不支持预览，恢复后可查看）
          </div>
        ) : isFile ? (
          <div className="preview-files">
            {item.contentText.split(", ").map((p, i) => (
              <div key={i} className="preview-file-row">
                {p}
              </div>
            ))}
          </div>
        ) : (
          <div className={`preview-text${item.contentType === "code" ? " code" : ""}`}>
            {item.contentText}
          </div>
        )}
      </div>

      <div className="detail-meta">
        <div className="meta-row">
          <span className="meta-key">大小</span>
          <span className="meta-val">{formatBytes(item.sizeBytes)}</span>
        </div>
        <div className="meta-row">
          <span className="meta-key">删除时间</span>
          <span className="meta-val">{fullDateTime(item.deletedAt ?? item.createdAt)}</span>
        </div>
      </div>

      <div className="detail-actions">
        <button className="primary" onClick={() => restore(item)}>
          <RestoreIcon size={15} />
          恢复
        </button>
        <button className="danger" onClick={() => purge(item)}>
          <TrashIcon size={15} />
          彻底删除
        </button>
      </div>
    </aside>
  );
}
