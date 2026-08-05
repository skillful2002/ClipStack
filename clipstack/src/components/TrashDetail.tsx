// 回收站详情面板（P8·修复 / 回收站图片预览）：展示选中已删除条目，提供恢复 / 彻底删除。

import { useEffect, useState } from "react";
import { useHistory } from "../store/history";
import { useItemActions } from "../lib/actions";
import { getTrashBlob } from "../lib/tauri";
import { TYPE_META, formatBytes, fullDateTime } from "../lib/format";
import { TypeIcon, RestoreIcon, TrashIcon } from "./icons";

export function TrashDetail() {
  const item = useHistory(
    (s) => s.trashItems.find((i) => i.id === s.selectedTrashId) ?? null,
  );
  const { restore, purge } = useItemActions();

  // 回收站图片预览状态：加载中 / 对象 URL / 失败。
  const [imgUrl, setImgUrl] = useState<string | null>(null);
  const [imgLoading, setImgLoading] = useState(false);
  const [imgError, setImgError] = useState(false);

  // 选中回收站内的图片时，按 id 拉取 trash 表的 PNG 二进制生成预览 URL；
  // 切换条目 / 卸载时回收旧 URL，避免内存泄漏。
  useEffect(() => {
    let revoked = false;
    let url: string | null = null;

    if (item && item.contentType === "image") {
      setImgLoading(true);
      setImgError(false);
      setImgUrl(null);
      getTrashBlob(item.id)
        .then((bytes) => {
          if (revoked) return;
          const blob = new Blob([bytes], { type: "image/png" });
          url = URL.createObjectURL(blob);
          setImgUrl(url);
          setImgLoading(false);
        })
        .catch(() => {
          if (!revoked) {
            setImgError(true);
            setImgLoading(false);
          }
        });
    } else {
      setImgLoading(false);
      setImgError(false);
      setImgUrl(null);
    }

    return () => {
      revoked = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [item?.id, item?.contentType]);

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
          <div className="preview-image-wrap">
            {imgLoading && <div className="preview-image-placeholder">图片加载中…</div>}
            {imgError && <div className="preview-image-placeholder">图片加载失败</div>}
            {imgUrl && !imgLoading && !imgError && (
              <img
                className="preview-image"
                src={imgUrl}
                alt="回收站剪贴板图片预览"
                onError={() => setImgError(true)}
              />
            )}
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
