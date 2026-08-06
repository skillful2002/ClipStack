// P3 · 中间主列表：时间筛选标签 + 按日分组的历史条目。

import { useMemo, useState } from "react";
import { useHistory, filterItems, type TimeFilter } from "../store/history";
import { useItemActions } from "../lib/actions";
import { dayLabel } from "../lib/format";
import type { HistoryItem } from "../types";
import { HistoryItemRow } from "./HistoryItemRow";
import { TrashBinIcon } from "./icons";

const TIME_TABS: { key: TimeFilter; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "today", label: "今天" },
  { key: "yesterday", label: "昨天" },
  { key: "week", label: "本周" },
];

export function HistoryList() {
  const items = useHistory((s) => s.items);
  const category = useHistory((s) => s.category);
  const timeFilter = useHistory((s) => s.timeFilter);
  const setTimeFilter = useHistory((s) => s.setTimeFilter);
  const search = useHistory((s) => s.search);
  const selectedId = useHistory((s) => s.selectedId);
  const select = useHistory((s) => s.select);
  const { copy, pin, fav, del, clearAll } = useItemActions();
  const [confirmOpen, setConfirmOpen] = useState(false);

  const filtered = useMemo(
    () => filterItems(items, category, timeFilter, search),
    [items, category, timeFilter, search],
  );

  // 按日分组（保持置顶优先、时间倒序的既有顺序）。
  const groups = useMemo(() => {
    const out: { label: string; items: HistoryItem[] }[] = [];
    for (const it of filtered) {
      const label = dayLabel(it.createdAt);
      const last = out[out.length - 1];
      if (last && last.label === label) last.items.push(it);
      else out.push({ label, items: [it] });
    }
    return out;
  }, [filtered]);

  return (
    <section className="list-pane">
      <div className="list-toolbar">
        <div className="time-tabs">
          {TIME_TABS.map((t) => (
            <button
              key={t.key}
              className={`time-tab${timeFilter === t.key ? " active" : ""}`}
              onClick={() => setTimeFilter(t.key)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <div className="toolbar-right">
          <button
            className="clear-all-btn"
            disabled={items.length === 0}
            onClick={() => setConfirmOpen(true)}
            title="清除全部记录"
          >
            <TrashBinIcon size={14} />
            清除全部
          </button>
          <span className="list-count">{filtered.length} 项</span>
        </div>
      </div>

      <div className="list-body">
        {filtered.length === 0 ? (
          <div className="list-empty">
            {search ? "没有匹配的内容" : "暂无剪贴板记录，复制点什么试试"}
          </div>
        ) : (
          groups.map((g) => (
            <div key={g.label} className="list-group">
              <div className="group-header">{g.label}</div>
              {g.items.map((it) => (
                <HistoryItemRow
                  key={it.id}
                  item={it}
                  selected={it.id === selectedId}
                  onSelect={select}
                  onCopy={copy}
                  onPin={pin}
                  onFav={fav}
                  onDelete={del}
                />
              ))}
            </div>
          ))
        )}
      </div>

      {confirmOpen && (
        <div
          className="confirm-overlay"
          role="dialog"
          aria-modal="true"
          onClick={() => setConfirmOpen(false)}
        >
          <div className="confirm-dialog" onClick={(e) => e.stopPropagation()}>
            <div className="confirm-title">清除全部记录</div>
            <div className="confirm-body">
              将把全部 {items.length} 条剪贴板记录移入回收站（可在回收站恢复），此操作不可撤销。
            </div>
            <div className="confirm-actions">
              <button
                className="confirm-btn cancel"
                onClick={() => setConfirmOpen(false)}
              >
                取消
              </button>
              <button
                className="confirm-btn danger"
                onClick={() => {
                  setConfirmOpen(false);
                  void clearAll();
                }}
              >
                清除全部
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
