# ClipStack

跨平台（macOS / Windows）剪切板管理器。基于 **Tauri 2 + React 18 + TypeScript + Vite** 构建。

## 开发环境要求

| 依赖 | 说明 | 验证 |
|---|---|---|
| Node ≥ 22（nvm 管理）| 前端构建 / Vite | `node -v` |
| Rust 工具链（rustup + cargo）| 后端编译（Tauri 必需）| `cargo --version` |
| Xcode Command Line Tools（macOS）| 链接原生库 | `xcode-select -p` |

> 若未安装 Rust：`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y`

## 常用命令

```bash
npm install            # 安装前端依赖
npm run dev            # 仅前端热更新（Vite，端口 1420）
npm run typecheck      # tsc --noEmit 类型检查
npm run lint           # ESLint 检查
npm run tauri dev      # 启动完整 Tauri 应用（前端 + 原生窗口）
npm run tauri build    # 打包为安装包（mac: .dmg / .app）
```

## 目录结构

```
clipstack/
├── src/                 # 前端（React + TS）
│   ├── styles/          # tokens.css 设计 Token、global.css 基础样式
│   ├── App.tsx          # 入口组件
│   └── main.tsx
├── src-tauri/           # 后端（Rust）
│   ├── src/
│   │   ├── main.rs      # 二进制入口
│   │   └── lib.rs       # run() 启动 Tauri
│   ├── capabilities/    # 权限能力声明
│   ├── tauri.conf.json  # 窗口 / 打包配置
│   └── Cargo.toml
└── docs/                # 规划与步骤文档
```

## 开发阶段

按 `docs/clipstack-build-steps.md` 逐阶段推进，每阶段验证通过再进入下一阶段。
当前进度：**P0 脚手架**。
