// P6 · 主题工具：将设置中的主题（light/dark/system）应用到根元素 data-theme。

export type Theme = "light" | "dark" | "system";

/** 把 system 解析为当前系统实际明暗。 */
export function resolveTheme(theme: Theme): "light" | "dark" {
  if (theme === "system") {
    return window.matchMedia?.("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  }
  return theme;
}

/** 应用到 <html data-theme>，样式见 app.css 的 [data-theme="dark"]。 */
export function applyTheme(theme: Theme): void {
  document.documentElement.setAttribute("data-theme", resolveTheme(theme));
}
