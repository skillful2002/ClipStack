// 局域网共享（L2/L3，见 `docs/clipstack-lan-sync-design.md`）。
//
// 设计要点：
// - 无服务器全 mesh 直连；每台既是监听方（绑 LAN_PORT）也是发起方。
// - mDNS 自动发现（`mdns-sd`，纯 Rust，不依赖系统 Bonjour/Avahi）。
// - 发现对端后按 `device_id` 字典序决定谁主动：较大者向较小者发起连接，避免双向双连。
// - 信封 `SyncEnvelope` 复用 clipstack-protocol；内容用 crypto.rs AES-256-GCM 逐条加密（PSK）。
// - 回环防护 / 去重由 ClipStore 负责（自己的广播绕回自己 -> 丢弃；重复到达 -> 丢弃）。

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::net::TcpListener;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use uuid::Uuid;

use clipstack_protocol::{
    ClipKind, ClipboardItem, ClipStore, IngestOutcome, SyncEnvelope, NONCE_LEN,
};
use crate::crypto;
use crate::db::{self, DbState};
use crate::models::{ContentType, HistoryItem};

pub const SERVICE_TYPE: &str = "_clipstack._tcp.local.";
pub const LAN_PORT: u16 = 8787;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 局域网共享配置（内存态；落库经内部密钥包装为 wrapped_key，见 L3）。
#[derive(Clone)]
pub struct LanConfig {
    pub device_id: String,
    pub device_name: String,
    pub share_group: String,
    /// 明文共享密钥（仅内存）。L3 落库时经内部密钥包装为 wrapped_key，不裸存。
    pub share_key: String,
    pub share_out: bool,
    pub file_limit_mb: u64,
    pub manual_peers: Vec<String>,
    /// 本机 WebSocket 监听端口。默认 LAN_PORT；当该端口被占用（冲突）时，
    /// 用户可在设置中修改为其它端口（如 8790）以规避，并持久化到 settings 表。
    pub listen_port: u16,
}

impl Default for LanConfig {
    fn default() -> Self {
        Self {
            device_id: Uuid::new_v4().to_string(),
            device_name: default_device_name(),
            // 默认未配置共享：组/密钥为空且 share_out 关闭，避免误广播给同子网陌生人。
            share_group: String::new(),
            share_key: String::new(),
            share_out: false,
            file_limit_mb: 100,
            manual_peers: Vec::new(),
            listen_port: LAN_PORT,
        }
    }
}

/// 跨平台获取系统机器名（hostname）：
/// - Windows: %COMPUTERNAME%
/// - 其它（macOS / Linux 等）：优先 $HOSTNAME，其次 /etc/hostname，最后回退 `hostname` 命令
/// 任一环节均失败则回退 "ClipStack"。仅作默认设备名，用户仍可在设置中手动覆盖。
fn default_device_name() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|_| "ClipStack".into())
    }
    #[cfg(not(windows))]
    {
        if let Ok(h) = std::env::var("HOSTNAME") {
            let h = h.trim().to_string();
            if !h.is_empty() {
                return h;
            }
        }
        if let Ok(s) = std::fs::read_to_string("/etc/hostname") {
            let h = s.trim().to_string();
            if !h.is_empty() {
                return h;
            }
        }
        if let Ok(out) = std::process::Command::new("hostname").output() {
            if out.status.success() {
                let h = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !h.is_empty() {
                    return h;
                }
            }
        }
        "ClipStack".into()
    }
}

/// 组内在线设备（对前端）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    pub device_id: String,
    pub name: String,
    pub addr: String,
    pub connected: bool,
}

/// 收到共享条目事件载荷。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedClipPayload {
    pub sync_id: String,
    pub origin_device: String,
    pub kind: String,
}

