# ClipStack 局域网共享 · 运行时机制梳理

> 定位：本文是 [`clipstack-lan-sync-design.md`](./clipstack-lan-sync-design.md) 的**运行时补充**，聚焦「共享发现 / 连接建立 / 断开重连 / 首次打开 / 在线设备列表」五大机制在代码里的实际流转。
> 代码权威来源：`clipstack/src-tauri/src/lan.rs`（核心，约 2280 行）、`crypto.rs`、`clipstack-protocol/src/`。
> 注：设计稿中端口写为 `8787`，**实际代码为 `LAN_PORT = 21995`**，本文以代码为准。

---

## 0. 核心结论速览

- **默认关闭、不广播**：首启动不注册 mDNS、不监听、不发任何数据包。
- **无 per-device 配对 / 无 token**：共享 = 两端「共享组 + 密钥」完全一致，由 PSK 对称加密完成鉴权。
- **无服务器全 mesh**：每机既是监听方也是发起方；按 `device_id` 字典序决定谁连谁，避免双向双连。
- **心跳 15s、静默超时 45s、重连指数退避上限 30s**。
- **在线设备列表**由 Tauri 事件 `lan-peer-online/offline` 实时驱动前端，含多重防护过滤幽灵设备。

---

## 1. 共享发现机制（mDNS 服务发现）

**库**：`mdns-sd`（纯 Rust，不依赖系统 Avahi/Bonjour）。

**常量**（`lan.rs:35-36`）：
```rust
pub const SERVICE_TYPE: &str = "_clipstack._tcp.local.";
pub const LAN_PORT: u16 = 21995;
```

### 1.1 广播自己（注册）
`LanManager::start()`（`lan.rs:356`）：
1. `ServiceDaemon::new()` 创建 mDNS 守护（`lan.rs:372`）。
2. 组装 TXT 记录：`device_id`、`name`、`version`、`group_fp`（分组指纹）（`lan.rs:386-395`）。
3. `ServiceInfo::new(SERVICE_TYPE, instance, host, ip, port, props)`（`lan.rs:396`）—— 实例名只传纯名，`mdns-sd` 自动拼成 `实例名._clipstack._tcp.local.`；**注意不能带服务类型后缀**，否则全名畸形、互相发现失败（代码注释 `lan.rs:380-385` 专门警告）。
4. `mdns.register(info)`（`lan.rs:405`）注册。

### 1.2 发现对端（浏览）
`mdns.browse(SERVICE_TYPE)`（`lan.rs:411`）；事件在独立任务循环 `browse_rx.recv()` → `handle_mdns_event`（`lan.rs:868`）。

### 1.3 分组指纹过滤（安全隔离）
`crypto::group_fingerprint`（`crypto.rs:101`）：取 `SHA256(group::key)` **前 8 字节**作为 `group_fp` 广播，**绝不泄露密钥原文**。
`handle_mdns_event`（`lan.rs:884-888`）比对 `fp != my_fp` 直接跳过 —— 不同组 / 不同密钥连不到一起。

### 1.4 有限次重宣告（解决错过首播）
`lan.rs:483`：启动后 2s / 7s / 15s 各重注册一次（有限次，非常驻），避免对端启动时错过本机首次宣告而看不到。

### 1.5 角色判定（避免双向双连）
`lan.rs:912-926`：按 `device_id` 字典序：
- `device_id >= my_id` → 本机**仅监听**；
- `device_id < my_id` → 本机作为**客户端**发起连接。

两端角色互补，任意两机之间恒为单条连接。

### 1.6 device_id 碰撞自愈
`lan.rs:914-923`：若发现对端 `device_id == my_id`（克隆 / 还原镜像导致），后台自动重生 `device_id`（`resolve_device_id_collision`，`lan.rs:325`），并以 30s 冷却避免风暴。

---

## 2. 连接建立机制（WebSocket 同步）

**传输**：`tokio-tungstenite`（WebSocket，mesh 直连）。

### 2.1 谁 server / 谁 client
- **Server**：`start()` 内 `lan.rs:434-471` 每机 `TcpListener::bind(("0.0.0.0", listen_port))`，`accept_async` 接受；由本机 `device_id` 较小的一侧承担。
- **Client**：仅当本机 `device_id` 较大时，`handle_mdns_event` 经 `spawn_client_retry`（`lan.rs:991`）向对端 `ws://{addr}` 发起（`lan.rs:1010` `connect_async`）。
- 两侧连接建立后**统一走 `accept_peer`（`lan.rs:1040`）**（泛型 `S` 同时兼容 `TcpStream` 与 `MaybeTlsStream`），收发逻辑一致。

