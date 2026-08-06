# ClipStack 开发步骤与验证清单

> 状态：**开发中（P0–P7 代码与配置已全部落地；P8 图片预览已实现并通过自动构建验证；托盘/快捷键/主题/出包等需在用户桌面或 CI 手动验证）**
> 用途：把 `clipstack-development-plan.md` 拆成可逐步执行、每步带验收标准的开发流程
> 原则：**每阶段验证通过，才进入下一阶段**
> 最后确认：基于「Node 22（nvm 管理）已就绪」这一前提进入开发；Rust 工具链为 Tauri 后端必需，开发机需另行安装。

---

## 〇、前置环境（开发机）

| 依赖 | 用途 | 验证 | 当前状态 |
|---|---|---|---|
| Rust 工具链（rustup + cargo）| 编译后端 | `cargo --version` 有输出 | ✅ 已装（rustc 1.97.1）；**cargo 需入 PATH**：`~/.zshenv` 已写 `source $HOME/.cargo/env`，新终端自动生效，旧终端需手动 `source $HOME/.cargo/env`（否则 `tauri dev` 报 cargo not found）|
| Node 22 + npm | 前端构建 | `node -v` ≥ 22 | ✅ v22.22.2（nvm 管理）|
| Tauri CLI | 脚手架 / 构建 | `npm run tauri --version` 有输出 | ✅ 经 npm `@tauri-apps/cli` v2 提供，`npm run tauri` 即调用，无需 `cargo install tauri-cli` |
| macOS：Xcode Command Line Tools | 链接原生库 | `xcode-select -p` 有路径 | ✅ 已就绪 |
| Windows（CI 侧）：WebView2 + VS Build Tools | 出 Win 包 | 仅在打包机需要 | — 仅 CI |

---

## 进度记录

- **2026-08-05 · P0 脚手架**
  - ✅ 环境基线确认：Node 22.22.2（nvm）就绪；Rust 工具链已安装（rustc 1.97.1）；Xcode CLT 就绪。
  - ✅ 工程落地 `clipstack/`：前端 React18+TS+Vite、后端 Tauri 2（Cargo.toml / tauri.conf.json / lib.rs / capabilities）。
  - ✅ 设计 Token 写入 `src/styles/tokens.css`（与 Ardot 设计稿一致）。
  - ✅ 应用图标集生成（mac `.icns` / Win `.ico` / 多尺寸 PNG）。
  - ✅ 前端静态检查：`tsc --noEmit` 0 error、`npm run lint` 0 告警。
  - ✅ 后端 `cargo check` 通过（首次编译 Tauri 依赖耗时约 19 分钟，0 error）。
  - ✅ 后端 `cargo clippy --all-targets -- -D warnings` 零告警（质量门禁通过）。
  - ⚠️ **GUI 验证需在用户桌面执行** `npm run tauri dev`：无 GUI 的 CI/沙箱无法拉起原生窗口，故「窗口可启动」一项留待你本地确认。

- **2026-08-05 · P1 剪贴板捕获引擎（实现完成，待本地手动验证）**
  - ✅ 新增 `src-tauri/src/clipboard.rs`：monitor 线程（mac 用 `NSPasteboard.changeCount` 300ms 比对，非 mac 占位）、`read_clipboard`（arboard 读 text/image/file）、`classify`（text/link/code/image/file）、`content_hash` 去重、来源应用读取（macOS `NSWorkspace.frontmostApplication`）+ 忽略列表过滤、广播 `clipboard-changed` 事件、`add_ignored_app` 命令。
  - ✅ `Cargo.toml` 加 `arboard = "3"`；macOS target 加 `objc2`/`objc2-app-kit`(NSPasteboard/NSWorkspace/NSRunningApplication)/`objc2-foundation`(NSString)。
  - ✅ `lib.rs`：`mod clipboard`、托管 `MonitorState`、注册 `add_ignored_app`、setup 启动 `start_monitor`。
  - ✅ 构建验证：`cargo check` 0 error；`cargo clippy --all-targets -- -D warnings` 0 告警；`cargo test` 9 项单测全过（classify/hash/truncate/is_link）。
  - ⏳ **本地手动验证待你执行**（步骤见 P1 小节「本地手动验证步骤」）：复制 文本/链接/图片/文件 看日志、去重、忽略列表。沙箱无 GUI/剪贴板，无法代跑。
  - ⚠️ macOS 无系统级剪贴板变更事件，采用 300ms changeCount 比对（业界通行，非忙等）；Windows 真事件驱动 `AddClipboardFormatListener` 留待后续。

