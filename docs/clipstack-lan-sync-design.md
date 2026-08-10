# ClipStack 局域网剪贴板共享（简易版）· 技术设计文档

> 状态：设计稿（待实施）
> 日期：2026-08-10
> 范围：同一局域网内多台电脑之间的剪贴板共享（**不含**公网、NAT、穿透、跨网段）
> 关联文档：`clipstack-sync-design.md`（全量「中继 + E2E + NAT 穿透」设计，本方案为其裁剪版）

---

## 一、背景与目标

ClipStack 当前是 **Tauri 2 + React + Rust + SQLite** 的本地优先剪贴板管理器。既有 `clipstack-sync-design.md` 规划了跨网络 / 跨 NAT 的「中转中继 + 端到端加密」方案，复杂度较高（公网中继、Cloudflare Tunnel、ECDH、钥匙串包装等）。

本方案是其**简易裁剪版**，先把范围收紧到「同一局域网内共享」，目标：

- **同局域网多机互享**：A 机复制，同局域网内其他机器近实时收到。
- **零部署、开箱即用**：同子网内无需填 IP、无需服务器。
- **参数一致才共享**：只有「共享组 + 密钥」两组参数都相同的客户端才互相共享。
- **≥2 台、任意退出不影响其余**：无中心节点，天然支持多客户端与容错。

> 跨 VLAN / 跨公网不在本期范围。跨 VLAN 场景统一用「手动配置对端 IP:端口」兜底，**不引入中继、不做双模式**。

---

## 二、总体架构

**选定方案：无服务器 P2P 全 mesh + mDNS 自动发现 + 配对参数 PSK 加密。**

| 维度 | 本方案（简易版） | 全量设计（参考） |
|---|---|---|
| 拓扑 | 无服务器全 mesh 直连 | 经公网中继中转 |
| 发现 | mDNS（同子网）+ 手动 IP（兜底） | 中继登记 |
| 加密 | 共享组 + 密钥 → PSK（PBKDF2 + AES-GCM） | ECDH 协商 + 钥匙串包装 |
| 跨网/NAT | 不支持（手动 IP 仅覆盖已路由的跨 VLAN） | 支持（Cloudflare Tunnel） |
| 部署 | 零部署（同子网） | 需自托管中继 |

**核心抽象（与全量设计一致）**：把每次复制当作一条**不可变事件（append-only）**，带全局唯一 `sync_id`、来源 `device_id`、`lamport` 逻辑时钟与 `hash`；排序靠 `(lamport, device_id)`，去重靠 `hash` / `sync_id`，回环靠 `is_remote` 标记。协议层完全复用全量设计，差异只在「路由方式」（广播给 mesh peers vs 发往中继）。

---

## 三、服务发现（mDNS）

### 3.1 原理
mDNS（Multicast DNS, RFC 6762）在局域网内**无需中心服务器**，设备把 DNS 查询/应答发到多播地址 `224.0.0.251:5353`（UDP，IPv6 为 `ff02::fb:5353`），路由器默认不转发——恰好契合「只同局域网」定位。DNS-SD（RFC 6763）在其上用服务名归类。

### 3.2 在本方案中的用法
- **注册（被找）**：每台客户端启动时注册服务实例，例如 `MyMacBook._clipstack._tcp.local`，监听固定端口（默认 `8787`），并带：
  - **SRV 记录**：本机 IP + 端口
  - **TXT 记录**：`device_id`、`name`、`version`、`group_fp`
- **发现（找人）**：持续发 PTR 查询 `_clipstack._tcp.local`，收到同服务的实例应答后，再查 SRV+TXT 拿到对方 IP:端口与指纹。
- **上线感知**：新机器连网后主动发通告（gratuitous response），其他机器无需轮询即知。
- **下线感知**：对方发 `TTL=0` 的 goodbye 包，或本地缓存/心跳超时，即从 `peers` 移除。

### 3.3 共享参数与指纹（分组 + 鉴权）
客户端维护两个共享设置：

1. **共享组（`share_group`）**：逻辑分组标识（如 `home` / `office`）。
2. **密钥（`share_key`）**：对称密钥源，私密。

**一致性门槛**：两端必须**共享组 与 密钥同时完全一致**才建立共享。实现上派生唯一匹配指纹，mDNS TXT 只广播它：

