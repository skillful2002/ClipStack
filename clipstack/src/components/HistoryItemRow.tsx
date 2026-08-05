// P3 · 历史条目行（列表中的单条）。

import type { HistoryItem } from "../types";
import { TYPE_META, relativeTime } from "../lib/format";
import { TypeIcon, CopyIcon, PinIcon, StarIcon, TrashIcon } from "./icons";

interface Props {
  item: HistoryItem;
  selected: boolean;
  onSelect: (id: number) => void;
  onCopy: (item: HistoryItem) => void;
  onPin: (item: HistoryItem) => void;
  onFav: (item: HistoryItem) => void;
  onDelete: (item: HistoryItem) => void;
}

export function HistoryItemRow({
  item,
  selected,
  onSelect,
  onCopy,
  onPin,
  onFav,
  onDelete,
}: Props) {
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
        <div className="item-preview">{item.preview || "（空内容）"}</div>
        <div className="item-sub">
          <span className="item-app">{item.sourceApp || "未知来源"}</span>
          <span className="item-dot">·</span>
          <span>{relativeTime(item.createdAt)}</span>
        </div>
      </div>

      <div className="item-badges">
        {item.isPinned && <PinIcon size={14} active />}
        {item.isFavorite && <StarIcon size={14} active />}
      </div>

      <div className="item-actions" onClick={(e) => e.stopPropagation()}>
        <button title="复制" onClick={() => onCopy(item)}>
          <CopyIcon size={15} />
        </button>
        <button title={item.isPinned ? "取消置顶" : "置顶"} onClick={() => onPin(item)}>
          <PinIcon size={15} active={item.isPinned} />
        </button>
        <button title={item.isFavorite ? "取消收藏" : "收藏"} onClick={() => onFav(item)}>
          <StarIcon size={15} active={item.isFavorite} />
        </button>
        <button title="删除" className="danger" onClick={() => onDelete(item)}>
          <TrashIcon size={15} />
        </button>
      </div>
    </div>
  );
}
