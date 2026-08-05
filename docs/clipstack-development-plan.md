# ClipStack 剪切板管理器 · 开发规划与编码规范

> 状态：规划稿（未实施）
> 范围：功能规划 + 技术架构 + 编码规范 + 数据模型 + 开发流程 + 里程碑
> 目标平台：macOS / Windows（Linux 为可选）

---

## 一、产品定位

- **是什么**：跨平台剪切板管理工具，常驻后台实时捕获剪贴板内容，提供历史检索、智能分类与一键复用。
- **核心价值**：让「复制过就找不到」成为过去；菜单栏 / 系统托盘一击即取最近内容。
- **关键约束**：常驻、低内存、事件驱动（不轮询）、本地优先、隐私可控。

---

## 二、功能模块规划

### M1 剪贴板捕获引擎（后端核心）
- 事件驱动监听：macOS 用 `NSPasteboard.changeCount` 比对，Windows 用 `AddClipboardFormatListener`，**禁止高频轮询**。
- 捕获类型：纯文本、富文本（转纯文本并保留换行）、URL、图片、文件、代码片段。
- 去重：相同 `hash` 短时间内重复不重复入库，更新 `created_at`。
- 容量限制：单条大小上限（默认图片/文件 10MB），超限记录占位并提示。
- 隐私围栏：可配置「忽略应用列表」（如密码管理器 1Password / Bitwarden），命中不捕获。

### M2 历史记录管理
- 倒序列表存储，最近在前。
- 容量上限：默认 500 条（可配置），超出自动清理最旧条目。
- 标记：置顶（`is_pinned`）、收藏（`is_favorite`）独立状态，置顶恒靠前。
- 回收站：删除进入 `trash`，可恢复 / 彻底删除，定期自动清空。

### M3 智能分类与识别
- 五大类：文本 / 链接 / 图片 / 代码 / 文件。
- 链接识别：正则匹配 `http(s)://`。
- 代码识别：启发式（含代码特征符号、行数、语言特征），可选 `tree-sitter` 做语言判定与语法高亮。
- 文件：支持多文件，记录路径 / 大小 / 类型。

### M4 检索与过滤
- 全文搜索：标题 / 内容 / 来源应用。
- 过滤维度：分类、时间（今天 / 本周 / 全部）。
- 快捷键 `⌘K` / `Ctrl K` 聚焦搜索框。

### M5 条目快捷操作
- 复制回剪贴板（默认点击 / 回车），可选「复制并粘贴」。
- 置顶、收藏、翻译（本地词典或翻译 API）、删除。
- 多格式复制：纯文本 / 富文本 / Markdown。

### M6 菜单栏（mac）/ 系统托盘（Win）
- macOS：菜单栏图标下拉 → 最近历史 3–5 条 + 打开主界面 + 设置 + 退出。
- Windows：托盘图标右键菜单，内容同上；弹层**向上弹出**。
- 点击历史项即复制并关闭弹层。

### M7 全局快捷键
- 唤起主界面：`⌘⇧V` / `Ctrl Shift V`（可自定义）。
- 唤起快捷粘贴面板（可选，P2 后）。

### M8 设置
- 常规：开机自启、界面语言、主题（浅 / 深 / 跟随系统）。
- 捕获：忽略应用、单条大小上限、富文本处理策略。
- 存储：历史上限、自动清理周期、是否加密。
- 快捷键：自定义绑定。
- 关于 / 导出备份。

### M9 数据持久化
- SQLite 存储：`history` / `trash` / `settings` / `ignored_apps`。
- 可选加密：敏感字段或整库 `sqlcipher`。
- 导出 / 备份为 JSON。

### M10 安全与隐私
- 默认沙箱：前端仅能通过 Tauri command 访问后端，不直接碰系统 API。
- 忽略列表防捕获密码；本地优先，无云端默认。

### M11 未来扩展（不在首版）
- 多设备云同步、OCR 图片转文字、模板 / 片段库、插件市场。

---

## 三、技术架构

