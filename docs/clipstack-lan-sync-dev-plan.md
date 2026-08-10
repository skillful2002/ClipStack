# ClipStack 局域网剪贴板共享 · 开发计划

> 状态：**L1–L4 已全部实现并代码层验收通过**（2026-08-10）
> 日期：2026-08-10
> 依据：`clipstack-lan-sync-design.md`（局域网共享技术设计，2026-08-10）
> 关联：`clipstack-development-plan.md`（总体编码规范）、`clipstack-sync-design.md`（全量中继设计，本方案为其裁剪版）

---

## 〇、本计划用途

依据局域网共享设计文档，把 L1–L4 四个阶段展开为**可执行、可验收**的开发任务清单。
**本文件为待确认稿**：在你确认前不会动任何代码；确认后按第四节顺序落地。

---

## 一、目标与范围（引自设计）

- **同局域网多机互享**：A 机复制，同局域网内其他机器近实时收到。
- **零部署、开箱即用**：同子网内无需填 IP、无需服务器。
- **参数一致才共享**：只有「共享组 + 密钥」两组参数都相同的客户端才互相共享。
- **≥2 台、任意退出不影响其余**：无中心节点，天然多客户端与容错。
- **本期不做**：跨公网 / NAT 穿透（属全量设计范畴）。跨 VLAN 仅提供「手动配置对端 IP:端口」兜底，**不引入中继、不做双模式**。

---

## 二、关键技术决策（摘要，供确认）

| 项 | 决策 | 说明 |
|---|---|---|
| 拓扑 | 无服务器全 mesh 直连 | 每台既是监听方（绑 `8787`）也是发起方 |
| 服务发现 | `mdns-sd`（纯 Rust） | 不依赖系统 Avahi/Bonjour，用户零额外安装 |
| 分组/鉴权 | `group_fp = SHA256(share_group ‖ "::" ‖ share_key)` 前 8 字节 | 广播指纹，不泄露密钥原文 |
| 连接收敛 | 按 `device_id` 字典序，较大者向较小者发起 | 任意两机恒为单条连接 |
| 加密 | PSK：PBKDF2-HMAC-SHA256 派生对称密钥 + 现有 `crypto.rs` AES-256-GCM 逐条加密 | 替换全量设计的 ECDH |
| 协议 | 复用 `SyncEnvelope` + 去重 / Lamport / 回环防护 | 与全量设计协议层完全一致 |
| 文件上限 | 默认 100MB（可配置），超过只本机保留 | 全量设计为 25MB |

---

## 三、现状盘点（代码库实际状态）

> 实施前需明确「已有」与「待建」，避免计划与代码脱节。

**已有（可复用）：**
- `crypto.rs`：AES-256-GCM 加解密（`Key` 封装、`encrypt`/`decrypt`），直接作信封内容加密。
- `db.rs`：`open` / `migrate` / `add_column`（`ALTER TABLE`）模板，新增表与列沿用同一模式。
- `keychain.rs`：钥匙串 `wrapped_key` 持久化，PSK 的 `share_key` 落地复用。
- 事件总线：Tauri `Emitter` / `Listener`，前端 `subscribe` + 卸载 `unlisten`（见 `lib.rs` 既有事件接线）。

**待建（计划内新增）：**
- ⚠️ `sync_profiles` 表**当前不存在**（设计文档假设它已存在）。本计划 L3 改为**新建**该表。
- 新协议库 `clipstack-protocol`（设计 L1）。
- 后端新模块 `lan_sync.rs`（发现 / 连接 / mesh / 收发）。
- 前端「局域网共享」设置面板 + store + 设备列表。

**待你确认的工程结构决策（见第四节开头）：**
- `clipstack-protocol` 落地方式：① 新建 `clipboards/` 根 workspace（设计原案）；② 作为 `clipstack/src-tauri` 的 path 依赖（最小侵入）。
- 倾向方案 ①，与设计文档一致；若你更看重改动最小可选 ②。

---

## 四、分阶段实施计划

> 顺序即落地顺序。每阶段含「任务 / 产出 / 验收 / 涉及文件」。

### L1 · 协议抽取（纯类型与逻辑库，无 IO）

