// P3 · 右侧详情面板：类型标签、内容预览、元数据、操作区。
// P8 · 图片预览：选中图片时按 id 拉取 PNG 二进制，生成对象 URL 渲染 <img>。

import { useEffect, useState } from "react";
import { useHistory } from "../store/history";
import { useItemActions } from "../lib/actions";
import { getItemBlob } from "../lib/tauri";
import { TYPE_META, formatBytes, fullDateTime } from "../lib/format";
import { TypeIcon, CopyIcon, PinIcon, StarIcon, TrashIcon } from "./icons";

export function DetailPanel() {
  const item = useHistory((s) => s.items.find((i) => i.id === s.selectedId) ?? null);
  const { copy, pin, fav, del } = useItemActions();

  // 图片预览状态：加载中 / 已就绪的对象 URL / 失败。
  const [imgUrl, setImgUrl] = useState<string | null>(null);
  const [imgLoading, setImgLoading] = useState(false);
  const [imgError, setImgError] = useState(false);

  // 选中图片时拉取二进制并生成可预览的对象 URL；切换 / 卸载时回收旧 URL，避免内存泄漏。
  useEffect(() => {
    let revoked = false;
    let url: string | null = null;

    if (item && item.contentType === "image") {
      setImgLoading(true);
      setImgError(false);
      setImgUrl(null);
      getItemBlob(item.id)
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
          <div className="preview-image-wrap">
            {imgLoading && <div className="preview-image-placeholder">图片加载中…</div>}
            {imgError && <div className="preview-image-placeholder">图片加载失败</div>}
            {imgUrl && !imgLoading && !imgError && (
              <img
                className="preview-image"
                src={imgUrl}
                alt="剪贴板图片预览"
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
