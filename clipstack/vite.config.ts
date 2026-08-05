import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @tauri-apps/cli 在 dev 时通过 TAURI_DEV_HOST 决定热更新地址
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  // Tauri 接管了窗口控制，关闭 Vite 的清屏以免干扰日志
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      // 忽略 Rust 侧改动，避免前端热更新被触发
      ignored: ["**/src-tauri/**"],
    },
  },
});