### 2.2 握手 / 鉴权（无配对、无 token）
无 per-device 配对、无 token、无 ECDH 协商。鉴权即「是否知道同一个共享组密钥」：
- 对称 PSK 由共享参数派生：`crypto::derive_psk`（`crypto.rs:114`）= `PBKDF2-HMAC-SHA256(share_key, salt=SHA256(share_group), 100_000 轮)`。
- 不同组 / 密钥在 **mDNS 指纹层**即被隔离，连不上；连上后用 PSK 逐条加解密。

**Hello 握手**（`lan.rs:1081-1087`）：连接建立即互发 JSON `{"type":"hello","device_id":...,"name":...}`。
- Client 侧 mDNS 已知对端 id，立即 `register_conn`（`lan.rs:1078`）；
- Server 侧从 hello 学到对端 id 后再登记（`lan.rs:1126-1154`）。

使「在线列表」在连接后即可显示，无需等首条剪贴板。

### 2.3 自研分片协议（绕开大消息阻塞心跳）
常量：`LAN_CHUNK = 32*1024`（`lan.rs:1765`），`FRAME_START=0x01`（`lan.rs:1769`）、`FRAME_DATA=0x02`（`lan.rs:1770`）。
- `START` 帧 = `[0x01][u32 BE total][首片]`；`DATA` 帧 = `[0x02][后续片]`；
- 收齐 `total` 字节才反序列化为 `SyncEnvelope`（`lan.rs:1156-1188`）。
- 分片让 Ping/Pong 可穿插在传输中，避免大文件传输时心跳误杀。

### 2.4 加密信封
`build_envelope`（`lan.rs:1668`）用 `psk` 对明文 payload 做 AES-256-GCM（`crypto::encrypt`，`crypto.rs:71`）→ 构造 `SyncEnvelope`（`clipstack-protocol/src/envelope.rs:27` 字段：`sync_id/device_id/source_app/lamport/kind/hash/nonce/ciphertext`）。接收端在 `accept_peer`（`lan.rs:1221-1230`）用 `psk` 解密。

### 2.5 协议层去重 / 回环（不转发第三方）
`cli­pstack-protocol/src/store.rs:64` `ClipStore::ingest`：
1. **回环**：`env.device_id == self_device_id` → `Loopback`（丢弃，不入库不转发）；
2. **去重**：按 `hash` 或 `sync_id` 双键（`dedup.rs`，`Duplicate`）；
3. 解密 → 推进 Lamport 时钟 → 入库（`Stored`）。`is_remote=1` 落库（`lan.rs:1414` `insert_remote_clip`）。

---

## 3. 断开处理机制

### 3.1 心跳 / 断线检测
读循环 `accept_peer`（`lan.rs:1095-1486`）：
- 每 **15s** 发 `Ping`（`lan.rs:1101`），对端底层自动回 `Pong`；
- `idle = 45s`，用 `last_activity`（收到**任何帧含 Pong** 即刷新，`lan.rs:1107`）计时，**非读空闲**；
- `tokio::select!` 同时等 `read.next()` 与 `ping.tick()`（`lan.rs:1114`）；
- `last_activity.elapsed() > idle`（`lan.rs:1481`）判定对端断电 / 断网，break 关闭连接。

> 这样大文件传输期间底层仍回 Pong，不会被误杀。

### 3.2 断开后处理
读循环 break → `remove_conn`（`lan.rs:1647`）：从 `Inner.conns` 移除（`lan.rs:1650`）并 `emit("lan-peer-offline", …)`（`lan.rs:1655`）。

### 3.3 重连策略（指数退避）
`spawn_client_retry`（`lan.rs:991-1035`）：`backoff` 起始 1s，`backoff = (backoff*2).min(30)`，上限 **30s**；连接关闭后 sleep 退避再重连，循环直到 `stop` 置位或对端被移除。

mDNS `ServiceRemoved`（`lan.rs:944-985`）会置 `stop`、清 `known_peers`、主动 `abort` 读任务（真关 socket），对端立即检测到下线。

