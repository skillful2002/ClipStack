// i18n 类型定义。

/** 可选语言：system 表示跟随系统，其余为具体语言。 */
export type Language = "system" | "zh-CN" | "zh-TW" | "en" | "ja" | "de" | "fr";

/** 实际生效的语言（不含 system）。 */
export type ResolvedLang = Exclude<Language, "system">;

/** 设置下拉中展示的语言项（label 用各语言原生名，不随界面翻译）。 */
export interface LanguageOption {
  key: Language;
  /** 原生名（无论界面语言如何都固定显示）。 */
  native: string;
}
