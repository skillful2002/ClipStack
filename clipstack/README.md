# ClipStack

跨平台（macOS / Windows）剪切板管理器。基于 **Tauri 2 + React 18 + TypeScript + Vite** 构建。常驻后台实时捕获剪贴板内容，提供历史检索、智能分类与一键复用。

## 功能特性

- **实时捕获**：事件驱动监听剪贴板（macOS 用 `NSPasteboard.changeCount` 比对，非轮询），支持纯文本、链接、代码、图片、文件。
- **历史管理**：倒序列表、按日分组；置顶（`is_pinned`）恒靠前、收藏（`is_favorite`）独立标记。
- **回收站**：删除进入 `trash`，可恢复 / 彻底清空；图片与回收站内容均可**预览**（`get_item_blob` / `get_trash_blob` 返回 PNG 字节，前端 `<img>` 渲染）。
- **检索与过滤**：全文搜索（`⌘/` / `Ctrl+/` 聚焦）、分类切换（全部/文本/链接/代码/图片/文件）、时间筛选（今天/昨天/本周/全部）。
- **一键复制**：点击 / 回车复制回系统剪贴板；文本/链接/代码写回文字，**图片**以 PNG 写回、**文件**以真实文件 URL 写回（可在访达 / 文件管理器中粘贴为文件本身）。
- **托盘 / 菜单栏**：
  - macOS 菜单栏图标下拉最近若干条历史 +「打开主界面 / 设置 / 退出」，菜单项带图标；
  - 历史条数可在设置中配置（**托盘菜单历史条数**，缺省 30）；
  - 托盘图标复用应用图标（`default_window_icon()`）。
- **全局快捷键**：`⌘⇧V` / `Ctrl Shift V` 切换主窗口可见性；关闭主窗口改为隐藏（常驻托盘不退出）。
- **设置**：
  - 外观：**浅色 / 深色 / 跟随系统**（缺省「跟随系统」，使用 Tauri 原生主题 API）；
  - 存储：历史容量上限、托盘菜单历史条数；
  - 通用：开机自启（`tauri-plugin-autostart`）；
  - **忽略应用**：可手动输入或以系统中文名从「已安装应用」选择；已添加项可即时移除（与监控过滤、持久化同步）。
- **清除全部**：主列表工具栏「清除全部」按钮，二次确认后将所有历史软删入回收站（可恢复）。
- **应用图标**：自定义 ClipStack 图标（蓝紫剪贴板 + 栈层），托盘同步使用。
- **安全与隐私**：本地数据库以 AES-256-GCM 加密（仅加密 `content_text` / `content_blob`，密钥存文件 `~/.clipstack/dbkey.dat`，权限 600）；可设主密码应用锁（macOS 支持 Touch ID / 登录密码解锁）；命中密码 / Token / 卡号的内容自动掩码预览；支持留存过期与按类型（文本 / 图片 / 文件）过滤捕获。

## 开发环境要求

| 依赖 | 说明 | 验证 |
|---|---|---|
| Node ≥ 22（nvm 管理）| 前端构建 / Vite | `node -v` |
| Rust 工具链（rustup + cargo）| 后端编译（Tauri 必需）| `cargo --version` |
| Xcode Command Line Tools（macOS）| 链接原生库 | `xcode-select -p` |

> 若未安装 Rust：`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y`
> cargo 需入 PATH（新终端默认已生效；旧终端先 `source $HOME/.cargo/env`）。

## 常用命令

```bash
npm install            # 安装前端依赖
npm run dev            # 仅前端热更新（Vite，端口 1420）
npm run typecheck      # tsc --noEmit 类型检查
npm run lint           # ESLint 检查
npm run tauri dev      # 启动完整 Tauri 应用（前端 + 原生窗口）
npm run tauri build    # 打包为安装包（mac: .dmg / .app；Win: .msi）
```

## 目录结构

