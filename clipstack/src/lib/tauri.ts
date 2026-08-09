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

/** 按 id 批量删除（软删入回收站，可回收站恢复）。用于「按当前查询条件清除」。 */
export const deleteItems = (ids: number[]): Promise<number> =>
  invoke<number>("delete_items", { ids });

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

/** 一键复制图片：从数据库读取 PNG 二进制，解码后写回系统剪贴板。 */
export const copyImage = (id: number): Promise<void> =>
  invoke<void>("copy_image", { id });

/** 一键复制文件：从数据库读取路径列表，写回系统剪贴板文件列表（可粘贴为文件本身）。 */
export const copyFile = (id: number): Promise<void> =>
  invoke<void>("copy_file", { id });

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

/** 托盘 / 全局快捷键触发的视图切换（"all" 回到主界面，"settings" 打开设置，"about" 打开关于，"help" 打开帮助）。 */
export const onShowView = (
  cb: (view: "all" | "settings" | "about" | "help") => void,
): Promise<UnlistenFn> =>
  listen<"all" | "settings" | "about" | "help">("show-view", (event) => cb(event.payload));

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

/** 首次运行判定：窗口显示与标记写入已在启动阶段同步完成；此处仅读取标志，供前端决定是否进入设置页。 */
export const wasFirstRun = (): Promise<boolean> =>
  invoke<boolean>("was_first_run");

// ===== P0 · 应用锁 / 主密码命令封装 =====

/** 是否已设置主密码（前端据此显示「设置」还是「解锁 / 修改」）。 */
export const hasMasterPassword = (): Promise<boolean> =>
  invoke<boolean>("has_master_password");

/** 当前是否锁定。 */
export const isLocked = (): Promise<boolean> => invoke<boolean>("is_locked");

/** 首次设置主密码：仅写入校验信息；数据库加密由内部密钥负责（与主密码无关）。 */
export const setupMasterPassword = (pwd: string): Promise<void> =>
  invoke<void>("setup_master_password", { pwd });

/** 主密码解锁：校验通过后解除应用锁（数据库内部密钥始终在内存，不载入加解密密钥）。 */
export const unlock = (pwd: string): Promise<boolean> =>
  invoke<boolean>("unlock", { pwd });

/** 钥匙串 + Touch ID 解锁：触发生物识别，成功后解除应用锁（非 macOS 回退失败）。 */
export const unlockTouchId = (): Promise<boolean> =>
  invoke<boolean>("unlock_touch_id");

/** 启用 / 关闭 Touch ID 解锁：写入受 BiometryCurrentSet 保护的随机令牌（仅用于生物识别校验，不参与加解密）。 */
export const setTouchId = (enabled: boolean): Promise<void> =>
  invoke<void>("set_touch_id", { enabled });

/** 立即锁定：仅置应用锁标记，不清空数据库加密密钥。 */
export const lockApp = (): Promise<void> => invoke<void>("lock");

/** 上报一次用户活动：重置后端「闲置自动锁定」计时（前端已做限流调用）。 */
export const touchActivity = (): Promise<void> => invoke<void>("touch_activity");

/** 修改主密码：校验原密码后更新校验信息（不影响数据库加密，不重加密数据）。 */
export const changeMasterPassword = (
  oldPwd: string,
  newPwd: string,
): Promise<void> =>
  invoke<void>("change_master_password", { oldPwd, newPwd });

/** 清除主密码：直接移除应用锁凭据与 Touch ID 设置（无需旧密码）；数据库仍以内部密钥加密（不解密）。 */
export const clearMasterPassword = (): Promise<void> =>
  invoke<void>("clear_master_password");

/** 检测 Touch ID 解锁是否可用（需要安装 Xcode Command Line Tools 才能运行 swift 子进程）。 */
export const checkTouchIdAvailable = (): Promise<boolean> =>
  invoke<boolean>("check_touch_id_available");

/** 订阅应用锁状态变化（后端自动锁触发，payload=true 表示已锁定）。 */
export const onAppLockChanged = (
  cb: (locked: boolean) => void,
): Promise<UnlistenFn> =>
  listen<boolean>("app-lock-changed", (event) => cb(event.payload));

// ===== P1b · 留存过期命令封装 =====

/** 立即按 retention_days 清理超期历史（未置顶）与回收站内容，返回删除条数。 */
export const purgeExpired = (): Promise<number> =>
  invoke<number>("purge_expired");
