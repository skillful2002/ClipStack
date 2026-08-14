// P3 · 中间主列表：时间筛选标签 + 按日分组的历史条目。

import { useMemo, useState } from "react";
import { useHistory, filterItems, type TimeFilter } from "../store/history";
import { useItemActions } from "../lib/actions";
import { dayLabel } from "../lib/format";
import { useT } from "../lib/i18n";
import type { HistoryItem } from "../types";
import { HistoryItemRow } from "./HistoryItemRow";
import { TrashBinIcon } from "./icons";

const TIME_TABS: { key: TimeFilter; labelKey: string }[] = [
  { key: "all", labelKey: "timefilter.all" },
  { key: "today", labelKey: "timefilter.today" },
  { key: "yesterday", labelKey: "timefilter.yesterday" },
  { key: "week", labelKey: "timefilter.week" },
];

export function HistoryList() {
  const t = useT();
  const items = useHistory((s) => s.items);
  const category = useHistory((s) => s.category);
  const timeFilter = useHistory((s) => s.timeFilter);
  const setTimeFilter = useHistory((s) => s.setTimeFilter);
  const search = useHistory((s) => s.search);
  const sourceFilter = useHistory((s) => s.sourceFilter);
  const selectedId = useHistory((s) => s.selectedId);
  const select = useHistory((s) => s.select);
  const { copy, pin, fav, del, save, clearFiltered } = useItemActions();
  const [confirmOpen, setConfirmOpen] = useState(false);

  const filtered = useMemo(
    () => filterItems(items, category, timeFilter, search, sourceFilter),
    [items, category, timeFilter, search, sourceFilter],
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
          {TIME_TABS.map((tt) => (
            <button
              key={tt.key}
              className={`time-tab${timeFilter === tt.key ? " active" : ""}`}
              onClick={() => setTimeFilter(tt.key)}
            >
              {t(tt.labelKey)}
            </button>
          ))}
        </div>
        <div className="toolbar-right">
          <button
            className="clear-all-btn"
            disabled={filtered.length === 0}
            onClick={() => setConfirmOpen(true)}
            title={t("list.clearAllTooltip")}
          >
            <TrashBinIcon size={14} />
            {t("list.clearAllButton")}
          </button>
          <span className="list-count">{t("list.itemsCount", { n: filtered.length })}</span>
        </div>
      </div>

      <div className="list-body">
        {filtered.length === 0 ? (
          <div className="list-empty">
            {search ? t("list.emptySearch") : t("list.emptyDefault")}
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
                  onSave={save}
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
            <div className="confirm-title">{t("confirm.clearTitle")}</div>
            <div className="confirm-body">
              {t("confirm.clearBody", { n: filtered.length })}
            </div>
            <div className="confirm-actions">
              <button
                className="confirm-btn cancel"
                onClick={() => setConfirmOpen(false)}
              >
                {t("confirm.cancel")}
              </button>
              <button
                className="confirm-btn danger"
                onClick={() => {
                  setConfirmOpen(false);
                  void clearFiltered(filtered.map((i) => i.id));
                }}
              >
                {t("confirm.clearConfirm")}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