```
clipstack/
├── src/                 # 前端（React + TS）
│   ├── components/      # 组件：Sidebar / HistoryList / DetailPanel / SettingsView / Trash* / HistoryItemRow
│   ├── store/           # Zustand：store/history.ts
│   ├── lib/             # tauri.ts（invoke 封装）、actions.ts（复制/置顶/收藏/删除/清除全部）、theme.ts（主题）
│   ├── styles/          # tokens.css 设计 Token、app.css 基础样式
│   ├── App.tsx          # 入口组件（三栏装配 + 实时事件订阅）
│   └── main.tsx
├── src-tauri/           # 后端（Rust）
│   ├── src/
│   │   ├── main.rs      # 二进制入口
│   │   ├── lib.rs       # run() 启动 Tauri（注册命令、插件、托盘、监控）
│   │   ├── clipboard.rs # 剪贴板监听 + 类型识别 + 忽略应用 + 系统应用枚举（mdls 中文名）
│   │   ├── db.rs        # SQLite 连接/迁移 + 命令层数据操作（含 clear_history 等）
│   │   ├── commands.rs  # Tauri commands
│   │   ├── tray.rs      # 托盘 / 菜单栏（含菜单项图标）
│   │   ├── models.rs    # 数据模型 + serde
│   │   └── error.rs
│   ├── icons/           # 应用图标（tauri icon 生成全套）+ 菜单项图标（menu-open/menu-settings）+ 生成脚本
│   ├── capabilities/    # 权限能力声明
│   ├── tauri.conf.json  # 窗口 / 打包配置
│   └── Cargo.toml
├── docs/                # 规划与步骤文档（见下）
└── README.md
```

## 主题与外观

- 主题缺省值为**跟随系统**；切换为浅色/深色时即时应用，重启保持。
- 深色模式实现：根元素 `data-theme` 驱动设计 Token（`--cs-*`）变量覆盖；跟随系统走 Tauri 原生 `getCurrentWindow().theme()` / `onThemeChanged`（非 `matchMedia`），避免不可靠。
- 已修复深色下若干控件（如忽略应用手动输入框）回退浏览器默认外观的问题。

## 忽略应用

- 监控线程按应用名过滤；忽略项可**手动输入**或**从已安装应用选择**（macOS 用 `mdls kMDItemDisplayName` 取 Finder 本地化中文名，与监控源同源，保证匹配一致）。
- 已添加项每行带移除按钮，即时生效（内存集 + 持久化同步清理）。
- 匹配大小写不敏感，存储保留系统原名。

## 开发进度

规划与里程碑见 `docs/clipstack-development-plan.md`，逐阶段步骤与验证清单见 `docs/clipstack-build-steps.md`。

- 规划阶段 **P0–P7** 脚手架、捕获引擎、持久化、主界面、托盘/快捷键、检索/分类、设置/安全、打包签名配置均已落地。
- **P8 图片预览**已实现。
- **P8 之后**还完成了一系列 bug 修复与功能增强：主题三轮修复、置顶/收藏不被覆盖、重新复制不入列、复制报错修复、托盘历史条数（含死锁修复）、忽略应用增强（显示/移除/系统选择器）、系统中文名显示与选择（含设置页卡顿修复）、设置界面布局与深色输入框、托盘菜单项图标、主题默认跟随系统、主界面「清除全部」、自定义应用图标。**详见 `docs/clipstack-build-steps.md` 的 2026-08-06 进度记录。**

> 自动化测试门禁：Rust `cargo test`（50/50）、`cargo clippy --all-targets`（0 告警）；前端 `npm run build`（tsc + vite）通过。GUI / 剪贴板 / 出包等需在桌面或 CI 手动验证。

## 文档索引

- `docs/clipstack-development-plan.md` — 产品定位、功能规划、技术架构、编码规范、数据模型、里程碑。
- `docs/clipstack-build-steps.md` — 逐阶段开发步骤、验收标准与进度记录（含 P8 之后的修复增强日志）。
- `docs/clipstack-packaging.md` — 本地构建、macOS 签名+公证、Windows .msi、CI、应用图标与托盘图标的 dev/build 差异。