/// 一条已建立的对端连接句柄：持有写端 mpsc，用于向该对端推送信封。
struct ConnHandle {
    device_id: String,
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

/// manager 内部可变状态。
struct Inner {
    config: LanConfig,
    psk: crypto::Key,
    store: ClipStore,
    /// 已建立连接（按 device_id）。
    conns: HashMap<String, ConnHandle>,
    /// 正在进行的客户端连接任务停止标志（按对端 device_id）。
    client_stops: HashMap<String, Arc<AtomicBool>>,
    /// 已知的组内对端（device_id -> 最近地址），用于断线重连判定。
    known_peers: HashMap<String, SocketAddr>,
    mdns: Option<ServiceDaemon>,
    /// 数据库句柄（写入对端共享条目）。
    db: DbState,
}

/// 局域网同步管理器（Tauri 托管状态）。
#[derive(Clone)]
pub struct LanManager {
    inner: Arc<Mutex<Inner>>,
    app: AppHandle,
}

impl LanManager {
    pub fn new(app: AppHandle, db: DbState) -> Self {
        let cfg = LanConfig::default();
        let psk = crypto::derive_psk(&cfg.share_group, &cfg.share_key);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                config: cfg,
                psk,
                store: ClipStore::new(""), // device_id 在 start() 后设置
                conns: HashMap::new(),
                client_stops: HashMap::new(),
                known_peers: HashMap::new(),
                mdns: None,
                db,
            })),
            app,
        }
    }

    /// 启动：注册 mDNS + 监听 + 浏览。幂等（重复调用安全）。
    pub async fn start(&self) {
        let mut inner = self.inner.lock().await;
        // 设置回环检测用的本机 device_id。
        let dev_id = inner.config.device_id.clone();
        inner.store.set_self_device(dev_id);
        if inner.mdns.is_some() {
            return; // 已在运行
        }
        let mdns = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[lan] mDNS 守护创建失败: {e}");
                return;
            }
        };

        // 1) 注册本机服务实例。
        let fp = crypto::group_fingerprint(&inner.config.share_group, &inner.config.share_key);
        let instance = format!("{}.{}", sanitize_name(&inner.config.device_name), SERVICE_TYPE);
        let host = format!("{}.local.", sanitize_name(&inner.config.device_name));
        let local_ip = local_ipv4();
        let ip_arg: IpAddr = local_ip.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        let mut props: HashMap<String, String> = HashMap::new();
        props.insert("device_id".into(), inner.config.device_id.clone());
        props.insert("name".into(), inner.config.device_name.clone());
        props.insert("version".into(), APP_VERSION.into());
        props.insert("group_fp".into(), fp);
        let info = match ServiceInfo::new(SERVICE_TYPE, &instance, &host, ip_arg, LAN_PORT, props) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("[lan] 服务实例创建失败: {e}");
                return;
            }
        };
        if let Err(e) = mdns.register(info) {
            eprintln!("[lan] mDNS 注册失败: {e}");
            return;
        }

        // 2) 浏览同服务。
        let browse_rx = match mdns.browse(SERVICE_TYPE) {
            Ok(rx) => rx,
            Err(e) => {
                eprintln!("[lan] mDNS 浏览失败: {e}");
                return;
            }
        };
        inner.mdns = Some(mdns);

        // 3) mDNS 事件循环。
        let inner_c = self.inner.clone();
        let app_c = self.app.clone();
        tauri::async_runtime::spawn(async move {
            while let Ok(event) = browse_rx.recv() {
                handle_mdns_event(event, &inner_c, &app_c).await;
            }
            // 通道关闭 -> 守护已停
        });

        // 4) TCP 监听（始终作为服务端，供较小 device_id 的对端连入）。
        let inner_l = self.inner.clone();
        let app_l = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let listener = match TcpListener::bind(("0.0.0.0", LAN_PORT)).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[lan] 监听 {LAN_PORT} 失败: {e}");
                    return;
                }
            };
            loop {
                match listener.accept().await {
                    Ok((socket, _)) => {
                        let inner_cc = inner_l.clone();
                        let app_cc = app_l.clone();
                        tauri::async_runtime::spawn(async move {
                            match tokio_tungstenite::accept_async(socket).await {
                                Ok(ws) => {
                                    if let Err(e) =
                                        accept_peer(ws, &inner_cc, &app_cc, None).await
                                    {
                                        eprintln!("[lan] 接受对端连接失败: {e}");
                                    }
                                }
                                Err(e) => eprintln!("[lan] accept_async 失败: {e}"),
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("[lan] accept 失败: {e}");
                        break;
                    }
                }
            }
        });

        // 5) 手动 peer：每个地址起一个客户端连接任务。
        for addr in inner.config.manual_peers.clone() {
            if let Ok(sa) = addr.parse::<SocketAddr>() {
                spawn_client_retry(sa, &self.inner, &self.app, None).await;
            }
        }
    }

    /// 更新配置并重启发现（组/密钥变更需重新注册 mDNS 指纹）。
    pub async fn set_config(&self, cfg: LanConfig) {
        {
            let mut inner = self.inner.lock().await;
            inner.config = cfg.clone();
            inner.psk = crypto::derive_psk(&cfg.share_group, &cfg.share_key);
            inner.store.set_self_device(cfg.device_id.clone());
        }
        // 停掉旧的 mDNS，重新 start。
        self.stop().await;
        self.start().await;
    }

    /// 停止发现与所有连接。
    pub async fn stop(&self) {
        let mut inner = self.inner.lock().await;
        if let Some(mdns) = inner.mdns.take() {
            let _ = mdns.shutdown();
        }
        for stop in inner.client_stops.values() {
            stop.store(true, Ordering::SeqCst);
        }
        inner.client_stops.clear();
        inner.conns.clear();
        inner.known_peers.clear();
    }

    /// 广播一条剪贴板条目给所有已连对端（L3 由监控线程调用；L2 由测试命令调用）。
    /// 返回推送到的对端数量。
    pub async fn broadcast_clip(&self, item: ClipboardItem) -> usize {
        let mut inner = self.inner.lock().await;
        // 复制配置与密钥，避免与 `store` 的可变借用冲突。
        let cfg = inner.config.clone();
        let psk = inner.psk.clone();
        let env = match build_envelope(&cfg, &psk, &mut inner.store, item) {
            Some(e) => e,
            None => return 0,
        };
        let bytes = match env.to_bytes() {
            Ok(b) => b,
            Err(_) => return 0,
        };
        let mut count = 0;
        // 复制发送端，避免持有锁期间 await。
        let handles: Vec<(String, mpsc::UnboundedSender<Vec<u8>>)> = inner
            .conns
            .iter()
            .map(|(id, h)| (id.clone(), h.tx.clone()))
            .collect();
        drop(inner);
        for (_, tx) in handles {
            if tx.send(bytes.clone()).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// 当前在线设备列表。
    pub async fn peers(&self) -> Vec<PeerInfo> {
        let inner = self.inner.lock().await;
        inner
            .conns
            .values()
            .map(|h| PeerInfo {
                device_id: h.device_id.clone(),
                name: h.device_id.clone(), // L3 用已知名填充
                addr: String::new(),
                connected: true,
            })
            .collect()
    }

    /// L3 · 切换发布开关（不影响 mDNS 指纹，无需重启发现）。
    pub async fn set_share_out(&self, enabled: bool) {
        self.inner.lock().await.config.share_out = enabled;
    }

    pub async fn config(&self) -> LanConfig {
        self.inner.lock().await.config.clone()
    }

    /// L3 · 本地捕获后广播（由监控线程调用）。仅当 `share_out` 开启时广播；
    /// 未配置共享（组/密钥空）时 `share_out` 恒为关闭，故不会误广播。
    /// 返回推送到的对端数。
    pub async fn broadcast_local(&self, content_type: &str, content_text: &str) -> usize {
        let share_out = self.inner.lock().await.config.share_out;
        if !share_out {
            return 0;
        }
        let kind = match content_type {
            "link" => ClipKind::Link,
            "code" => ClipKind::Code,
            "image" => ClipKind::Image,
            "file" => ClipKind::File,
            _ => ClipKind::Text,
        };
        let item = ClipboardItem {
            sync_id: Uuid::new_v4().to_string(),
            device_id: String::new(), // 由 broadcast_clip 时覆盖为本地 device_id
            lamport: 0,
            kind,
            hash: ClipboardItem::content_hash(content_text.as_bytes()),
            payload: content_text.as_bytes().to_vec(),
        };
        self.broadcast_clip(item).await
    }
}

// ===== 内部函数 =====

/// 处理 mDNS 浏览事件：解析对端、按 device_id 决定角色、发起或忽略。
async fn handle_mdns_event(event: ServiceEvent, inner: &Arc<Mutex<Inner>>, app: &AppHandle) {
    match event {
        ServiceEvent::ServiceResolved(info) => {
            let device_id = match info.get_property_val_str("device_id") {
                Some(d) => d.to_string(),
                None => return,
            };
            let fp = info.get_property_val_str("group_fp").unwrap_or("").to_string();
            // 分组指纹不一致 -> 跳过（不同组 / 不同密钥）。
            {
                let g = inner.lock().await;
                if fp != crypto::group_fingerprint(&g.config.share_group, &g.config.share_key) {
                    return;
                }
            }
            // 取对端地址（优先非回环；无则取首个）。
            let addr = {
                let addrs: Vec<SocketAddr> = info
                    .get_addresses()
                    .iter()
                    .map(|s| SocketAddr::new(s.to_ip_addr(), info.get_port()))
                    .collect();
                addrs
                    .iter()
                    .find(|a| !a.ip().is_loopback())
                    .or_else(|| addrs.first())
                    .copied()
            };
            let addr = match addr {
                Some(a) => a,
                None => return,
            };
            // 角色判定：本机 device_id 较大 -> 我是客户端，向对端发起；否则我监听，等对方连。
            let my_id = inner.lock().await.config.device_id.clone();
            if device_id >= my_id {
                return; // 我较小或相等 -> 仅监听
            }
            // 记录已知对端并发起客户端连接（带重连）。
            {
                let mut g = inner.lock().await;
                g.known_peers.insert(device_id.clone(), addr);
            }
            spawn_client_retry(addr, inner, app, Some(device_id)).await;
        }
        ServiceEvent::ServiceRemoved(fullname, _) => {
            // 找不到 device_id（fullname 是实例名），按已知对端名匹配停止。
            let mut g = inner.lock().await;
            let target = g
                .known_peers
                .iter()
                .find(|(id, _)| fullname.contains(id.as_str()))
                .map(|(id, _)| id.clone());
            if let Some(id) = target {
                if let Some(stop) = g.client_stops.remove(&id) {
                    stop.store(true, Ordering::SeqCst);
                }
                g.known_peers.remove(&id);
                g.conns.remove(&id);
                let _ = app.emit("lan-peer-offline", PeerInfo {
                    device_id: id,
                    name: String::new(),
                    addr: String::new(),
                    connected: false,
                });
            }
        }
        _ => {}
    }
}

/// 发起客户端连接，断线指数退避重连，直到停止标志置位或对端移除。
async fn spawn_client_retry(
    addr: SocketAddr,
    inner: &Arc<Mutex<Inner>>,
    app: &AppHandle,
    peer_id: Option<String>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    if let Some(id) = &peer_id {
        inner.lock().await.client_stops.insert(id.clone(), stop.clone());
    }
    let inner = inner.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut backoff = 1u64;
        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let url = format!("ws://{addr}");
            match tokio_tungstenite::connect_async(url).await {
                Ok((ws, _)) => {
                    backoff = 1;
                    if let Err(e) =
                        accept_peer(ws, &inner, &app, peer_id.clone()).await
                    {
                        eprintln!("[lan] 客户端连接处理失败: {e}");
                    }
                    // 连接关闭 -> 重连（除非被要求停止）。
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(30);
                }
                Err(_) => {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(30);
                }
            }
        }
    });
}