**任务**
1. 新建协议库（落地方式见决策）：
   - 定义 `SyncEnvelope { sync_id, device_id, lamport, kind, hash, nonce[24], ciphertext }`。
   - 实现去重（按 `hash` / `sync_id`）、Lamport `(lamport, device_id)` 排序、`is_remote` 回环防护。
   - 提供 `ClipboardItem` 序列化与「加密信封 ↔ 明文条目」编解码接口（加密细节注入，不直接依赖 `crypto.rs`，保持零 IO）。
2. 编写单测：双端内存模拟收发、乱序到达排序、重复条目去重、回环不入库。

**产出**：可独立 `cargo test` 通过的协议库。
**验收**：离线内存双端往返一致；`cargo test` 全绿；`clippy -D warnings` 无报错。
**涉及文件**：`clipstack-protocol/src/{envelope, dedup, ordering, error}.rs` + `Cargo.toml`；`clipboards/Cargo.toml`（若采用 workspace）。

---

### L2 · 发现 + 单连（mDNS + 双机直连）

**任务**
1. 在 `clipstack/src-tauri` 引入 `mdns-sd` 依赖；新增 `lan_sync.rs` 骨架（模块挂载于 `lib.rs`）。
2. 启动时注册 mDNS 服务实例 `{name}._clipstack._tcp.local`，监听端口 `8787`，TXT 带 `device_id / name / version / group_fp`。
3. 浏览器持续查 `_clipstack._tcp.local`；收到实例后查 SRV+TXT 得到对端 IP:端口与指纹。
4. `group_fp` 不一致则跳过；一致则按 `device_id` 字典序，较大者向较小者发起 WebSocket（TCP 8787）连接。
5. 单连接收发联调：本机捕获 → 加密 `SyncEnvelope` → 发给对端；对端解密 → 去重 → 暂存（先不入库，确认往返）。
6. 上线/下线：`group_fp` 同组主动发通告；对端 `TTL=0` goodbye 或心跳超时，从 `peers` 移除。

**产出**：两台同网机器剪贴板信封可往返。
**验收**：两机实机验证信封往返成功、指纹不一致的两组互不连接；`cargo test` 绿。
**涉及文件**：`lan_sync.rs`（发现/连接/收发雏形）、`lib.rs`（挂载模块 + 启动接线）、`Cargo.toml`（加 `mdns-sd`、`tokio`、`tokio-tungstenite`）。

---

### L3 · mesh + 加密 + DB（多机、持久化、分组）

**任务**
1. **新建 `sync_profiles` 表**（设计误以为已存在，此处补建）：
   ```sql
   CREATE TABLE IF NOT EXISTS sync_profiles (
     id          TEXT PRIMARY KEY,
     mode        TEXT NOT NULL DEFAULT 'lan',   -- 'lan' | 'relay'
     share_group TEXT,
     wrapped_key TEXT,                           -- 经钥匙串包装的 share_key，不裸存
     is_active   INTEGER NOT NULL DEFAULT 0,
     created_at  INTEGER
   );
   ```
   沿用「多配置单激活」：`is_active` 互斥，切换即断旧连新。
2. `history` 增量列（同设计 §8）：`sync_id / origin_device / lamport / profile_id / is_remote`。
3. PSK 派生：`sym_key = PBKDF2-HMAC-SHA256(share_key, salt=SHA256(share_group), rounds=100_000)`，内容用 `crypto.rs` AES-256-GCM 逐条加密，随机 nonce 随信封。
4. `share_out` 发布开关：关闭则停止未来发布、照常收他人（复用既有总开关门控）。
5. mesh 广播：本地捕获 → 加密信封 → 广播给 `peers` 所有已连对端；收到他人 `Clip` → 解密 → 按 `hash`/`sync_id` 去重 → 写 `history(is_remote=1)` → **不向第三方转发**（防回环/放大）。
6. 指数退避重连；≥2 台全互联；某台退出其余不受影响。
7. 手动 peer 兜底：IP:端口列表，与 mDNS 发现合并进同一 `peers`，跨 VLAN（已做 L3 路由且放行 8787）直连。
8. 命令与事件接线（草案见第五节）。

**产出**：多机互享、参数一致才共享、断线自恢复、配置可持久化。
**验收**：3 台机互拷任一内容，其余近实时出现且 `is_remote=1`；密钥/组不一致不共享；杀掉任一节点其余照常；跨 VLAN 手动 peer 可通；`cargo test` 绿。
**涉及文件**：`lan_sync.rs`（加密/广播/mesh/手动 peer）、`db.rs`（建表 + 迁移 + 读写命令）、`crypto.rs`（复用，新增 PSK 派生函数）、`commands.rs`、`lib.rs`（启动/事件接线）、`Cargo.toml`（`pbkdf2`/`sha2` 等）、`models.rs`（`SyncProfile` 等结构）。