### 3.4 主动关闭（stop）
`LanManager::stop()`（`lan.rs:584`）：
1. 给所有连接发 `Close` 帧（`lan.rs:592`）；
2. 递增 `gen` 使旧读任务失效（`lan.rs:598`）；
3. `mdns.shutdown()`（`lan.rs:600`）；
4. 置 `client_stops`（`lan.rs:602`）；
5. **abort 所有连接读任务**（`conn_aborts`，`lan.rs:639-641`，真正发 FIN，对端即时下线而非等 45s 超时）；
6. 仅当共享确已关闭才 `emit lan-peer-offline`（`lan.rs:644-656`）。

### 3.5 关键防护
- `gen`（网络代际，`lan.rs:256`）：防止 stop 后残留读任务把对端插回 `conns`；
- `conn_aborts`（`lan.rs:259-261`）：防止僵尸连接。

---

## 4. 首次打开时如何处理共享

### 4.1 默认状态（核心：默认关闭，不广播）
`LanConfig::default()`（`lan.rs:57-72`）：`share_group = ""`、`share_key = ""`、`share_out = false`，注释明确「避免误广播给同子网陌生人」。

`lib.rs:164` 启动即 `lan.start()`，但 `start()` 在 `lan.rs:366-368` 检查 `!share_out` 直接 `return` —— **首启动不会注册 mDNS、不监听、不广播**。

### 4.2 持久化加载
`LanManager::new`（`lan.rs:288`）→ `load_persisted_config`（`lan.rs:157`）从 settings 表载入。首次运行无记录时，`device_id` 生成一次并**立即持久化**（`lan.rs:194-203`），保证跨启动稳定。

落库键：`lan_share_group / lan_share_key(包装) / lan_device_name / lan_share_out / lan_file_limit_mb / lan_manual_peers / lan_share_types / lan_listen_port / lan_device_id`（`persist_config`，`lan.rs:139-154`）。

### 4.3 首次运行引导
`lib.rs:188-205`：读 `first_launch_done` 标志，首次运行则显示主窗口 + Dock 可见，并置 `first_run_flag`（前端 `was_first_run` 命令 `commands.rs:625` 读取决定是否自动进设置页）。

> **引导仅显示界面，不自动开启共享** —— 用户须手动在 `LanSettings` 填组 / 密钥并拨「共享」开关。

### 4.4 开启前置校验
`missing_share_prereqs`（`lan.rs:738`）要求 组 / 密钥 / 端口 三者齐全；`set_share_out`（`lan.rs:752`）与 `set_config`（`lan.rs:542`）不满足则返回 `SHARE_PREREQ_MISSING:group,key,port`（前端 `parseLanPrereqError`，`tauri.ts:344`，`LanSettings.tsx:139-143` 弹本地化提示）。

### 4.5 启动兜底发现
`lib.rs:166-181`：启动后每 3s 若仍无任何已连对端，则 `stop()+start()` 重启 mDNS，最多约 30s，解决「重启后需手动点一次保存才看得到设备」。

### 4.6 L3 多配置（可选）
`commands.rs:1015-1106` 提供 `lan_upsert_profile / lan_list_profiles / lan_set_active_profile / lan_delete_profile`，支持多套共享配置（密钥经内部数据库密钥 wrap 落库，`wrap_profile_key` `commands.rs:1225`）。但默认仍无激活配置。

---

## 5. 在线设备列表机制

### 5.1 数据结构
- 前端 → `PeerInfo`（`lan.rs:208-213`）：`{ device_id, name, addr, connected }`（camelCase 序列化）。
- 后端内存态 → `Inner.conns: HashMap<String, ConnHandle>`（`lan.rs:246`，`ConnHandle` 含 `tx: mpsc::UnboundedSender<Message>` 用于推送信封）、`peer_names: HashMap<String,String>`（`lan.rs:252`，device_id → 友好名）。

### 5.2 后端如何维护列表
- **上线**：`register_conn`（`lan.rs:1509`）插入 `conns` 并 `emit("lan-peer-online", PeerInfo)`（`lan.rs:1604`），仅「新上线」才广播（`lan.rs:1517` `is_new`）。
- **下线**：`remove_conn`（`lan.rs:1647`）`emit("lan-peer-offline")`；另 `ServiceRemoved`（`lan.rs:974`）与 `stop()`（`lan.rs:646`）也会触发。
- **同名更新**：`update_peer_name`（`lan.rs:1617`）。
- **全量读取**：`peers()` 命令（`lan.rs:702`）从 `conns` 收集，**防御性过滤**掉「name 为空或回退为 device_id」的幽灵连接（`lan.rs:723`）。

### 5.3 多重防护
`gen` 代际（`lan.rs:1537`）、同机去重（相同 IP+name 不同 device_id 合并，`lan.rs:1554-1587`）、自连守卫（`lan.rs:1525`）、无名守卫（`lan.rs:1550`）。