- **2026-08-05 · P2 持久化与命令层（实现完成，待本地手动验证）**
  - 新增 `src-tauri/src/models.rs`：ContentType（snake→camel 序列化）、HistoryItem、NewItem、Setting。
  - 新增 `src-tauri/src/db.rs`：AppDb（Mutex<Connection>）+ DbState（Arc）；`open(app)` 解析 `app_data_dir` 建 `clipstack.db`；`migrate` 建 history/trash/settings/ignored_apps 四表 + 索引；`insert_or_bump`（hash 去重/置顶）、`get_history`（默认 500、置顶优先）、`delete_item`（移入 trash）、`toggle_pin/favorite`、`update_setting/get_settings`、`insert/get_ignored_apps`、`enforce_capacity`（5000 上限）。含 6 项单测。
  - 新增 `src-tauri/src/commands.rs`：add_item/get_history/delete_item/toggle_pin/toggle_favorite/update_setting/get_settings/add_ignored_app/get_ignored_apps。
  - 改造 `clipboard.rs`：capture 落库（insert_or_bump）、emit HistoryItem；ContentType 迁移到 models；materialize 拆分 preview/content_text/blob；新增 `ignore_app`。
  - 改造 `lib.rs`：manage DbState、启动从 DB 加载忽略列表、启动 monitor 时传 db、注册 9 个命令。
  - 依赖：Cargo.toml 加 `rusqlite = { version = "0.32", features = ["bundled"] }`。
  - ⚠️ 偏差说明：`ignored_apps` 表用 `name`（小写 app 名）而非规划里的 bundle_id/exec_name——因为 macOS 检测只拿到应用名；忽略列表按名称过滤。
  - ✅ 构建验证：`cargo check` 0 error；`cargo clippy --all-targets -- -D warnings` 0 告警；`cargo test` 15 项全过（含 P1 9 项 + P2 6 项）。
  - ⏳ **本地手动验证待你执行**（步骤见 P2 小节「本地手动验证步骤」）：复制内容看落库、查 DB、命令读写、忽略列表持久化。沙箱无 GUI/剪贴板，无法代跑。

- **2026-08-05 · P3 主界面落地（React 三栏，实现完成，待本地手动验证）**
  - 新增 `src/types.ts`（HistoryItem/ContentType/Setting，camelCase 字段）、`src/lib/tauri.ts`（`invoke` 封装 + `listen clipboard-changed`）。
  - 新增 `src/store/history.ts`（Zustand 单 store：items/selectedId/category/timeFilter/search/view/toast + filterItems + 置顶优先排序）、`src/lib/actions.ts`（复制/置顶/收藏/删除 hook）。
  - 新增组件：`Sidebar`（搜索 + 分类导航 + 回收站/设置入口）、`HistoryList`（时间筛选标签 + 按日分组 + 条目行 + 悬停操作）、`DetailPanel`（类型标签/预览/元数据/操作区）、`SettingsView`（忽略应用管理）、`TrashPlaceholder`。
  - `App.tsx`：装配三栏；`useEffect` 启动 `load()` + 订阅实时事件（卸载 unlisten）；toast 自动消失。
  - 后端：新增 `copy_item` 命令（arboard `set_text`，文本/链接/代码写回系统剪贴板；图片/文件 P3 暂不支持），注册进 invoke_handler。
  - ✅ 构建验证：前端 `npm run build`（tsc + vite build）成功，60 模块打包；后端 `cargo clippy --all-targets -- -D warnings` 0 告警；`cargo test` 15/15 通过（P1/P2 未受影响）。
  - ⏳ **本地手动验证待你执行**（步骤见 P3 小节「验证标准」）：实时追加、复制写回、置顶/收藏持久、删除、忽略应用。沙箱无 GUI/剪贴板，无法代跑。