| 层 | 选型 | 说明 |
|---|---|---|
| 框架 | **Tauri 2.x** | 轻量跨平台，不打包浏览器内核 |
| 前端 | **React 18 + TypeScript + Vite** | 生态成熟，1:1 还原设计稿 |
| 状态 | **Zustand** | 单一来源，轻量 |
| 样式 | CSS 变量绑定设计 Token（可叠加 Tailwind） | 与设计系统一致 |
| 后端 | **Rust (edition 2021)** | 捕获 / 托盘 / 快捷键 / DB |
| 剪贴板 | `arboard` | 跨平台读写 |
| 快捷键 | `tauri-plugin-global-shortcut` | 全局唤起 |
| 托盘 | `tauri-plugin-tray-icon` | 菜单栏 / 托盘 |
| 对话框 | `tauri-plugin-dialog` | 导出备份 |
| 数据库 | `rusqlite` / `sqlx` + `serde` | SQLite 持久化 |
| 测试 | `cargo test` / `Vitest` | 单测关键逻辑 |
| 构建 | `tauri build` + GitHub Actions | 双平台签名打包 |

> 前端用 React 而非 Svelte 的考量：团队资料多、招人易、`@tauri-apps/api` 示例全。若团队已熟练 Svelte，可换 SvelteKit，规范相应调整。

---

## 四、项目结构

```
clipstack/
├─ src/                      # 前端 (React + TS)
│  ├─ components/            # 可复用组件（与 Ardot 组件一一对应）
│  ├─ pages/                 # 主界面 / 设置页
│  ├─ store/                 # zustand 按领域拆分
│  ├─ api/                   # Tauri invoke 封装（前后端契约）
│  ├─ styles/                # design tokens (CSS 变量)
│  └─ main.tsx
├─ src-tauri/                # Rust 后端
│  ├─ src/
│  │  ├─ main.rs             # 入口、注册插件
│  │  ├─ clipboard.rs        # 剪贴板监听 + 类型识别
│  │  ├─ db.rs               # SQLite 连接与迁移
│  │  ├─ commands.rs         # Tauri commands
│  │  ├─ tray.rs             # 托盘 / 菜单栏
│  │  ├─ shortcut.rs         # 全局快捷键
│  │  ├─ models.rs           # 数据模型 + serde
│  │  └─ error.rs            # AppError + thiserror
│  ├─ icons/                 # 应用图标 / 托盘图标
│  ├─ Cargo.toml
│  ├─ tauri.conf.json
│  ├─ capabilities/          # 权限声明（安全）
│  └─ build.rs
├─ package.json
├─ vite.config.ts
├─ tsconfig.json
├─ .editorconfig
├─ .gitignore
└─ README.md
```

---

## 五、编码规范

### 5.1 通用
- 前端 TypeScript **strict 模式**；后端 Rust **edition 2021**。
- 行宽：Rust 100，TS 100。
- 缩进：TS 2 空格；Rust 用 `rustfmt` 默认（4 空格）。
- 注释讲 **why**，不讲 what；禁止裸 `TODO`/`FIXME`，必须带 issue 引用。

### 5.2 Rust 规范
- 格式化 `rustfmt`（默认），`clippy -D warnings` 视为错误。
- 命名：`snake_case` 变量/函数，`CamelCase` 类型 / trait，`SCREAMING_SNAKE` 常量。
- 错误：自定义 `AppError`（`thiserror`）；command 返回 `Result<T, String>`。
- 模块单职责：`clipboard` / `db` / `commands` 各司其职；单文件 > 300 行考虑拆分。
- 异步：仅在 IO 阻塞处用 `async`（如批量写入）。

### 5.3 前端规范（React / TS）
- 格式化 `Prettier`；Lint `ESLint` + `@typescript-eslint` strict。
- 命名：组件 `PascalCase`，hook `useXxx`，常量 `UPPER_SNAKE`，变量 `camelCase`。
- 组件：函数式 + hooks，**禁止 class 组件**。
- 状态：Zustand 单一来源，不滥用 Context。
- 样式：绑定 design token 的 CSS 变量，**禁止硬编码颜色**（一次性特例除外）。
- 类型：**禁止 `any`**；`api/` 层类型与 Rust 模型字段对齐。
- Tauri 事件：在 `useEffect` 内 `subscribe`，卸载时务必 `unlisten`。

