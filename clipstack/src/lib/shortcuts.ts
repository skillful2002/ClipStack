// 快捷键定义与显示工具（跨平台：macOS 用 ⌘/⇧，其它平台用 Ctrl/Shift）。
//
// 用途：仅在「菜单项 / 按钮后显示」快捷键提示。真正的功能由两处负责：
//   - 前端 App.tsx 的 window keydown 监听（分类切换、条目操作）；
//   - Rust 端全局快捷键插件（⌘⇧V 切换主界面，见 src-tauri/src/lib.rs）。
// 此处集中维护显示文案，避免菜单、按钮、托盘三处文案漂移。

const UA =
  typeof navigator !== "undefined"
    ? (navigator.platform || navigator.userAgent || "")
    : "";
export const isMac = /Mac|iPhone|iPad|iPod/.test(UA);

const MOD = isMac ? "⌘" : "Ctrl";
const SHIFT = isMac ? "⇧" : "Shift+";

/** 侧边栏导航项的快捷键显示文案（key 与 Sidebar 的 Category / 底部项对应）。 */
export const NAV_SHORTCUTS: Record<string, string> = {
  all: MOD + "1",
  text: MOD + "2",
  link: MOD + "3",
  code: MOD + "4",
  image: MOD + "5",
  file: MOD + "6",
  settings: MOD + ",",
  trash: MOD + SHIFT + "T",
  search: MOD + "/",
};

/** 主界面条目操作按钮的快捷键显示文案。 */
export const ACTION_SHORTCUTS = {
  copy: "⏎",
  pin: "P",
  fav: "F",
  del: isMac ? "⌫" : "Del",
};

/**
 * 把 Tauri 风格的快捷键字符串（如 "CmdOrCtrl+Shift+V"）转成展示文案（如 "⌘⇧V"）。
 * 供托盘菜单等需要把 accelerator 字符串展示给用户时使用。
 */
export function formatAccelerator(combo: string): string {
  const parts = combo.split("+");
  let out = "";
  for (const p of parts) {
    switch (p) {
      case "CmdOrCtrl":
      case "CommandOrControl":
        out += MOD;
        break;
      case "Super":
      case "Meta":
      case "Cmd":
      case "Command":
        out += "⌘";
        break;
      case "Ctrl":
      case "Control":
        out += "Ctrl";
        break;
      case "Alt":
        out += isMac ? "⌥" : "Alt";
        break;
      case "Shift":
        out += SHIFT;
        break;
      case "Enter":
        out += "⏎";
        break;
      default:
        out += p.length === 1 ? p.toUpperCase() : p;
    }
  }
  return out;
}