- **2026-08-05 · P4 托盘 / 菜单栏 + 全局快捷键（实现完成，待本地手动验证）**
  - 新增 `src-tauri/src/tray.rs`：托盘图标（复用窗口图标）+ 菜单（最近 5 条历史 + 打开主界面 + 设置 + 退出）；`build_menu`/`refresh_menu`；点击历史项 → `set_clipboard_text` 写回并广播 `tray-copied`；点击打开/设置 → 唤起主窗口并广播 `show-view`。每次 `clipboard-changed` 重建菜单保持常新。
  - 后端 `lib.rs`：注册 `tauri-plugin-global-shortcut`；`CmdOrCtrl+Shift+V` 切换主窗口可见性（`ShortcutState::Pressed`）；主窗口 `CloseRequested` 改为 `hide()`（常驻托盘不退出）；启动托管 DbState 后构建托盘并订阅刷新。
  - 后端 `clipboard.rs` 新增 `set_clipboard_text`（arboard `set_text`）；`commands.rs::copy_item` 复用之（去掉直接 Clipboard 依赖）。
  - 前端：`lib/tauri.ts` 新增 `onShowView`/`onTrayCopied`；`App.tsx` 订阅并处理视图切换与复制回执；`Sidebar` 搜索框加 `id` 供 ⌘K 聚焦。
  - 依赖：`Cargo.toml` 加 `tray-icon` feature 与 `tauri-plugin-global-shortcut = "2"`。
  - ✅ 构建验证：后端 `cargo clippy --all-targets -- -D warnings` 0 告警；`cargo test` 15/15 通过（P1/P2 未受影响）；前端 `npm run build` 成功。
  - ⏳ **本地手动验证待你执行**（见 P4 小节「验证标准」）：点击托盘菜单复制、全局快捷键切换窗口、关闭按钮隐藏而非退出。沙箱无 GUI，无法代跑。

- **2026-08-05 · P5 检索 / 分类 / 置顶 / 收藏（实现完成）**
  - 搜索 / 分类（全部/文本/链接/代码/图片/文件）/ 时间筛选（今天/昨天/本周/全部）/ 置顶 / 收藏 已于 P3 落在前端（Sidebar + store.filterItems + 置顶优先排序）。
  - 新增 ⌘K / Ctrl+K 聚焦搜索框（App.tsx keydown 监听 + Sidebar 搜索框 `id`），P4 一并提交。
  - 翻译操作为可选占位，未实现（非核心能力）。
  - ✅ 构建验证：前端 `npm run build` 成功；后端单测未受影响。

- **2026-08-05 · P6 设置 + 安全（实现完成，待本地手动验证）**
  - `SettingsView` 增强：外观（浅色/深色/跟随系统，写入 `theme` 设置并即时应用）、存储（历史上限，写入 `max_history`，store 启动时读取作为拉取条数）、开机自启开关（tauri-plugin-autostart）、忽略应用管理（沿用 P3）。
  - 新增 `src/lib/theme.ts`：`applyTheme`/`resolveTheme`，根元素 `data-theme` 驱动；`[data-theme="dark"]` 覆盖设计 Token 变量（app.css）。
  - App 启动读取 `theme` 设置并应用；store 启动读取 `max_history`。
  - 后端：`tauri-plugin-autostart = "2"`，`lib.rs` 注册（MacosLauncher::LaunchAgent）；`capabilities/default.json` 加 `autostart:default`（已核对插件 permissions/default.toml 含 allow-enable/disable/is-enabled）。
  - ✅ 构建验证：后端 `cargo clippy --all-targets -- -D warnings` 0 告警；前端 `npm run build` 成功；`cargo test` 15/15 通过；autostart 权限标识符已核对存在。
  - ⏳ **本地手动验证待你执行**：切换主题即时变暗、重启保持；修改历史上限重启生效；开机自启开关开启后注销重登自动运行；移除忽略项仍待后续。

