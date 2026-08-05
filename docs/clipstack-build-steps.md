# ClipStack 开发步骤与验证清单

> 状态：**开发中（P0 脚手架进行中）**
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
1. `store/`（Zustand）：history / settings / ui 三个 store。
2. `api/`：封装 `invoke`，类型与 Rust 模型对齐，禁止 `any`。
3. 组件：Sidebar（搜索 + 分类 + 筛选）、主列表（分组 + 条目卡片）、详情面板（预览 + 元数据 + 操作）。
4. 监听 `clipboard-changed` 事件 → 列表实时追加（订阅在 `useEffect`，卸载 `unlisten`）。
5. 点击条目 → 调 command 写回剪贴板并高亮。

验证标准：
- [ ] **视觉**：三栏布局、配色、间距与 Ardot 设计稿截图一致。
- [ ] **手动**：复制新内容，列表顶部实时出现该条目。
- [ ] **手动**：点击某条目，系统粘贴验证内容确被写回剪贴板。
- [ ] **手动**：置顶项恒在列表最前并高亮；收藏标记持久。
- [ ] `tsc --noEmit` 0 error，无 `any`。

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
- [ ] **干净环境**：在**未安装 Rust** 的虚拟机装包并正常运行。
- [ ] mac 包通过 Gatekeeper；Win 包安装无 WebView2 缺失报错。
- [ ] 安装包体积在合理范围（目标几 MB ~ 几十 MB）。
- [ ] 常驻内存占用低于对标 Electron 工具一个数量级。

---

## 确认后即开始的动作

你确认后，我将从 **阶段 P0** 起实施：初始化 Tauri 工程、落地目录结构、跑通最小窗口，并每一步附验证结果回报。后续阶段按上述清单逐段推进，任一段验证未过会先停下来跟你同步。
