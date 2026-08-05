// P3 · 右侧详情面板：类型标签、内容预览、元数据、操作区。

import { useHistory } from "../store/history";
import { useItemActions } from "../lib/actions";
import { TYPE_META, formatBytes, fullDateTime } from "../lib/format";
import { TypeIcon, CopyIcon, PinIcon, StarIcon, TrashIcon } from "./icons";

export function DetailPanel() {
  const item = useHistory((s) => s.items.find((i) => i.id === s.selectedId) ?? null);
  const { copy, pin, fav, del } = useItemActions();

  if (!item) {
    return (
      <aside className="detail-pane">
        <div className="detail-empty">选择左侧条目查看详情</div>
      </aside>
    );
  }

  const meta = TYPE_META[item.contentType];
  const isCode = item.contentType === "code";
  const isFile = item.contentType === "file";
  const isImage = item.contentType === "image";

  return (
    <aside className="detail-pane">
      <div className="detail-head">
        <span className="detail-type" style={{ color: meta.color, background: `${meta.color}1a` }}>
          <TypeIcon type={item.contentType} size={16} />
          {meta.label}
        </span>
        <span className="detail-app">{item.sourceApp || "未知来源"}</span>
      </div>

      <div className="detail-preview">
        {isImage ? (
          <div className="preview-image-placeholder">图片预览（P3 暂不支持）</div>
        ) : isFile ? (
          <div className="preview-files">
            {item.contentText.split(", ").map((p, i) => (
              <div key={i} className="preview-file-row">
                {p}
              </div>
            ))}
          </div>
        ) : (
          <div className={`preview-text${isCode ? " code" : ""}`}>{item.contentText}</div>
        )}
      </div>

      <div className="detail-meta">
        <div className="meta-row">
          <span className="meta-key">来源应用</span>
          <span className="meta-val">{item.sourceApp || "—"}</span>
        </div>
        <div className="meta-row">
          <span className="meta-key">大小</span>
          <span className="meta-val">{formatBytes(item.sizeBytes)}</span>
        </div>
        <div className="meta-row">
          <span className="meta-key">时间</span>
          <span className="meta-val">{fullDateTime(item.createdAt)}</span>
        </div>
        <div className="meta-row">
          <span className="meta-key">哈希</span>
          <span className="meta-val mono">{item.hash}</span>
        </div>
      </div>

      <div className="detail-actions">
        <button className="primary" onClick={() => copy(item)}>
          <CopyIcon size={15} /> 复制
        </button>
        <button onClick={() => pin(item)}>
          <PinIcon size={15} active={item.isPinned} /> {item.isPinned ? "取消置顶" : "置顶"}
        </button>
        <button onClick={() => fav(item)}>
          <StarIcon size={15} active={item.isFavorite} /> {item.isFavorite ? "取消收藏" : "收藏"}
        </button>
        <button className="danger" onClick={() => del(item)}>
          <TrashIcon size={15} /> 删除
        </button>
      </div>
    </aside>
  );
}