- **2026-08-05/06 · P7 打包签名与分发（配置完成，待本地/CI 出包与签名）**
  - `tauri.conf.json`：`bundle.category=Utility`、`shortDescription`/`longDescription`、macOS `minimumSystemVersion=10.15`、Windows `webviewInstallMode=downloadBootstrapper`。**已移除全局 `bundle.targets`**（原 `["dmg","app","msi"]` 在 macOS CI runner 上会因 msi 不支持而报错）——改为依赖 Tauri 各平台默认目标（macOS → app+dmg、Windows → msi+nsis），CI 上传路径不变。
  - 新增 `.github/workflows/release.yml`：双 job（build-macos 签名+公证 / build-windows msi），由 `v*` 标签或 `workflow_dispatch` 触发；读取 `APPLE_SIGN_IDENTITY/APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID` 四个 Secrets，缺失则仅 ad-hoc 签名（本地测试）；Rust 缓存 + 构建产物 artifact 上传。
  - 新增 `docs/clipstack-packaging.md`：本地构建、macOS 签名+公证、Windows msi、CI、体积基线、约束说明。
  - ✅ 构建验证：`cargo build --release`（后台执行）经 `build.rs` 解析 `tauri.conf.json` 通过、release 编译 0 error（验证配置 schema 合法 + release profile 编译可过）；前端 `npm run build` 此前已绿；`cargo clippy --all-targets -- -D warnings` 0 告警、`cargo test` 15/15（P7 无 Rust 代码改动，沿用 P6 成果）。
  - ⏳ **出包/签名验证待用户在桌面或 CI 执行**（见 P7 小节「验证标准」与 `docs/clipstack-packaging.md`）：`npm run tauri build` 出 dmg/msi、macOS Gatekeeper、Windows WebView2 引导、体积/内存基线。沙箱无签名密钥且无法拉起原生出包，无法代跑。

- **2026-08-05 · P8 图片预览（已实现，待本地手动验证）**
  - 问题：详情面板选中图片时只有「图片预览（P3 暂不支持）」占位符；arboard 捕获到的是**原始 RGBA 像素**而非图片文件，且 `get_history`/`get_item` 从未 SELECT `content_blob`，图片字节从不回前端。
  - 修复链路：
    - `clipboard.rs`：新增 `encode_rgba_to_png`，在落库时把 RGBA 编码为 PNG 再存入 `content_blob`（自描述、前端可直接 `<img>`）；`materialize` 图片分支改为存 PNG、size_bytes 记 PNG 长度。
    - `commands.rs`：新增 `get_item_blob(db, id)` 读取 `content_blob` 返回 `Vec<u8>`（Tauri 自动以二进制 ArrayBuffer 回传）；`lib.rs` 注册。
    - 前端 `tauri.ts`：`getItemBlob(id)` 封装，返回 `Uint8Array<ArrayBuffer>`。
    - 前端 `DetailPanel.tsx`：选中图片时按 id 拉取 PNG，生成 `URL.createObjectURL` 的 `<img>` 预览；含加载中 / 失败 / `onError` 占位与对象 URL 回收（切换/卸载时 `revokeObjectURL`，避免泄漏）。
    - `app.css`：新增 `.preview-image-wrap` / `.preview-image`，沿用 `--cs-*` 设计 Token。
    - `Cargo.toml`：`png = "0.17"`（纯 Rust、无系统依赖）。
  - ✅ 构建验证：`cargo clippy --all-targets -- -D warnings` 0 告警；`cargo test` 16/16（新增 `png_roundtrip_preserves_pixels` 验证编码/解码像素一致）；前端 `npm run build` 成功。
  - ⏳ **本地手动验证**：复制一张图片 → 选中该条目 → 详情面板显示图片预览；切换条目预览随之更新、旧 URL 被回收。沙箱无 GUI 无法代跑。
  - ⚠️ 兼容性：本次之前已落库的开发期图片为原始 RGBA（非 PNG），预览会走失败占位；重新复制一张图片即可（新数据均为 PNG）。

