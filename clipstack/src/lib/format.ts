// P3 · 展示格式化与类型元信息。
// 文本（类型名、相对时间、分组标签）均通过 i18n 翻译，随界面语言切换。

import type { ContentType } from "../types";
import { getResolvedLang, translate } from "./i18n";

export interface TypeMeta {
  /** i18n key（如 "text"），显示时用 t(`type.${key}`) 翻译。 */
  label: string;
  /** 主色（用于图标与标签）。 */
  color: string;
}

export const TYPE_META: Record<ContentType, TypeMeta> = {
  text: { label: "text", color: "#059669" },
  link: { label: "link", color: "#2563eb" },
  code: { label: "code", color: "#7c3aed" },
  image: { label: "image", color: "#ea580c" },
  file: { label: "file", color: "#6b7280" },
};

/** 字节数可读化（国际通用单位，不翻译）。 */
export function formatBytes(n: number): string {
  if (n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  const v = n / Math.pow(1024, i);
  return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

const pad = (n: number): string => String(n).padStart(2, "0");

/** 相对时间（i18n）：刚刚 / X 分钟前 / X 小时前 / 今天 HH:MM / 昨天 HH:MM / M月D日。 */
export function relativeTime(ts: number): string {
  const lang = getResolvedLang();
  const diff = Date.now() - ts;
  if (diff < 60_000) return translate(lang, "rel.justnow");
  if (diff < 3_600_000)
    return translate(lang, "rel.minutes", { n: Math.floor(diff / 60_000) });
  if (diff < 86_400_000)
    return translate(lang, "rel.hours", { n: Math.floor(diff / 3_600_000) });

  const d = new Date(ts);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  const yesterday =
    d.getFullYear() === y.getFullYear() &&
    d.getMonth() === y.getMonth() &&
    d.getDate() === y.getDate();

  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (sameDay) return translate(lang, "rel.todayAt", { time: hm });
  if (yesterday) return translate(lang, "rel.yesterdayAt", { time: hm });
  return translate(lang, "rel.date", { m: d.getMonth() + 1, d: d.getDate() });
}

/** 列表分组标签：今天 / 昨天 / M月D日（i18n）。 */
export function dayLabel(ts: number): string {
  const lang = getResolvedLang();
  const d = new Date(ts);
  const now = new Date();
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  const same = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate();
  if (same(d, now)) return translate(lang, "rel.today");
  if (same(d, y)) return translate(lang, "rel.yesterday");
  return translate(lang, "rel.date", { m: d.getMonth() + 1, d: d.getDate() });
}

/** 完整日期时间（详情用）。 */
export function fullDateTime(ts: number): string {
  const d = new Date(ts);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(
    d.getHours(),
  )}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}
