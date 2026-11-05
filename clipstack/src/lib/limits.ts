// 历史上限 / 托盘历史条数的取值范围。
//
// 与 Rust 端 `db::MIN_MAX_HISTORY` / `db::MAX_MAX_HISTORY` /
// `db::DEFAULT_MAX_HISTORY` 及 `tray::DEFAULT_TRAY_HISTORY` 保持一致，
// 避免设置页、列表拉取与后端清理三处各写一套数字。

/** 历史上限（条）：最小值。 */
export const MIN_MAX_HISTORY = 50;
/** 历史上限（条）：最大值（后端 `enforce_capacity` 的硬上限）。 */
export const MAX_MAX_HISTORY = 5000;
/** 历史上限（条）：缺省值（未设置时后端按此清理、前端按此拉取）。 */
export const DEFAULT_MAX_HISTORY = 1000;

/** 托盘菜单历史条数：区间与缺省值。 */
export const MIN_TRAY_HISTORY = 1;
export const MAX_TRAY_HISTORY = 100;
export const DEFAULT_TRAY_HISTORY = 30;

/** 把任意输入归一到合法的历史上限：非正数 / 非法值回退缺省，其余夹到区间内。 */
export function clampMaxHistory(v: number): number {
  if (!Number.isFinite(v) || v <= 0) return DEFAULT_MAX_HISTORY;
  return Math.max(MIN_MAX_HISTORY, Math.min(MAX_MAX_HISTORY, Math.round(v)));
}

/** 把任意输入归一到合法的托盘历史条数。 */
export function clampTrayHistory(v: number): number {
  if (!Number.isFinite(v) || v <= 0) return DEFAULT_TRAY_HISTORY;
  return Math.max(MIN_TRAY_HISTORY, Math.min(MAX_TRAY_HISTORY, Math.round(v)));
}