- **2026-08-06 · P8 之后的修复与增强（全部本地提交，门禁全绿）**
  - 通用门禁：Rust `cargo clippy --all-targets` 0 告警；`cargo test` 由 16 项增至 **24 项**（新增 clear_history、delete_ignored_app、ignored_apps 大小写匹配等单测）；前端 `npm run build` 通过。GUI / 剪贴板 / 出包仍需桌面或 CI 手动验证。
  - **主题三轮修复**（`63fb14a`）：① 浅/深色手动切换失效 → 提升深色 CSS 选择器特异性（`[data-theme="dark"]` 覆盖 `tokens.css` 的 `:root` 浅色块）；② 跟随系统颜色不对 → 改用 Tauri 原生 `getCurrentWindow().theme()` / `onThemeChanged`（放弃不可靠的 `matchMedia`）；③ 跟随系统恒浅色 → 移除 `tauri.conf.json` 强制 `"theme":"Light"`；`capabilities/default.json` 增 `core:window:allow-theme`。
  - **回收站图片预览**（`11f2a0e`）：新增 `get_trash_blob`（读 trash 表 `content_blob`）+ `TrashDetail` 复用预览逻辑。
  - **置顶/收藏被覆盖**（`01f0f9e`）：`capture()` 原本硬编码 `is_pinned:false` 覆盖用户置顶，改为落库后回填真实 pin/fav 状态。
  - **重新复制不入列**（`836cfac`）：选中条目复制写回剪贴板会再次触发捕获；新增 `note_self_copy` 占位去重（写 `recent` 队列），不重复入列、不改写原复制时间。
  - **复制报错**（`9a59b00`）：Tauri v2 将 snake_case 参数转为 camelCase，前端错传 `content_type`，改 `contentType` 修复 `missing required key contentType`。
  - **托盘历史条数 + 死锁**（`a6060e1`）：设置新增「托盘菜单历史条数」（缺省 30，`DEFAULT_TRAY_HISTORY`），托盘菜单按配置条数展示；保存时 `update_setting` 持 db 锁同步 emit 触发监听器再 lock 导致死锁 → 改为释放锁（独立作用域 drop conn）后再 `app.emit`。
  - **忽略应用增强**（`c8ca0ea`）：已添加项每行显示 × 即时移除（`remove_ignored_app` + `db::delete_ignored_app`）；新增「从已安装应用选择」下拉（`list_installed_apps`，macOS 扫描 /Applications、~/Applications、/System/Applications/Utilities 的 .app）。
  - **中文名显示与选择 + 卡顿修复**（`303cf52`，含两处耦合）：① 用 `mdls kMDItemDisplayName` 取 Finder 本地化中文名（与监控 `localizedName()` 同源），并去掉 mdls 返回的 `.app` 后缀保证匹配一致；存储/匹配大小写不敏感且保留原名；② 修复设置页卡顿：收窄扫描范围、递归深度限 1 层、会话级 `OnceLock` 缓存、启动后台线程预热缓存。
  - **设置界面布局**（`360b23c`）：分组间加 `margin-bottom` 间隔；移除 `.settings-card` 的 `max-width:560px` 使填满窗口宽度，消除右侧空白。
  - **深色输入框**（`63fbc94`）：`.settings-add input` 补 `background/color/caret-color`，深色模式不再回退浏览器浅色默认。
  - **托盘菜单项图标**（`c274e4d`）：用 `IconMenuItemBuilder` 给「打开主界面」（窗口 glyph）/「设置」（齿轮 glyph）加图标；新增 `menu-open.png`/`menu-settings.png`（@2x 中性灰，PIL 生成脚本 `gen_menu_icons.py`），`include_bytes` 内嵌；`tauri` 依赖加 `image-png` feature。
  - **主题缺省值**（`6e58ad5`）：缺省改为「跟随系统」（theme.ts `currentTheme`、`App.tsx` 无已保存设置时退化为 `applyTheme("system")`、SettingsView 分段控件默认 `system`）；已保存的 light/dark 仍被尊重。
  - **主界面「清除全部」**（`bb73022`）：工具栏按钮（空列表禁用）+ 自定义确认弹窗；确认后 `db::clear_history`（事务：history 全量入 trash 后清空，trash id 自增避免冲突）软删入回收站，主列表与回收站同步刷新。
  - **应用图标**（`b408c97`）：新增 `gen_app_icon.py` 生成蓝紫剪贴板源图标，`tauri icon` 生成全套平台图标（mac `icon.icns` / Win `icon.ico` / 通用 `icon.png` + 32/64/128/iOS/Android/StoreLogo 系列）替换默认 Tauri 图标；托盘经 `default_window_icon()` 自动复用。
  - ⚠️ **图标排障要点**：图标由 `tauri-build` 编译期嵌入二进制，仅 `tauri.conf.json` 变动才重嵌；改 PNG 后需 `touch tauri.conf.json` 重启 dev 才生效。**dev 模式 Dock 不显示自定义应用图标**（不打包 .app），`tauri build` 后才出现。详见 `docs/clipstack-packaging.md`.

## 通用验证手段（每阶段都跑）