---

### L4 · UX 与文件（设置面板、来源标识、文件上限）

**任务**
1. 「局域网共享」设置面板（React）：共享组输入、密钥密码框（可显隐）、`share_out` 开关、手动 peer 列表、组内在线设备列表、文件大小上限（可配置）。
2. 组内在线设备列表：来自 mDNS 发现 + 已连 `peers`，显示设备名；经 `lan-peer-online/offline` 事件实时刷新。
3. 历史条目来源标识：本机 / 对端设备名（读 `origin_device`，`is_remote` 区分样式）。
4. 文件传输：上限默认 100MB 可配置；超上限本机保留、不广播；`share_out` 关闭不发布文件。
5. i18n：面板文案接入既有 `i18n` 体系（中/英）。

**产出**：可日常使用的局域网共享 UI。
**验收**：手动验收——配置保存后自动重连、设备列表实时、来源标识正确、超大文件不广播且提示；前端 `tsc --noEmit` 0 error + `Vitest` 相关单测通过。
**涉及文件**：`src/components/` 新增设置子组件、`src/store/`（新增 lan sync store）、`src/api/`（invoke 封装）、`src-tauri` 命令与事件（`lan-peer-*`、`sync-profile-changed`）。

---

## 五、前后端契约草案（确认后细化）

**Tauri commands（命名 `动词_名词`，`Result<T, String>`）：**
- `get_sync_profiles() -> Vec<SyncProfile>`
- `upsert_sync_profile(profile) -> ()`
- `set_active_profile(id) -> ()`  // 断旧连新
- `delete_sync_profile(id) -> ()`
- `add_manual_peer(addr) -> ()` / `remove_manual_peer(addr) -> ()`
- `set_share_out(enabled: bool) -> ()` / `get_share_out() -> bool`
- `get_lan_peers() -> Vec<PeerInfo>`  // 在线设备

**Tauri events（`kebab-case`）：**
- `lan-peer-online` / `lan-peer-offline`（payload：`PeerInfo`）
- `lan-clipboard-received`（payload：`sync_id`，驱动历史刷新）
- `sync-profile-changed`（配置增删/激活切换后刷新面板）

> 后端 `snake_case`，前端 `#[serde(rename_all="camelCase")]`；前端在 `useEffect` 内 `subscribe`，卸载 `unlisten`。

---

## 六、数据模型变更汇总（SQL）

```sql
-- L3 新建（设计文档误记为「增列」，实际表不存在）
CREATE TABLE IF NOT EXISTS sync_profiles (
  id          TEXT PRIMARY KEY,
  mode        TEXT NOT NULL DEFAULT 'lan',
  share_group TEXT,
  wrapped_key TEXT,
  is_active   INTEGER NOT NULL DEFAULT 0,
  created_at  INTEGER
);

-- L3 history 增量列（若已存在则跳过）
ALTER TABLE history ADD COLUMN sync_id       TEXT;
ALTER TABLE history ADD COLUMN origin_device TEXT;
ALTER TABLE history ADD COLUMN lamport       INTEGER;
ALTER TABLE history ADD COLUMN profile_id    TEXT;
ALTER TABLE history ADD COLUMN is_remote     INTEGER;   -- 1=来自共享，不触发回环
```

---

## 七、编码与测试规范（复用总体计划）

- Rust：`edition 2021`，`rustfmt` 默认，`clippy -D warnings` 视为错误；单文件 > 300 行考虑拆分。
- 前端：TS `strict`、Prettier + ESLint；**禁止 `any`**；函数式组件；样式绑定 design token，**禁止硬编码颜色**。
- 测试门：Rust `cargo test` + `clippy`；前端 `Vitest` + `tsc --noEmit`；CI 三道门 lint→test→build。
- 注释讲 **why**；禁止裸 `TODO`/`FIXME`（须带 issue 引用，本期可用 `#Lx` 阶段标记代替）。
- Git：分支 `feature/lan-sync`；提交遵循 Conventional Commits（`feat(lan): ...`）；PR 至少 1 review，CI 全绿合入。

---

## 八、风险与注意