```
group_fp = SHA256(share_group ‖ "::" ‖ share_key)   取前 8 字节
```

- 共享组不同 → 指纹不同 → 不连接。
- 密钥不同 → 指纹不同 → 不连接。
- 两者都一致 → 指纹必一致（抗碰撞）→ 才连接共享。
- 指纹**不含密钥原文**，LAN 广播也不泄露密钥。

### 3.4 第三方库选型
| 库 | 性质 | 对用户/打包影响 |
|---|---|---|
| **`mdns-sd`** ✅ 采用 | 纯 Rust 实现，自带 responder + browser，**不依赖系统 Avahi/Bonjour** | 编译进二进制，**终端用户零额外安装**，三平台行为一致 |
| `zeroconf` | 封装系统 DNS-SD（macOS Bonjour / Linux Avahi / Windows Bonjour） | Linux 端需系统有 Avahi，Windows 老版本需 Bonjour，否则发现不了 |

> 结论：采用 **`mdns-sd`（纯 Rust）**，分发时用户无需安装任何系统库。运行时不强制装系统组件（macOS 内置 Bonjour；Windows 10 2004+ 原生支持；Linux 桌面通常预装 Avahi，但本方案不依赖它）。

### 3.5 兜底：手动 peer（含跨 VLAN）
mDNS 多播不过路由器/VLAN，**跨 VLAN 不支持自动发现**。统一用「手动添加对端 IP:端口」列表兜底：

- 前提：两个 VLAN 之间已做 L3 路由且防火墙放行 `8787` 端口，直连 WS 即可建立。
- 若组织策略完全禁止 VLAN 间路由，则纯应用层无解（属 NAT/穿透范畴，本期不做）。
- 手动 peer 与 mDNS 发现的节点合并进同一 `peers` 映射，后续收发逻辑完全一致。

---

## 四、连接与 mesh 收敛

- 每台客户端**既是监听方（绑 `8787`）也是发起方**。
- 发现对端后，按 `device_id` **字典序**决定谁主动：**较大者向较小者发起连接**，较小者只监听。任意两机之间恒为单条连接，避免双向双连。
- 维护 `peers: HashMap<device_id, WsStream>`，带断线指数退避重连。
- **≥2 台全互联**：N 台 → 每台上连 N-1 台，构成全 mesh。新机器上线被其余自动发现接入。
- **任意退出不影响其余**：mesh 无中心节点；某台退出/掉线，其余只在自己 `peers` 里删掉它，**剩余 N-1 台连接原样保留、继续互享**（对比「本地中继」方案：中继机一挂全体断联，本方案无此单点）。

---

## 五、传输协议（复用，零新协议）

直接复用全量设计的 `SyncEnvelope` 与信封类型，语义微调为「对等」：

- **本地捕获**：→ 加密 `SyncEnvelope` → **广播给 `peers` 里所有已连对端**（原设计是发给中继）。
- **收到对端 `Clip`**：→ 解密 → 按 `hash`/`sync_id` 去重 → 写 `history(is_remote=1)` → **不向第三方转发**（回环/放大防护）。
- `lamport` 逻辑时钟、`(lamport, device_id)` 排序、`share_out` 门控全部沿用。

线上信封（与全量设计一致）：

```
SyncEnvelope {
  sync_id:    uuid            // 条目全局唯一 id
  device_id:  string          // 来源设备
  lamport:    u64             // 逻辑时钟，用于排序
  kind:       string          // text | link | code | image | file
  hash:       string          // 内容去重
  nonce:      [u8;24]         // XChaCha20/AES-GCM nonce
  ciphertext: [u8]            // 加密后的 ClipboardItem
}
```

---

## 六、加密（PSK 简化）

LAN 威胁模型不同于公网，采用轻量 PSK：

```
sym_key = PBKDF2-HMAC-SHA256(share_key, salt = SHA256(share_group), rounds = 100_000)
```

- 内容用现有 `crypto.rs` 的 AES-256-GCM 逐条加密，随机 nonce 随 `SyncEnvelope` 走。
- 密钥来源从全量设计的 ECDH 协商替换为「共享组 + 密钥」这组共享参数，无需钥匙串包装协商（但 `share_key` 落库仍建议经本机钥匙串包装为 `wrapped_key`）。

---

## 七、发布 / 接收分离（share_out）