- **Rust**：`cargo test`（单测）、`cargo clippy -D warnings`（零告警）
- **前端**：`npm run lint`、`npx tsc --noEmit`（0 error）、`npm run test`（Vitest）
- **手动验证**：按各阶段「手动清单」逐项操作
- **视觉验证**：对照 Ardot 设计稿截图，确认布局 / 配色 / 间距一致
- **门禁**：以上任一项失败 → 本阶段不视为通过

---

## 阶段 P0 · 脚手架与环境打通

**目标**：能起一个最小 Tauri 窗口。

步骤：
1. 用 `npm create tauri-app`（React + TS + Vite 模板）初始化 `clipstack/` 工程。
2. 落地 `docs/clipstack-development-plan.md` 中的目录结构（src / src-tauri 各模块空文件占位）。
3. 配置 `tauri.conf.json`（窗口尺寸 1240×800、标题 ClipStack）、`capabilities/` 最小权限。
4. 接入设计 Token：把 Ardot 的 CSS 变量写入 `src/styles/tokens.css`。

验证标准：
- [ ] `cargo tauri dev` 能启动一个空白主窗口，无报错。**（需本地桌面验证）**
- [x] `tsc --noEmit` / `npm run lint` 全绿（已验证：0 error / 0 告警）。
- [x] `cargo check` 通过（0 error，首次编译已缓存依赖）。
- [x] 设计 Token 变量已就位，可被组件引用（`src/styles/tokens.css`）。
- [x] `cargo clippy --all-targets -- -D warnings` 零告警（质量门禁通过）。

---

## 阶段 P1 · 剪贴板捕获引擎（Rust）

**目标**：后台事件驱动捕获剪贴板，识别类型并去重。

步骤：
1. `clipboard.rs`：mac 用 `NSPasteboard.changeCount` 比对；Win 用 `AddClipboardFormatListener`（先用 cfg 条件编译占位 Win 分支）。
2. 类型识别：text / link / image / code / file。
3. 计算 `hash` 用于去重；同 hash 短时间不重复入库。
4. 读取来源应用名；应用「忽略列表」拦截。
5. 捕获到内容 → 通过 Tauri event `clipboard-changed` 广播。

验证标准：
- [x] **单测**：构造不同内容，类型识别正确；同内容两次捕获返回同一 hash。（`cargo test` 已覆盖 classify / hash / truncate / is_link）
- [ ] **手动**：分别复制 文本 / 链接 / 图片 / 文件 各一次，后端日志打印「类型 + hash + 来源」。
- [ ] **手动**：连续复制相同内容，只捕获一次（去重生效）。
- [ ] **手动**：把某应用加入忽略列表后复制，不被捕获。
- [x] 确认是变更检测驱动（macOS 用 `NSPasteboard.changeCount` 300ms 比对；非忙等）。Windows 真事件驱动 `AddClipboardFormatListener` 留待后续。

### P1 本地手动验证步骤（在桌面执行 `npm run tauri dev`）
1. **看日志**：终端出现 `[clipstack] captured <类型> hash=… source=<应用> size=…` 即捕获成功。
2. **分类**：分别复制 一段纯文本、一个 https 链接、一张图片、一个文件，确认类型正确（Text/Link/Image/File；多行代码会被识别为 Code）。
3. **去重**：连续复制同一段文本两次，应只有一次 `captured`，第二次为 `dedup skip …`。
4. **忽略列表**：在 WebView DevTools 控制台（应用内右键 → 检查 / 或 `npm run tauri dev` 附带）执行：
   ```js
   const { invoke } = await import('@tauri-apps/api/core');
   await invoke('add_ignored_app', { name: 'Safari' });   // 用你实际复制时所在的 App 名
   ```
   随后在该 App 内复制，应出现 `ignored by app filter: …` 而不再 `captured`。
5. **事件广播**：DevTools 控制台执行 `const { listen } = await import('@tauri-apps/api/event'); await listen('clipboard-changed', e => console.log('EVENT', e.payload));` 复制内容时应收到事件负载（完整 UI 消费在 P3）。

---

## 阶段 P2 · 持久化与命令层

**目标**：捕获内容落库，前端可经 command 读写。

