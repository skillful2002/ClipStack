// P3 · 历史条目行（列表中的单条）。

import type { HistoryItem } from "../types";
import { TYPE_META, relativeTime } from "../lib/format";
import { useT } from "../lib/i18n";
import { ACTION_SHORTCUTS } from "../lib/shortcuts";
import { TypeIcon, CopyIcon, PinIcon, StarIcon, TrashIcon, SaveIcon, ExternalLinkIcon } from "./icons";
import { Tooltip } from "./Tooltip";

interface Props {
  item: HistoryItem;
  selected: boolean;
  onSelect: (id: number) => void;
  onCopy: (item: HistoryItem) => void;
  onSave: (item: HistoryItem) => void;
  onPin: (item: HistoryItem) => void;
  onFav: (item: HistoryItem) => void;
  onDelete: (item: HistoryItem) => void;
  onOpen: (item: HistoryItem) => void;
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
  onOpen,
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
        {item.isPinned && (
          <Tooltip label={t("action.unpin")}>
            <PinIcon size={14} active />
          </Tooltip>
        )}
        {item.isFavorite && (
          <Tooltip label={t("action.unfav")}>
            <StarIcon size={14} active />
          </Tooltip>
        )}
        {item.isSensitive && (
          <Tooltip label={t("item.sensitiveHint")}>
            <span className="item-sensitive">{t("item.sensitive")}</span>
          </Tooltip>
        )}
      </div>

      <div className="item-actions" onClick={(e) => e.stopPropagation()}>
        <Tooltip label={`${t("action.copy")}  ${ACTION_SHORTCUTS.copy}`}>
          <button onClick={() => onCopy(item)} aria-label={t("action.copy")}>
            <CopyIcon size={15} />
          </button>
        </Tooltip>
        {(item.contentType === "image" || item.contentType === "file") && (
          <Tooltip label={t("action.save")}>
            <button onClick={() => onSave(item)} aria-label={t("action.save")}>
              <SaveIcon size={15} />
            </button>
          </Tooltip>
        )}
        {item.contentType === "link" && (
          <Tooltip label={t("action.open")}>
            <button onClick={() => onOpen(item)} aria-label={t("action.open")}>
              <ExternalLinkIcon size={15} />
            </button>
          </Tooltip>
        )}
        <Tooltip label={`${item.isPinned ? t("action.unpin") : t("action.pin")}  ${ACTION_SHORTCUTS.pin}`}>
          <button onClick={() => onPin(item)} aria-label={item.isPinned ? t("action.unpin") : t("action.pin")}>
            <PinIcon size={15} active={item.isPinned} />
          </button>
        </Tooltip>
        <Tooltip label={`${item.isFavorite ? t("action.unfav") : t("action.fav")}  ${ACTION_SHORTCUTS.fav}`}>
          <button onClick={() => onFav(item)} aria-label={item.isFavorite ? t("action.unfav") : t("action.fav")}>
            <StarIcon size={15} active={item.isFavorite} />
          </button>
        </Tooltip>
        <Tooltip label={`${t("action.delete")}  ${ACTION_SHORTCUTS.del}`}>
          <button onClick={() => onDelete(item)} className="danger" aria-label={t("action.delete")}>
            <TrashIcon size={15} />
          </button>
        </Tooltip>
      </div>
    </div>
  );
}
