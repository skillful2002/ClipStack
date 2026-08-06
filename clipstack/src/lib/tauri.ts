// P3 · Tauri 命令 / 事件封装层。
//
// 约定：Tauri 命令的「顶层参数名」需与 Rust 函数形参一致（snake_case），
// 而结构体内部字段遵循 serde rename（camelCase）。例如 `copy_item` 传
// { content_type, content_text }，NewItem 等嵌套结构其字段为 camelCase。

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ContentType, HistoryItem, Setting } from "../types";

/** 读取历史（默认 500 条、置顶优先、时间倒序）。 */
export const getHistory = (limit?: number): Promise<HistoryItem[]> =>
  invoke<HistoryItem[]>("get_history", limit != null ? { limit } : {});

/** 删除条目（移入回收站，后端处理）。 */
export const deleteItem = (id: number): Promise<void> =>
  invoke<void>("delete_item", { id });

/** 读取回收站（按删除时间倒序）。 */
export const getTrash = (): Promise<HistoryItem[]> =>
  invoke<HistoryItem[]>("get_trash");

/** 恢复：从回收站移回历史。 */
export const restoreItem = (id: number): Promise<void> =>
  invoke<void>("restore_item", { id });

/** 彻底删除：从回收站永久移除。 */
export const purgeItem = (id: number): Promise<void> =>
  invoke<void>("purge_item", { id });

/** 清空回收站。 */
export const emptyTrash = (): Promise<void> => invoke<void>("empty_trash");

/** 清空全部历史（软删入回收站，可回收站恢复）。 */
export const clearAllHistory = (): Promise<void> => invoke<void>("clear_history");

/** 切换置顶，返回切换后状态。 */
export const togglePin = (id: number): Promise<boolean> =>
  invoke<boolean>("toggle_pin", { id });

/** 切换收藏，返回切换后状态。 */
export const toggleFavorite = (id: number): Promise<boolean> =>
  invoke<boolean>("toggle_favorite", { id });

/** 一键复制：将条目内容写回系统剪贴板（文本 / 链接 / 代码）。 */
export const copyItem = (
  contentType: ContentType,
  contentText: string,
): Promise<void> =>
  invoke<void>("copy_item", { contentType, contentText });

/** 读取全部设置项。 */
export const getSettings = (): Promise<Setting[]> =>
  invoke<Setting[]>("get_settings");

/** 写入 / 覆盖单个设置项。 */
export const updateSetting = (key: string, value: string): Promise<void> =>
  invoke<void>("update_setting", { key, value });

/** 读取全部忽略应用名（小写）。 */
export const getIgnoredApps = (): Promise<string[]> =>
  invoke<string[]>("get_ignored_apps");

/** 将来源应用加入忽略列表（即时生效 + 持久化）。 */
export const addIgnoredApp = (name: string): Promise<void> =>
  invoke<void>("add_ignored_app", { name });

/** 从忽略列表移除应用（即时生效 + 持久化）。 */
export const removeIgnoredApp = (name: string): Promise<void> =>
  invoke<void>("remove_ignored_app", { name });

/** 枚举系统中已安装应用的显示名（小写），供忽略应用从系统列表选择。 */
export const listInstalledApps = (): Promise<string[]> =>
  invoke<string[]>("list_installed_apps");

/** 读取条目的二进制内容（图片为 PNG 字节），用于详情面板预览。 */
export const getItemBlob = (id: number): Promise<Uint8Array<ArrayBuffer>> =>
  invoke<ArrayBuffer>("get_item_blob", { id }).then((b) => new Uint8Array(b));

/** 读取回收站条目的二进制内容（图片为 PNG 字节），用于回收站详情预览。 */
export const getTrashBlob = (id: number): Promise<Uint8Array<ArrayBuffer>> =>
  invoke<ArrayBuffer>("get_trash_blob", { id }).then((b) => new Uint8Array(b));

/** 订阅剪贴板变更事件，返回取消订阅函数。 */
export const onClipboardChanged = (
  cb: (item: HistoryItem) => void,
): Promise<UnlistenFn> =>
  listen<HistoryItem>("clipboard-changed", (event) => cb(event.payload));

/** 托盘 / 全局快捷键触发的视图切换（"all" 回到主界面，"settings" 打开设置，"about" 打开关于）。 */
export const onShowView = (
  cb: (view: "all" | "settings" | "about") => void,
): Promise<UnlistenFn> =>
  listen<"all" | "settings" | "about">("show-view", (event) => cb(event.payload));

/** 托盘菜单点击「复制」后触发：携带被复制条目 id，便于前端选中 + 提示。 */
export const onTrayCopied = (
  cb: (id: number) => void,
): Promise<UnlistenFn> =>
  listen<{ id: number }>("tray-copied", (event) => cb(event.payload.id));

/** 关于系统：读取运行平台与处理器架构。 */
export interface SystemInfo {
  platform: string;
  arch: string;
}

/** 关于系统：返回平台与架构信息。 */
export const getSystemInfo = (): Promise<SystemInfo> =>
  invoke<SystemInfo>("get_system_info");