步骤：
1. `db.rs`：建 `history` / `trash` / `settings` / `ignored_apps` 四表 + 索引（按规划模型）。
2. `models.rs`：`serde` 结构体，字段 `snake_case`，前端用 `rename_all="camelCase"`。
3. `commands.rs`：`add_item` / `get_history` / `delete_item` / `toggle_pin` / `toggle_favorite` / `update_settings` 等。
4. 容量上限：超出自动清理最旧条目；删除进 `trash`。

验证标准：
- [x] **单测**：`insert_or_bump` 后 `get_history` 按时间倒序返回；`delete_item` 移到 `trash`；同 hash 去重复用行并刷新 created_at；容量上限生效；toggle_pin、忽略列表持久化（`cargo test` 已覆盖）。
- [ ] **手动**：复制若干内容后，DB 文件存在且 `history` 表记录完整（类型/来源/大小/时间正确）。
- [x] `get_history` 默认 500 条上限、置顶项排序靠前（`pin_first=true` ORDER BY is_pinned DESC, created_at DESC）。

### P2 本地手动验证步骤（在桌面执行 `npm run tauri dev`）
1. **落库验证**：复制几段不同内容（文本 / 链接 / 图片 / 文件），终端出现 `[clipstack] captured ...` 即已落库。
2. **查库**：打开 `~/Library/Application Support/<bundle_id>/clipstack.db`（用 `sqlite3` 或 DB 工具），`SELECT content_type, source_app, size_bytes, created_at FROM history;` 确认记录完整、时间倒序。
3. **命令读写**（DevTools 控制台）：
   ```js
   const { invoke } = await import('@tauri-apps/api/core');
   // 拉历史（应含刚才复制的内容，最新在前）
   await invoke('get_history', { limit: 50 });
   // 置顶第一条
   const h = await invoke('get_history', { limit: 1 });
   await invoke('toggle_pin', { id: h[0].id });
   // 删除第一条（进 trash）
   await invoke('delete_item', { id: h[0].id });
   // 设置 + 读取
   await invoke('update_setting', { key: 'max_history', value: '2000' });
   await invoke('get_settings');
   ```
   再次查库确认：置顶项 `is_pinned=1`；被删项已不在 `history` 而在 `trash`。
4. **忽略列表持久化**：`await invoke('add_ignored_app', { name: 'Safari' })`，重启应用后该忽略仍生效（已从 DB 载入）。

---

## 阶段 P3 · 主界面落地（React，对齐设计稿）

**目标**：三栏界面可交互，数据从后端实时来。

步骤：
1. `src/types.ts` + `src/lib/tauri.ts`：TypeScript 类型（camelCase，与后端一致）、`invoke` 封装（get_history/delete_item/toggle_pin/toggle_favorite/copy_item/settings/ignored_apps + `listen clipboard-changed`），禁止 `any`。
2. `src/store/history.ts`（Zustand 单 store）：items/selectedId/category/timeFilter/search/view/toast；load/prepend/remove/applyToggle/filterItems；置顶优先、时间倒序排序。
3. `src/lib/actions.ts`：复制 / 置顶 / 收藏 / 删除 操作 hook（列表行与详情面板共用）。
4. 组件：`Sidebar`（搜索 + 分类导航 + 回收站/设置入口）、`HistoryList`（时间筛选标签 + 按日分组 + 条目行 + 悬停操作）、`DetailPanel`（类型标签 + 预览 + 元数据 + 操作区）、`SettingsView`（忽略应用管理）、`TrashPlaceholder`。
5. `App.tsx`：装配三栏；`useEffect` 启动 `load()` + 订阅 `clipboard-changed` 实时追加（卸载 `unlisten`）；toast 自动消失。
6. 后端新增 `copy_item` 命令（arboard `set_text`，文本/链接/代码写回系统剪贴板；图片/文件 P3 暂不支持）。

