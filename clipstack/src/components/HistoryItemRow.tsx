// P3 · 历史条目行（列表中的单条）。

import type { HistoryItem } from "../types";
import { TYPE_META, relativeTime } from "../lib/format";
import { useT } from "../lib/i18n";
import { ACTION_SHORTCUTS } from "../lib/shortcuts";
import { TypeIcon, CopyIcon, PinIcon, StarIcon, TrashIcon, SaveIcon } from "./icons";

interface Props {
  item: HistoryItem;
  selected: boolean;
  onSelect: (id: number) => void;
  onCopy: (item: HistoryItem) => void;
  onSave: (item: HistoryItem) => void;
  onPin: (item: HistoryItem) => void;
  onFav: (item: HistoryItem) => void;
  onDelete: (item: HistoryItem) => void;
}

export function HistoryItemRow({
  item,
  selected,
  onSelect,
  onCopy,
  onSave,
  onPin,
  onFav,
  onDelete,
}: Props) {
  const t = useT();
  const meta = TYPE_META[item.contentType];
  return (
    <div
      className={`item-row${selected ? " selected" : ""}`}
      onClick={() => onSelect(item.id)}
    >
      <span className="item-type" style={{ color: meta.color, background: `${meta.color}1a` }}>
        <TypeIcon type={item.contentType} size={16} />
      </span>

      <div className="item-main">
        <div className="item-preview">{item.preview || t("item.emptyContent")}</div>
        <div className="item-sub">
          {item.isRemote ? (
            <span
              className="item-remote-badge"
              title={t("lan.receivedFrom", { name: item.originDevice ?? "" })}
            >
              {t("lan.title")} · {item.originDevice || t("item.local")}
              {item.sourceApp ? ` · ${item.sourceApp}` : ""}
            </span>
          ) : (
            <span className="item-app">{item.sourceApp || t("item.unknownSource")}</span>
          )}
          <span className="item-dot">·</span>
          <span>{relativeTime(item.createdAt)}</span>
        </div>
      </div>

      <div className="item-badges">
        {item.isPinned && <PinIcon size={14} active />}
        {item.isFavorite && <StarIcon size={14} active />}
        {item.isSensitive && (
          <span className="item-sensitive" title={t("item.sensitiveHint")}>
            {t("item.sensitive")}
          </span>
        )}
      </div>

      <div className="item-actions" onClick={(e) => e.stopPropagation()}>
        <button title={`${t("action.copy")}  ${ACTION_SHORTCUTS.copy}`} onClick={() => onCopy(item)}>
          <CopyIcon size={15} />
        </button>
        {(item.contentType === "image" || item.contentType === "file") && (
          <button title={t("action.save")} onClick={() => onSave(item)}>
            <SaveIcon size={15} />
          </button>
        )}
        <button title={`${item.isPinned ? t("action.unpin") : t("action.pin")}  ${ACTION_SHORTCUTS.pin}`} onClick={() => onPin(item)}>
          <PinIcon size={15} active={item.isPinned} />
        </button>
        <button title={`${item.isFavorite ? t("action.unfav") : t("action.fav")}  ${ACTION_SHORTCUTS.fav}`} onClick={() => onFav(item)}>
          <StarIcon size={15} active={item.isFavorite} />
        </button>
        <button title={`${t("action.delete")}  ${ACTION_SHORTCUTS.del}`} className="danger" onClick={() => onDelete(item)}>
          <TrashIcon size={15} />
        </button>
      </div>
    </div>
  );
}