### 5.4 事件如何流转到前端（Tauri Event）
- Rust 侧 `app.emit("lan-peer-online"/"lan-peer-offline"/"lan-config-changed", payload)`（`lan.rs` 多处）。
- 前端封装：`src/lib/tauri.ts:293-332` 的 `onLanPeerOnline / onLanPeerOffline / onLanClipboardReceived / onLanPortInUse / onLanConfigChanged`（基于 Tauri `listen`）。
- 前端 UI：`src/components/LanSettings.tsx`：
  - `refreshPeers`（`LanSettings.tsx:43`）调 `api.lanGetPeers()`（全量覆盖）；
  - `useEffect`（`LanSettings.tsx:78-104`）订阅上下线事件；
  - 上线 → `setPeers(prev => upsertPeer(prev, p))`（`LanSettings.tsx:82`），`upsertPeer`（`LanSettings.tsx:480-486`）按 `deviceId` 插入 / 更新；
  - 下线 → `setPeers(prev => prev.filter(x => x.deviceId !== p.deviceId))`（`LanSettings.tsx:83-85`）；
  - 后端配置被外部变更（如托盘切共享）→ `onLanConfigChanged` 触发 `refreshConfig` 重新拉取（`LanSettings.tsx:96-98`）。

---

## 6. 速查代码地图

```
启动 / 状态
  lib.rs:107 run()                       ├─ 启动 LanManager、首次运行判定
  lib.rs:164 LanManager::new             │   lan.rs:288（载入持久化配置）
  lib.rs:166 lan.start() 启动 mDNS       └─ lan.rs:356（share_out=false 时直接 return）

发现（mDNS）
  lan.rs:35  SERVICE_TYPE "_clipstack._tcp.local."
  lan.rs:36  LAN_PORT 21995
  lan.rs:372 ServiceDaemon::new
  lan.rs:405 mdns.register(info)         注册 / 广播自己
  lan.rs:411 mdns.browse(SERVICE_TYPE)   扫描对端
  lan.rs:483 有限次重宣告(2/7/15s)
  lan.rs:868 handle_mdns_event           解析对端 / 角色判定 / 指纹过滤
  crypto.rs:101 group_fingerprint        分组指纹(SHA256[:8])

连接（WebSocket）
  lan.rs:434 TcpListener 绑定(server)
  lan.rs:991 spawn_client_retry(client, 指数退避≤30s)
  lan.rs:1040 accept_peer                收发统一入口
  lan.rs:1081 hello 握手(device_id+name)
  lan.rs:1221 解密+入库(PSK=AES-256-GCM)
  lan.rs:1668 build_envelope             加密构造信封
  crypto.rs:114 derive_psk               PBKDF2 派生 PSK

断开 / 重连
  lan.rs:1101 心跳 Ping(15s) / 超时 45s
  lan.rs:1481 静默断线判定
  lan.rs:1488 remove_conn → lan-peer-offline
  lan.rs:584 stop() 主动关闭 + abort 读任务

在线列表
  lan.rs:208 PeerInfo 结构
  lan.rs:246 Inner.conns / lan.rs:252 peer_names
  lan.rs:702 peers() 全量读取（过滤幽灵）
  lan.rs:1509 register_conn → lan-peer-online
  lan.rs:1647 remove_conn  → lan-peer-offline
  src/lib/tauri.ts:293-332 前端事件订阅封装
  src/components/LanSettings.tsx:43,78,480  前端状态与 upsertPeer

协议库 clipstack-protocol
  envelope.rs:27 SyncEnvelope / :15 ClipKind
  store.rs:64  ClipStore::ingest(回环/去重/Lamport)
  dedup.rs:21  DedupSet(按 hash 或 sync_id 双键)
```

> 一句话总结：默认关闭、不广播；用户配置组+密钥并开启后，每台通过 mDNS（`_clipstack._tcp.local.` @21995）按 `device_id` 字典序决定 server/client 角色建立 WebSocket，用「组+密钥」派生的 PSK 做 AES-256-GCM 对称加密（无 per-device 配对 / token）；15s 心跳 + 45s 超时检测断线、指数退避 ≤30s 重连；在线设备经 Tauri `lan-peer-online/offline` 事件实时驱动前端 `LanSettings.tsx` 的 `peers` 列表，并有多重代际 / 去重 / 无名防护避免幽灵设备。