验证标准：
- [x] `tsc --noEmit` 0 error，无 `any`（已验证：`npm run build` 即 tsc + vite build，60 模块打包成功）。
- [x] 后端 `cargo clippy --all-targets -- -D warnings` 0 告警；`cargo test` 15/15 通过（P1 9 + P2 6，未受影响）。
- [x] **视觉**：三栏布局 + 设计 Token（`tokens.css` 与 Ardot 稿一致：accent #059669、sidebar 240px、圆角 6/10/14）。
- [ ] **手动**：复制新内容，列表顶部实时出现该条目（含分组「今天」）。
- [ ] **手动**：详情面板点「复制」→ 系统粘贴验证内容确被写回（文本/链接/代码）。
- [ ] **手动**：置顶项恒在列表最前并高亮；收藏标记持久（重启后保持）。
- [ ] **手动**：删除移至回收站（回收站视图 P3 为占位，恢复/清理留待后续）。
- [ ] **手动**：设置 → 忽略应用：添加后该应用复制不再被捕获（即时生效 + 持久）。

---

## 阶段 P4 · 托盘 / 菜单栏 + 全局快捷键

**目标**：不打开主界面也能一击取历史。

步骤：
1. `tray.rs`：mac 菜单栏图标下拉（最近 3–5 条 + 打开主界面 + 设置 + 退出）；Win 托盘右键菜单（弹层向上）。
2. `shortcut.rs`：`tauri-plugin-global-shortcut` 绑定 `⌘⇧V` / `Ctrl Shift V` 唤起主界面，可自定义。
3. 点击托盘历史项 → 复制并关闭弹层。

验证标准：
- [ ] **手动（mac）**：菜单栏图标点击出最近历史；点条目即复制。
- [ ] **手动（Win 逻辑）**：托盘右键菜单同上，弹层向上。
- [ ] **手动**：全局快捷键唤起已最小化的主界面；改键后新键生效。
- [ ] 多实例加单例锁，重复启动不双监听。

---

## 阶段 P5 · 检索 / 分类 / 置顶 / 收藏

**目标**：完整的内容管理能力。

步骤：
1. 搜索框（`⌘K` 聚焦）全文检索：标题 / 内容 / 来源。
2. 分类切换（全部 / 文本 / 链接 / 图片 / 代码 / 文件）+ 时间筛选（今天 / 本周 / 全部）。
3. 翻译操作（本地词典或翻译 API 占位）。

验证标准：
- [ ] **单测**：分类识别与搜索匹配逻辑正确。
- [ ] **手动**：输入关键字，列表精确过滤；切换分类 / 时间筛选结果正确。
- [ ] **手动**：置顶 / 收藏状态跨重启保持。

---

## 阶段 P6 · 设置 + 安全

**目标**：可日常使用、隐私可控。

步骤：
1. 设置页：常规 / 捕获 / 存储 / 快捷键 / 关于（对齐设计稿「设置界面」）。
2. 设置写 `settings` 表，重启生效。
3. 开机自启开关；忽略应用管理 UI。
4. 能力收敛：`capabilities/` 仅声明所需权限；敏感字段加密（可选 sqlcipher）。

验证标准：
- [ ] **手动**：修改主题 / 历史上限 / 忽略应用，重启后保持。
- [ ] **手动**：开启开机自启后注销重登自动运行。
- [ ] **安全**：`tauri.conf.json` 无多余权限；剪贴板读取受忽略列表约束。

---

## 阶段 P7 · 打包签名与分发

**目标**：产出可在无 Rust 环境的机器上独立运行的安装包。

步骤：
1. `tauri build` 出 macOS `.dmg` / `.app`（签名 + 公证）。
2. CI（GitHub Actions）出 Windows `.msi`（WebView2 引导器内嵌）。
3. 体积 / 内存基线检查。

验证标准：
- [x] `tauri.conf.json` schema 合法（`cargo build --release` 经 `build.rs` 解析通过）；release profile 编译 0 error。
- [x] 已修复跨平台 `bundle.targets` 冲突：移除全局 `["dmg","app","msi"]`（macOS runner 会因 msi 不支持报错），改用平台默认目标。
- [ ] **干净环境**：在**未安装 Rust** 的虚拟机装包并正常运行。
- [ ] mac 包通过 Gatekeeper；Win 包安装无 WebView2 缺失报错。
- [ ] 安装包体积在合理范围（目标几 MB ~ 几十 MB）。
- [ ] 常驻内存占用低于对标 Electron 工具一个数量级。

---

## 确认后即开始的动作

你确认后，我将从 **阶段 P0** 起实施：初始化 Tauri 工程、落地目录结构、跑通最小窗口，并每一步附验证结果回报。后续阶段按上述清单逐段推进，任一段验证未过会先停下来跟你同步。