### 5.4 前后端通信契约
- command 命名：`动词_名词`，如 `get_history` / `add_item` / `delete_item`。
- 入参/出参：`serde` 结构体；后端 `snake_case`，前端用 `#[serde(rename_all="camelCase")]` 转换。
- 事件名：`kebab-case`，如 `clipboard-changed` / `item-pinned`。
- 统一返回：`{ ok: bool, data?: T, error?: string }`。

### 5.5 目录与文件
- 一个组件一个文件；store 按领域（history / settings / ui）拆分。
- 资源（图标 / 模板）放 `src-tauri/icons` 与 `public`。
- 删除 / 重命名遵守「先确认依赖」原则，避免悬空引用。

### 5.6 Git 工作流
- 分支：`main`（保护）/ `develop` / `feature/*` / `fix/*`。
- 提交遵循 **Conventional Commits**：
  - `feat(clipboard): 事件驱动监听`
  - `fix(db): 修正去重哈希逻辑`
  - `chore(deps): 升级 tauri 至 2.x`
- PR：至少 1 人 review；CI 全绿方可合入。
- 版本：**SemVer**（主.次.修订）。

### 5.7 测试与质量门
- Rust：`cargo test` 单测 + `clippy -D warnings`。
- 前端：`Vitest` 单测分类识别 / 去重等关键逻辑；`tsc --noEmit` 0 error。
- CI 三道门：lint → test → build，全部通过才出包。

### 5.8 安全基线
- 前端**不直连**系统 API，必须经 command。
- `tauri.conf.json` / `capabilities/` 显式声明允许的能力，最小权限。
- 读取剪贴板受忽略列表约束。
- 敏感字段加密，密钥存系统钥匙串（mac Keychain / Win Credential Manager）。

---

## 六、数据模型（SQLite）

```sql
-- 历史记录
history(
  id            INTEGER PRIMARY KEY,
  content_type  TEXT,            -- text | link | image | code | file
  content_text  TEXT,            -- 文本/链接/代码/文件路径
  content_blob  BLOB,            -- 图片/文件二进制（按需）
  source_app    TEXT,            -- 来源应用
  size_bytes    INTEGER,
  hash          TEXT,            -- 去重用
  is_pinned     INTEGER DEFAULT 0,
  is_favorite   INTEGER DEFAULT 0,
  created_at    INTEGER          -- 毫秒时间戳
);

-- 回收站（字段同 history + deleted_at）
trash( ... , deleted_at INTEGER );

-- 设置
settings( key TEXT PRIMARY KEY, value TEXT );

-- 忽略应用
ignored_apps( bundle_id TEXT, exec_name TEXT );
```

索引：`created_at`、`content_type`、`hash`（去重）。

---

## 七、开发里程碑

| 阶段 | 内容 | 产出 |
|---|---|---|
| P0 | 捕获引擎原型（命令行验证监听/去重） | Rust 模块跑通 |
| P1 | 主界面落地（三栏，对齐设计稿） | 可交互主窗口 |
| P2 | 托盘 / 菜单栏 + 全局快捷键 | 一击取历史 |
| P3 | 检索 / 分类 / 置顶 / 收藏 | 完整管理能力 |
| P4 | 设置 + 持久化 + 安全 | 可日常使用 |
| P5 | 双平台打包签名 + 内测 | 可分发安装包 |

---

## 八、风险与注意

- **耗电**：剪贴板监听必须事件驱动，绝对避免轮询。
- **大内容**：图片 / 富文本懒加载缩略图，超限占位。
- **平台差异**：托盘弹向（Win 向上 / mac 向下）、菜单栏位置、快捷键冲突。
- **多实例**：加单例锁，避免重复启动导致双监听。
- **权限**：macOS 可能需辅助功能 / 屏幕录制授权，安装引导要讲清。
