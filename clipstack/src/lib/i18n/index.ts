// i18n 运行时：语言状态、系统语言检测、翻译函数与 React hook。
//
// 用法：
//   const t = useT();            // 订阅语言变化，返回 t(key, params?)
//   const { lang, setLang } = useI18nStore();  // 读写当前语言（含 system）
//   getResolvedLang()           // 非 hook 场景（如 format.ts）获取实际语言

import { create } from "zustand";
import { translations } from "./translations";
import type { Language, LanguageOption, ResolvedLang } from "./types";

export type { Language, ResolvedLang, LanguageOption } from "./types";

/** 设置下拉的语言列表（native 为各语言原生名，不随界面翻译）。 */
export const LANGUAGE_OPTIONS: LanguageOption[] = [
  { key: "system", native: "系统 / System" },
  { key: "zh-CN", native: "简体中文" },
  { key: "zh-TW", native: "繁體中文" },
  { key: "en", native: "English" },
  { key: "ja", native: "日本語" },
  { key: "de", native: "Deutsch" },
  { key: "fr", native: "Français" },
];

/** 根据浏览器 / 系统区域解析实际语言。 */
function resolveFromBrowser(): ResolvedLang {
  const nav =
    typeof navigator !== "undefined" ? navigator.language : "en-US";
  const lower = (nav || "en-US").toLowerCase();
  if (lower.startsWith("zh")) {
    return lower.includes("tw") || lower.includes("hk") || lower.includes("mo")
      ? "zh-TW"
      : "zh-CN";
  }
  if (lower.startsWith("ja")) return "ja";
  if (lower.startsWith("de")) return "de";
  if (lower.startsWith("fr")) return "fr";
  return "en";
}

/** 将（可能含 system 的）语言解析为实际生效语言。 */
export function detect(lang: Language): ResolvedLang {
  return lang === "system" ? resolveFromBrowser() : lang;
}

interface I18nState {
  lang: Language;
  setLang: (l: Language) => void;
}

export const useI18nStore = create<I18nState>((set) => ({
  lang: "system",
  setLang: (lang) => set({ lang }),
}));

/** 非 hook 场景获取当前实际语言。 */
export function getResolvedLang(): ResolvedLang {
  return detect(useI18nStore.getState().lang);
}

type Params = Record<string, string | number>;

/** 纯翻译函数（指定语言）。缺失键回退到 zh-CN，再回退到 key 本身。 */
export function translate(
  lang: ResolvedLang,
  key: string,
  params?: Params,
): string {
  let s = translations[lang]?.[key] ?? translations["zh-CN"][key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      s = s.replace(new RegExp(`\\{${k}\\}`, "g"), String(v));
    }
  }
  return s;
}

/**
 * 组件内使用的翻译 hook：返回 t(key, params?)。
 * 订阅语言变化，语言切换时用到 t 的组件会自动重渲染。
 */
export function useT() {
  const lang = useI18nStore((s) => s.lang);
  const resolved = detect(lang);
  return (key: string, params?: Params) => translate(resolved, key, params);
}
