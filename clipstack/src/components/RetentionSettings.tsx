// P1b · 留存过期设置：按 retention_days 自动清理超期历史与回收站内容。
//
// 默认 "0" = 永久保留；非 0 时后端每 10 分钟定时清理（由 retention-sweep
// 线程执行），并提供手动「立即清理」按钮即时触发一次 purge_expired。

import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import { useHistory } from "../store/history";
import { useT } from "../lib/i18n";

// 可选项（天）：1 / 2 / 7 / 30 / 90 / 180 / 365 / 永久(0)。永久置于列表末尾。
const RETENTION_OPTIONS = [1, 2, 7, 30, 90, 180, 365, 0];

export function RetentionSettings() {
  const t = useT();
  const setToast = useHistory((s) => s.setToast);
  const [days, setDays] = useState(0);
  const [busy, setBusy] = useState(false);

  // 初始化：读取留存天数设置。
  useEffect(() => {
    void (async () => {
      try {
        const settings = await api.getSettings();
        const d = Number(settings.find((s) => s.key === "retention_days")?.value);
        if (!Number.isNaN(d)) setDays(d);
      } catch {
        /* 读取失败静默，使用默认值 */
      }
    })();
  }, []);

  const onRetentionChange = async (d: number) => {
    setDays(d);
    try {
      await api.updateSetting("retention_days", String(d));
      setToast(t("retention.saved"));
    } catch (e) {
      setToast(String(e));
    }
  };

  const onPurgeNow = async () => {
    setBusy(true);
    try {
      const n = await api.purgeExpired();
      setToast(t("retention.purged", { n }));
    } catch (e) {
      setToast(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <div className="settings-subtitle">{t("retention.title")}</div>
      <p className="settings-hint">{t("retention.hint")}</p>
      <div className="settings-row">
        <span>{t("retention.label")}</span>
        <select
          className="app-select"
          value={days}
          onChange={(e) => void onRetentionChange(Number(e.target.value))}
        >
          {RETENTION_OPTIONS.map((d) => (
            <option key={d} value={d}>
              {d === 0 ? t("retention.forever") : t("retention.days", { n: d })}
            </option>
          ))}
        </select>
      </div>
      <button className="sec-btn" disabled={busy} onClick={() => void onPurgeNow()}>
        {t("retention.cleanNow")}
      </button>
    </>
  );
}
