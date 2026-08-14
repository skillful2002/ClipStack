// P3 · 右侧详情面板：类型标签、内容预览、元数据、操作区。
// P8 · 图片预览：选中图片时按 id 拉取 PNG 二进制，生成对象 URL 渲染 <img>。

import { useEffect, useState, useRef, useLayoutEffect, useMemo } from "react";
import { useHistory } from "../store/history";
import { useItemActions } from "../lib/actions";
import { getItemBlob } from "../lib/tauri";
import { TYPE_META, formatBytes, fullDateTime } from "../lib/format";
import { useT } from "../lib/i18n";
import { ACTION_SHORTCUTS } from "../lib/shortcuts";
import { TypeIcon, CopyIcon, PinIcon, StarIcon, TrashIcon, SaveIcon, ExternalLinkIcon } from "./icons";
import hljs from "highlight.js/lib/common";
import "highlight.js/styles/github-dark.css";

const escapeHtml = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export function DetailPanel() {
  const t = useT();
  const item = useHistory((s) => s.items.find((i) => i.id === s.selectedId) ?? null);
  const { copy, pin, fav, del, save, open } = useItemActions();

  // 代码详情：用 highlight.js 自动识别语言并生成带颜色的 HTML。
  // 必须置于 early return 之前，保证每次渲染 hook 调用顺序一致。
  const codeHtml = useMemo(() => {
    if (item?.contentType !== "code") return "";
    try {
      return hljs.highlightAuto(item.contentText ?? "").value;
    } catch {
      return escapeHtml(item.contentText ?? "");
    }
  }, [item?.contentType, item?.contentText]);

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
  // 单击图片后全屏查看大图的浮层开关。
  const [lightboxOpen, setLightboxOpen] = useState(false);

  // ESC 关闭大图浮层。
  useEffect(() => {
    if (!lightboxOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setLightboxOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [lightboxOpen]);

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
              <>
                <img
                  className="preview-image"
                  src={imgUrl}
                  alt={t("detail.imageAlt")}
                  title={t("detail.clickZoom")}
                  onError={() => setImgError(true)}
                  onClick={() => setLightboxOpen(true)}
                />
                <p className="preview-image-hint">{t("detail.clickZoom")}</p>
              </>
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
        ) : isCode ? (
          <div
            className="preview-text code hljs"
            dangerouslySetInnerHTML={{ __html: codeHtml }}
          />
        ) : (
          <div className="preview-text">{item.contentText}</div>
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
        {(item.contentType === "image" || item.contentType === "file") && (
          <button onClick={() => save(item)} title={t("action.save")}>
            <SaveIcon size={16} />
          </button>
        )}
        {item.contentType === "link" && (
          <button onClick={() => open(item)} title={t("action.open")}>
            <ExternalLinkIcon size={16} />
          </button>
        )}
        <button className="primary" onClick={() => copy(item)} title={`${t("action.copy")}  ${ACTION_SHORTCUTS.copy}`}>
          <CopyIcon size={16} />
        </button>
        <button onClick={() => pin(item)} title={`${item.isPinned ? t("action.unpin") : t("action.pin")}  ${ACTION_SHORTCUTS.pin}`}>
          <PinIcon size={16} active={item.isPinned} />
        </button>
        <button onClick={() => fav(item)} title={`${item.isFavorite ? t("action.unfav") : t("action.fav")}  ${ACTION_SHORTCUTS.fav}`}>
          <StarIcon size={16} active={item.isFavorite} />
        </button>
        <button className="danger" onClick={() => del(item)} title={`${t("action.delete")}  ${ACTION_SHORTCUTS.del}`}>
          <TrashIcon size={16} />
        </button>
      </div>

      {lightboxOpen && imgUrl && (
        <div
          className="image-lightbox"
          role="dialog"
          aria-label={t("detail.imageZoom")}
          onClick={() => setLightboxOpen(false)}
        >
          <img
            className="image-lightbox-img"
            src={imgUrl}
            alt={t("detail.imageAlt")}
            onClick={(e) => e.stopPropagation()}
          />
          <span className="image-lightbox-hint">{t("detail.imageZoomClose")}</span>
        </div>
      )}
    </aside>
  );
}