/// 接受/处理一条已建立的 WebSocket 连接（服务端 accept 或客户端 connect 后都走这里）。
/// `peer_id` 为客户端侧已知的对端 id（服务端侧需等首条信封获知）。
/// 泛型 `S` 兼容服务端 `TcpStream` 与客户端 `MaybeTlsStream<TcpStream>` 两种底层流。
async fn accept_peer<S>(
    ws: WebSocketStream<S>,
    inner: &Arc<Mutex<Inner>>,
    app: &AppHandle,
    peer_id: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut write, mut read) = ws.split();

    // 建一个 mpsc，写任务从它取信封字节发往对端。
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let write_task = tauri::async_runtime::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if write.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
    });

    // 读循环：解信封 -> 解密 -> 入库(ClipStore) -> 发事件。
    let inner_r = inner.clone();
    let app_r = app.clone();
    let tx_r = tx.clone();
    let read_task = tauri::async_runtime::spawn(async move {
        let mut learned_id: Option<String> = peer_id.clone();
        while let Some(msg) = read.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(_) => break,
            };
            match msg {
                Message::Binary(bytes) => {
                    let env = match SyncEnvelope::from_bytes(&bytes) {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let rid = env.device_id.clone();
                    // 首个信封获知对端 id（服务端侧）。
                    if learned_id.is_none() {
                        learned_id = Some(rid.clone());
                        register_conn(&inner_r, &app_r, &rid, tx_r.clone()).await;
                    }
                    // 解密 + 入库。
                    let outcome = {
                        let mut g = inner_r.lock().await;
                        let psk = g.psk.clone();
                        let decrypt = |e: &SyncEnvelope| -> Option<Vec<u8>> {
                            let mut sealed = e.nonce.clone();
                            sealed.extend_from_slice(&e.ciphertext);
                            crypto::decrypt(&psk, &sealed)
                        };
                        g.store.ingest(&env, decrypt)
                    };
                    match outcome {
                        Ok(IngestOutcome::Stored(r)) => {
                            // 落库（is_remote=1）+ 通知前端/托盘刷新。
                            let ct_str = kind_name(r.item.kind);
                            let text = String::from_utf8_lossy(&r.item.payload).to_string();
                            let sync_id = r.item.sync_id.clone();
                            let origin = r.item.device_id.clone();
                            let lamport = r.item.lamport as i64;
                            let hash = r.item.hash.clone();
                            {
                                let db = inner_r.lock().await.db.clone();
                                let conn = db.conn.lock().unwrap();
                                let key = db.key.lock().unwrap();
                                match db::insert_remote_clip(
                                    &conn,
                                    db::RemoteClipInput {
                                        key: key.as_ref(),
                                        content_type: &ct_str,
                                        content_text: &text,
                                        content_blob: None,
                                        source_app: &origin,
                                        size_bytes: text.len() as i64,
                                        hash: &hash,
                                        is_sensitive: false,
                                        origin_device: &origin,
                                        sync_id: &sync_id,
                                        lamport,
                                        profile_id: "",
                                    },
                                ) {
                                    Ok(Some(rid)) => {
                                        // 新条目：触发历史刷新（托盘/前端均监听此事件）。
                                        // 用真实插入行 id，保证前端 prepend 后该条目可被选中 / 详情查看。
                                        let hi = HistoryItem {
                                            id: rid,
                                            content_type: content_type_from_str(&ct_str),
                                            content_text: text.clone(),
                                            preview: text.clone(),
                                            source_app: origin.clone(),
                                            size_bytes: text.len() as i64,
                                            hash: hash.clone(),
                                            is_pinned: false,
                                            is_favorite: false,
                                            is_sensitive: false,
                                            created_at: db::now_ms(),
                                            origin_device: origin.clone(),
                                            is_remote: true,
                                            deleted_at: None,
                                        };
                                        let _ = app_r.emit("clipboard-changed", hi);
                                    }
                                    Ok(None) => {} // 已存在（去重），不刷新
                                    Err(e) => eprintln!("[lan] 写入对端条目失败: {e}"),
                                }
                            }
                            let _ = app_r.emit(
                                "lan-clipboard-received",
                                ReceivedClipPayload {
                                    sync_id,
                                    origin_device: origin,
                                    kind: ct_str.to_string(),
                                },
                            );
                        }
                        Ok(_) => {} // Duplicate / Loopback -> 忽略
                        Err(e) => eprintln!("[lan] 信封处理失败: {e}"),
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        // 连接关闭：移除并通知。
        if let Some(id) = learned_id {
            remove_conn(&inner_r, &app_r, &id).await;
        }
    });

    // 等读任务结束即视为连接终止；取消写任务。
    let _ = read_task.await;
    write_task.abort();
    Ok(())
}

/// 把连接句柄登记进 manager，并广播在线事件。
async fn register_conn(
    inner: &Arc<Mutex<Inner>>,
    app: &AppHandle,
    device_id: &str,
    tx: mpsc::UnboundedSender<Vec<u8>>,
) {
    let name = {
        let mut g = inner.lock().await;
        let name = g
            .known_peers
            .get(device_id)
            .map(|_| device_id.to_string())
            .unwrap_or_else(|| device_id.to_string());
        g.conns.insert(
            device_id.to_string(),
            ConnHandle {
                device_id: device_id.to_string(),
                tx,
            },
        );
        name
    };
    let _ = app.emit(
        "lan-peer-online",
        PeerInfo {
            device_id: device_id.to_string(),
            name,
            addr: String::new(),
            connected: true,
        },
    );
}

/// 移除连接并广播离线事件。
async fn remove_conn(inner: &Arc<Mutex<Inner>>, app: &AppHandle, device_id: &str) {
    let existed = {
        let mut g = inner.lock().await;
        g.conns.remove(device_id).is_some()
    };
    if existed {
        let _ = app.emit(
            "lan-peer-offline",
            PeerInfo {
                device_id: device_id.to_string(),
                name: String::new(),
                addr: String::new(),
                connected: false,
            },
        );
    }
}

/// 由明文条目构造加密信封（推进 Lamport 时钟，写入 store 以保持本地排序一致）。
fn build_envelope(
    cfg: &LanConfig,
    psk: &crypto::Key,
    store: &mut ClipStore,
    item: ClipboardItem,
) -> Option<SyncEnvelope> {
    let lamport = store.clock().current() + 1;
    let hash = ClipboardItem::content_hash(&item.payload);
    let plain = serde_json::to_vec(&item).ok()?;
    let mut sealed = crypto::encrypt(psk, &plain); // nonce(12) || ct
    if sealed.len() < NONCE_LEN {
        return None;
    }
    let nonce = sealed[..NONCE_LEN].to_vec();
    let ciphertext = sealed.split_off(NONCE_LEN);
    Some(SyncEnvelope {
        sync_id: item.sync_id,
        device_id: cfg.device_id.clone(),
        lamport,
        kind: item.kind,
        hash,
        nonce,
        ciphertext,
    })
}

/// 取本机默认出接口的 IPv4 地址（用于 mDNS 广播 A 记录，供对端连入）。
/// 通过向公网地址发起 UDP connect（不发包）读取本地出口 IP，跨平台可用。
fn local_ipv4() -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip())
}

/// 设备名清洗为合法 mDNS 主机标签。
fn sanitize_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "clipstack".into()
    } else {
        s
    }
}

fn kind_name(k: ClipKind) -> String {
    match k {
        ClipKind::Text => "text",
        ClipKind::Link => "link",
        ClipKind::Code => "code",
        ClipKind::Image => "image",
        ClipKind::File => "file",
    }
    .to_string()
}

/// 信封类型字符串 -> 模型 `ContentType`（用于构造落库 HistoryItem）。
fn content_type_from_str(s: &str) -> ContentType {
    match s {
        "link" => ContentType::Link,
        "code" => ContentType::Code,
        "image" => ContentType::Image,
        "file" => ContentType::File,
        _ => ContentType::Text,
    }
}

/// 由文本构造一条待广播的 ClipboardItem（测试 / L3 监控钩子复用）。
pub fn text_item(text: &str) -> ClipboardItem {
    let payload = text.as_bytes().to_vec();
    let hash = ClipboardItem::content_hash(&payload);
    ClipboardItem {
        sync_id: Uuid::new_v4().to_string(),
        device_id: String::new(), // 由 broadcast 时覆盖为本地
        lamport: 0,
        kind: ClipKind::Text,
        hash,
        payload,
    }
}
