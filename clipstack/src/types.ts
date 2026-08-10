// P3 · 前端类型定义（字段名与后端 camelCase 序列化一致，无需额外转换）。

/** 内容类型（文本 / 链接 / 代码 / 图片 / 文件）。 */
export type ContentType = "text" | "link" | "code" | "image" | "file";

/** 历史条目：持久化行、事件负载、命令返回的同一形状。 */
export interface HistoryItem {
  id: number;
  contentType: ContentType;
  /** 文本 / 链接 / 代码为原文；文件为路径拼接；图片为 "WxH 图片"。 */
  contentText: string;
  /** 展示用预览（长文本截断）。 */
  preview: string;
  /** 是否命中敏感内容识别（启用掩码时预览被遮挡）。 */
  isSensitive: boolean;
  sourceApp: string;
  sizeBytes: number;
  hash: string;
  isPinned: boolean;
  isFavorite: boolean;
  /** 毫秒时间戳。 */
  createdAt: number;
  /** 来源设备名：本地捕获为空字符串；来自局域网共享的对端条目填对端设备名。 */
  originDevice?: string;
  /** 是否来自局域网共享（对端设备）；本地捕获为 false / undefined。 */
  isRemote?: boolean;
  /** 删除时间（仅回收站条目有值，主列表为 undefined）。 */
  deletedAt?: number;
}

/** 设置项（key / value 均为字符串）。 */
export interface Setting {
  key: string;
  value: string;
}
