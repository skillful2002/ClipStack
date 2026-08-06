// P6 · 主题工具：将设置中的主题（light/dark/system）应用到根元素 data-theme。
//
// 跟随系统时**必须**使用 Tauri 原生窗口主题（getCurrentWindow().theme()），
// 而非 window.matchMedia("(prefers-color-scheme: dark)")：
//   1. Tauri 的 webview 对 prefers-color-scheme 的探测并不可靠（常返回 light），
//      这会导致「跟随系统」在系统为深色时仍显示浅色；
//   2. matchMedia 的 change 事件只在设置页打开时监听，关闭后无法实时跟随系统切换。
// 原生 theme() 是窗口/系统的权威外观值，并配合 onThemeChanged 实时更新。

import { getCurrentWindow } from "@tauri-apps/api/window";

export type Theme = "light" | "dark" | "system";

// 当前设置的主题（用于判断是否需要跟随系统实时更新）。默认为跟随系统。
let currentTheme: Theme = "system";
// 最近一次探测到的系统真实明暗（供同步解析使用）。
let systemTheme: "light" | "dark" = "light";

/** 读取系统真实明暗：优先 Tauri 原生窗口主题，失败时回退到浏览器媒体查询。 */
async function readSystemTheme(): Promise<"light" | "dark"> {
  try {
    const t = await getCurrentWindow().theme();
    if (t === "dark" || t === "light") return t;
  } catch {
    /* 非 Tauri 环境或权限不足，走下方回退 */
  }
  try {
    return window.matchMedia?.("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  } catch {
    return "light";
  }
}

/** 把主题解析为实际明暗（跟随系统时使用最近一次探测结果）。 */
export function resolveTheme(theme: Theme): "light" | "dark" {
  return theme === "system" ? systemTheme : theme;
}

/** 同步原生窗口外观（标题栏等）：手动模式设固定值，跟随系统则传 null 交还系统。 */
async function setWindowTheme(theme: "light" | "dark" | null): Promise<void> {
  try {
    await getCurrentWindow().setTheme(theme);
  } catch {
    /* 不支持或权限不足时忽略，仅影响原生标题栏外观，不影响内容主题 */
  }
}

/** 应用主题到 <html data-theme>，并同步原生窗口外观。 */
export async function applyTheme(theme: Theme): Promise<void> {
  currentTheme = theme;
  if (theme === "system") {
    await setWindowTheme(null); // 窗口交还系统，跟随 OS 外观
    systemTheme = await readSystemTheme();
  } else {
    await setWindowTheme(theme); // 手动模式同步原生标题栏
  }
  document.documentElement.setAttribute("data-theme", resolveTheme(theme));
}

/**
 * 订阅系统主题变化：仅在「跟随系统」时实时更新界面。
 * 只需在应用启动时调用一次，返回取消订阅函数。
 */
export async function watchSystemTheme(): Promise<() => void> {
  try {
    return await getCurrentWindow().onThemeChanged(({ payload: theme }) => {
      systemTheme = theme === "dark" ? "dark" : "light";
      if (currentTheme === "system") {
        document.documentElement.setAttribute("data-theme", systemTheme);
      }
    });
  } catch {
    return () => {};
  }
}