复用全量设计：

- **接收（receive）**：只要该共享配置处于激活态，本机**始终接收**组内他人内容并入库（自动粘贴由总开关控制）。
- **发布（publish）**：由本机 `share_out` 开关控制。关闭后停止**未来**发布，但照样收他人共享。
- 关闭共享仅停发未来内容，已发到对端的副本无法撤回（UI 需明示）。

---

## 八、数据模型（SQLite 增量）

`sync_profiles` 增列，LAN 配置即「一个共享组 + 一个密钥」：

```sql
ALTER TABLE sync_profiles ADD COLUMN mode        TEXT DEFAULT 'relay';  -- 'lan' | 'relay'
ALTER TABLE sync_profiles ADD COLUMN share_group TEXT;                  -- 共享组
-- share_key 不裸存：经本机钥匙串(keyring-rs)包装为 wrapped_key，复用全量机制
```

沿用全量设计的**多配置单激活**：可存多个 LAN 配置（不同共享组），同一时刻仅一个激活，切换即断旧连新（`is_active` 互斥）。

`history` 增量列（与全量设计一致）：

```sql
ALTER TABLE history ADD COLUMN sync_id       TEXT;
ALTER TABLE history ADD COLUMN origin_device TEXT;
ALTER TABLE history ADD COLUMN lamport       INTEGER;
ALTER TABLE history ADD COLUMN profile_id    TEXT;
ALTER TABLE history ADD COLUMN is_remote     INTEGER;   -- 1=来自共享，不触发回环
```

---

## 九、文件传输

LAN 带宽大、延迟低，默认上限可从全量设计的 25MB 提到**可配置**（如 100MB）；超过仍只本机保留、不广播。`share_out` 关闭时同样不发布文件。

---

## 十、UX（设置面板「局域网共享」）

- 共享组（文本输入）
- 密钥（密码框，可显隐）
- 共享本机剪贴板开关（`share_out`，沿用）
- 手动 peer 列表（IP:端口，跨 VLAN / 组播被禁兜底）
- 组内在线设备列表（来自 mDNS 发现 + 已连 peers，显示设备名）
- 历史条目来源标识（本机 / 设备名）

---

## 十一、分阶段实施

| 阶段 | 交付物 | 验证方式 |
|---|---|---|
| **L1 协议抽取** | 新建 `clipstack-protocol` crate：`SyncEnvelope` + 去重 / Lamport 排序 / 回环防护（带单测） | 内存 SQLite 离线模拟双端收发，`cargo test` |
| **L2 发现 + 单连** | `mdns-sd` 发现 + 按 `device_id` 单边发起 + 两台机直连收发 | 两机实机验证信封往返 |
| **L3 mesh + 加密** | 多对端广播、`group_fp` 分组、PSK 派生密钥、`share_out`、自动粘贴开关、手动 peer 兜底 | 多机互拷 + 跨 VLAN 手动配置 |
| **L4 UX 与文件** | 局域网共享设置面板、配对参数输入、组内设备列表、来源标识、文件大小上限（可配置） | 手动验收 |

### 仓库布局（参考全量设计）

```
clipboards/
├─ clipstack-protocol/   # 共享协议库（纯类型 + 去重/排序/回环逻辑，无 IO）
│   └─ Cargo.toml
└─ clipstack/src-tauri/  # 客户端：新增 lan_sync.rs，依赖 clipstack-protocol + mdns-sd + tokio-tungstenite + rustcrypto
```

---

## 十二、与全量设计的关系

- 协议层（`SyncEnvelope`、去重、Lamport、回环）**完全一致**。
- 差异只在「路由方式」：全量是中继转发，本方案是 mesh 广播；将来要跨公网，加一个 `relay` 模式、把广播换成发往中继即可，**上层代码几乎不动**。
- 加密层：本方案用 PSK（共享组+密钥）替代 ECDH，更轻；如需更强安全可平滑切换回 ECDH。

## 十三、已知限制

- mDNS 只在本网段，跨 VLAN 需手动配置对端（不支持自动发现）。
- 首次运行系统可能弹防火墙允许（UDP 5353 / TCP 8787）。
- 组播被禁的网络环境（部分公司网）自动发现失效，需手动 peer。
- 不覆盖跨公网 / NAT 穿透场景（见全量设计）。
