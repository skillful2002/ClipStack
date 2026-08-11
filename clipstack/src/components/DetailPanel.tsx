// P3 · 右侧详情面板：类型标签、内容预览、元数据、操作区。
// P8 · 图片预览：选中图片时按 id 拉取 PNG 二进制，生成对象 URL 渲染 <img>。

import { useEffect, useState, useRef, useLayoutEffect } from "react";
import { useHistory } from "../store/history";
import { useItemActions } from "../lib/actions";
import { getItemBlob } from "../lib/tauri";
import { TYPE_META, formatBytes, fullDateTime } from "../lib/format";
import { useT } from "../lib/i18n";
import { ACTION_SHORTCUTS } from "../lib/shortcuts";
import { TypeIcon, CopyIcon, PinIcon, StarIcon, TrashIcon } from "./icons";

export function DetailPanel() {
  const t = useT();
  const item = useHistory((s) => s.items.find((i) => i.id === s.selectedId) ?? null);
  const { copy, pin, fav, del } = useItemActions();

  // 详情面板宽度随「当前语言的操作按钮实际宽度」自适应：中文窄、法文宽，
  // 既不换行也不截断，也不在非中文下留过多空白。
  const paneRef = useRef<HTMLElement>(null);
  const actionsRef = useRef<HTMLDivElement>(null);
  const copyLabel = t("action.copy");
  const pinLabel = item?.isPinned ? t("action.unpin") : t("action.pin");
  const favLabel = item?.isFavorite ? t("action.unfav") : t("action.fav");
  const delLabel = t("action.delete");
  useLayoutEffect(() => {
    const pane = paneRef.current;
    if (!pane) return;
    const actions = actionsRef.current;
    if (actions) {
      const needed = actions.scrollWidth + 32; // 左右各 16px 内边距
      pane.style.width = Math.min(Math.max(needed, 300), 520) + "px";
    } else {
      pane.style.width = "320px";
    }
  }, [copyLabel, pinLabel, favLabel, delLabel, item?.id]);

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
      <aside className="detail-pane" ref={paneRef}>
        <div className="detail-empty">{t("detail.empty")}</div>
      </aside>
    );
  }

  const meta = TYPE_META[item.contentType];
  const isCode = item.contentType === "code";
  const isFile = item.contentType === "file";
  const isImage = item.contentType === "image";

  return (
    <aside className="detail-pane" ref={paneRef}>
      <div className="detail-head">
        <span className="detail-type" style={{ color: meta.color, background: `${meta.color}1a` }}>
          <TypeIcon type={item.contentType} size={16} />
          {t(`type.${item.contentType}`)}
        </span>
        <span className="detail-app">
          {item.isRemote
            ? (item.originDevice || t("item.local"))
            : (item.sourceApp || t("item.unknownSource"))}
        </span>
      </div>

      <div className="detail-preview">
        {isImage ? (
          <div className="preview-image-wrap">
            {imgLoading && <div className="preview-image-placeholder">{t("detail.imageLoading")}</div>}
            {imgError && <div className="preview-image-placeholder">{t("detail.imageError")}</div>}
            {imgUrl && !imgLoading && !imgError && (
              <img
                className="preview-image"
                src={imgUrl}
                alt={t("detail.imageAlt")}
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
        ) : item.isSensitive ? (
          <div className="preview-text sensitive-masked">
            {item.preview}
            <span className="sensitive-note">{t("detail.sensitiveMasked")}</span>
          </div>
        ) : (
          <div className={`preview-text${isCode ? " code" : ""}`}>{item.contentText}</div>
        )}
      </div>

      <div className="detail-meta">
        <div className="meta-row">
          <span className="meta-key">{t("detail.metaSource")}</span>
          <span className="meta-val">
            {item.isRemote
              ? `${item.originDevice || t("item.local")}${item.sourceApp ? ` · ${item.sourceApp}` : ""}`
              : (item.sourceApp || t("detail.dash"))}
          </span>
        </div>
        <div className="meta-row">
          <span className="meta-key">{t("detail.metaSize")}</span>
          <span className="meta-val">{formatBytes(item.sizeBytes)}</span>
        </div>
        <div className="meta-row">
          <span className="meta-key">{t("detail.metaTime")}</span>
          <span className="meta-val">{fullDateTime(item.createdAt)}</span>
        </div>
        <div className="meta-row">
          <span className="meta-key">{t("detail.metaHash")}</span>
          <span className="meta-val mono">{item.hash}</span>
        </div>
      </div>

      <div className="detail-actions" ref={actionsRef}>
        <button className="primary" onClick={() => copy(item)}>
          <CopyIcon size={15} /> {t("action.copy")}
          <span className="btn-shortcut">{ACTION_SHORTCUTS.copy}</span>
        </button>
        <button onClick={() => pin(item)}>
          <PinIcon size={15} active={item.isPinned} /> {item.isPinned ? t("action.unpin") : t("action.pin")}
          <span className="btn-shortcut">{ACTION_SHORTCUTS.pin}</span>
        </button>
        <button onClick={() => fav(item)}>
          <StarIcon size={15} active={item.isFavorite} /> {item.isFavorite ? t("action.unfav") : t("action.fav")}
          <span className="btn-shortcut">{ACTION_SHORTCUTS.fav}</span>
        </button>
        <button className="danger" onClick={() => del(item)}>
          <TrashIcon size={15} /> {t("action.delete")}
          <span className="btn-shortcut">{ACTION_SHORTCUTS.del}</span>
        </button>
      </div>
    </aside>
  );
}