- **首次运行防火墙**：系统可能弹 UDP 5353 / TCP 8787 允许，安装引导需提示（属已知限制 §十三）。
- **组播被禁环境**（部分公司网）：自动发现失效，需手动 peer；UI 需明示。
- **workspace 改动风险**（若选方案 ①）：根 `Cargo.toml` 新增 workspace、`clipstack/src-tauri` 纳入成员，需验证 Tauri 构建链路不受影响。
- **钥匙串包装**：`share_key` 落库经 `wrapped_key`，解密失败需有明文回退或明确报错，不阻断启动。
- **回环/放大**：收到他人 `Clip` 绝不向第三方转发，本机捕获写入 `is_remote=1` 不再次触发发布。

---

## 九、确认结论（已开工并落地）

1. **`clipstack-protocol` 落地方式**：✅ 采用 **① 根 workspace**（`clipboards/Cargo.toml` 新增 workspace，`clipstack-protocol` 为成员，`clipstack/src-tauri` 以 path 依赖引入）。
2. **实施范围**：✅ **L1–L4 全做**。
3. **默认文件上限**：✅ **100MB**（设计建议）。

---

## 十、验收总览（完成后逐条核对）

> 图例：✅=代码层/单测已验证　🔶=需多机实机手动验收（当前为单机环境，未做真机联调）

- [x] 协议库 `cargo test` 全绿（去重/Lamport/回环）：✅ 13 passed（含双端内存模拟、乱序排序、重复去重、回环不入库）。
- [x] L1–L4 后端单测：✅ `clipstack_lib` 56 passed（含 L3 `sync_profiles` CRUD、按 `sync_id` 去重、远程条目 `is_remote=1/origin_device`、历史暴露远程来源）。
- [x] `clippy --workspace`：✅ 仅 5 条**历史既有**警告（空 slice / doc-list / usize 同型转换 / borrow / `AppState` 缺 `Default`），无新增。
- [x] 前端 `tsc --noEmit`：✅ 0 error。
- [x] 前端 `Vitest`：✅ `translations.test.ts` 3/3（27 个 `lan.*` 键在 6 语言齐备、占位符替换、缺失回退）。
- [x] 配置文件落盘与读取：`lan.rs` 经 `keychain` 包装 `share_key`；`lan_set_config` 留空密钥即保留原值。
- [ ] 同子网零配置：🔶 两机设相同「组+密钥」即互享，不同组/密钥互不连（待实机）。
- [ ] ≥3 台全 mesh 互联，杀任一台其余照常：🔶 待实机。
- [ ] 断线指数退避自动重连：🔶 待实机（代码已含退避重连逻辑）。
- [ ] 跨 VLAN 手动 peer 可通（环境已路由+放行）：🔶 待实机。
- [x] 来源标识（本机/设备名）：✅ `HistoryItem` 携 `origin_device`/`is_remote`；`HistoryItemRow` 远程显示蓝色 `局域网 · 设备名` 徽标。
- [x] `share_out` 发布开关生效：✅ `lan_set_share_out` 命令 + 面板开关接线。
- [x] 文件上限可配置（默认 100MB）、超限仅本机保留：✅ `file_limit_mb` 配置项 + 面板输入；超限广播拦截逻辑在 `lan.rs`。
- [x] `lan-peer-online/offline` + `lan-clipboard-received` 事件接线：✅ 后端 `lan.rs` 发射，前端 `LanSettings` 订阅实时刷新设备列表。

### 交付清单（改动文件）

- **新增**：`clipstack-protocol/`（协议库）、`src-tauri/src/lan.rs`（发现/连接/mesh/收发）、`src/components/LanSettings.tsx`（设置面板）、`src/lib/i18n/translations.test.ts`（i18n 单测）、`docs/clipstack-lan-sync-design.md`（设计）、本计划文。
- **修改**：`src-tauri/src/{db,models,clipboard,commands,crypto,lib}.rs`、`src-tauri/Cargo.toml`、根 `Cargo.toml`/`Cargo.lock`、`src/{components/HistoryItemRow,components/SettingsView,types,lib/tauri,styles/app,css,lib/i18n/translations}.*`。

### 后续可选（未列入本期）

- 真机多机联调（mDNS 发现、设备列表实时、接收项可点选查看详情）。
- 完整 `tauri build` 产物验证与防火墙放行引导。
- 文件 **blob** 实际跨机传输（本期仅做大小上限门控与本地保留，未做二进制分发）。
