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
use base64::{engine::general_purpose::STANDARD, Engine as _};
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
pub const LAN_PORT: u16 = 21995;
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

/// 将明文共享密钥用内部数据库密钥包装为可落库字符串（不裸存）。
/// 内部密钥不可用时回退空串（明文密钥一并丢失，与「未设置密钥」等价）。
fn wrap_lan_key(db: &DbState, plain: &str) -> String {
    let guard = db.key.lock().expect("key lock poisoned");
    match guard.as_ref() {
        Some(k) => STANDARD.encode(crypto::encrypt(k, plain.as_bytes())),
        None => String::new(),
    }
}

/// 还原包装的共享密钥为明文；包装串为空或解密失败均回退空串。
fn unwrap_lan_key(db: &DbState, wrapped: &str) -> String {
    let sealed = match STANDARD.decode(wrapped) {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    let guard = db.key.lock().expect("key lock poisoned");
    match guard.as_ref() {
        Some(k) => crypto::decrypt(k, &sealed)
            .and_then(|p| String::from_utf8(p).ok())
            .unwrap_or_default(),
        None => String::new(),
    }
}

/// 持久化全部局域网共享配置（密钥经包装后落库）。
fn persist_config(db: &DbState, cfg: &LanConfig) {
    let conn = db.lock();
    let _ = db::update_setting(&conn, "lan_share_group", &cfg.share_group);
    let _ = db::update_setting(&conn, "lan_share_key", &wrap_lan_key(db, &cfg.share_key));
    let _ = db::update_setting(&conn, "lan_device_name", &cfg.device_name);
    let _ = db::update_setting(&conn, "lan_share_out", if cfg.share_out { "1" } else { "0" });
    let _ = db::update_setting(&conn, "lan_file_limit_mb", &cfg.file_limit_mb.to_string());
    let peers_json = serde_json::to_string(&cfg.manual_peers).unwrap_or_else(|_| "[]".into());
    let _ = db::update_setting(&conn, "lan_manual_peers", &peers_json);
    let _ = db::update_setting(&conn, "lan_listen_port", &cfg.listen_port.to_string());
}

/// 从设置表载入已保存的局域网共享配置；无记录或解密失败时保留传入默认值。
fn load_persisted_config(db: &DbState, cfg: &mut LanConfig) {
    let conn = db.lock();
    cfg.share_group = db::get_string_setting(&conn, "lan_share_group", "");
    let wrapped = db::get_string_setting(&conn, "lan_share_key", "");
    if !wrapped.is_empty() {
        cfg.share_key = unwrap_lan_key(db, &wrapped);
    }
    let name = db::get_string_setting(&conn, "lan_device_name", "");
    if !name.is_empty() {
        cfg.device_name = name;
    }
    cfg.share_out = db::get_string_setting(&conn, "lan_share_out", "0") == "1";
    let fl = db::get_int_setting(&conn, "lan_file_limit_mb", 100);
    if fl > 0 {
        cfg.file_limit_mb = fl as u64;
    }
    if let Ok(peers) = serde_json::from_str::<Vec<String>>(
        &db::get_string_setting(&conn, "lan_manual_peers", "[]"),
    ) {
        cfg.manual_peers = peers;
    }
    let saved = db::get_int_setting(&conn, "lan_listen_port", LAN_PORT as i64) as u16;
    if (1..=65535).contains(&saved) {
        cfg.listen_port = saved;
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

/// 本机监听端口被占用事件载荷（前端据此给出明确的换端口提示）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortInUsePayload {
    pub port: u16,
}

/// 一条已建立的对端连接句柄：持有写端 mpsc，用于向该对端推送信封。
struct ConnHandle {
    device_id: String,
    tx: mpsc::UnboundedSender<Message>,
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
    /// 已知对端名称（device_id -> 友好名），来源 mDNS TXT 或握手 hello，用于共享列表展示。
    peer_names: HashMap<String, String>,
    mdns: Option<ServiceDaemon>,
    /// 数据库句柄（写入对端共享条目）。
    db: DbState,
    /// TCP 监听任务句柄：stop() 时中止以释放端口，避免重绑失败（见 set_config 重启）。
    server_task: Option<tauri::async_runtime::JoinHandle<()>>,
}

/// 局域网同步管理器（Tauri 托管状态）。
#[derive(Clone)]
pub struct LanManager {
    inner: Arc<Mutex<Inner>>,
    app: AppHandle,
}

impl LanManager {
    pub fn new(app: AppHandle, db: DbState) -> Self {
        let mut cfg = LanConfig::default();
        // 载入持久化的全部局域网共享配置（组/密钥/设备名/发布开关/文件上限/手动对端/端口）。
        load_persisted_config(&db, &mut cfg);
        let psk = crypto::derive_psk(&cfg.share_group, &cfg.share_key);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                config: cfg,
                psk,
                store: ClipStore::new(""), // device_id 在 start() 后设置
                conns: HashMap::new(),
                client_stops: HashMap::new(),
                known_peers: HashMap::new(),
                peer_names: HashMap::new(),
                mdns: None,
                db,
                server_task: None,
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
        // 注意：`ServiceInfo::new` 的实例名参数只需「纯实例标签」（如 "my-pc"），
        // 库内部会自行拼接为 `实例名.服务类型`（见 mdns-sd service_info.rs: fullname = "{name}.{ty_domain}"）。
        // 切勿把完整服务名（含 "_clipstack._tcp.local." 后缀）当作实例名传入，
        // 否则会生成 "my-pc._clipstack._tcp.local.._clipstack._tcp.local." 这样的畸形全名，
        // 永远匹配不上 `browse("_clipstack._tcp.local.")`，导致两端互相发现失败。
        let fp = crypto::group_fingerprint(&inner.config.share_group, &inner.config.share_key);
        let instance = lan_instance_name(&inner.config.device_name);
        let host = format!("{}.local.", sanitize_name(&inner.config.device_name));
        let local_ip = local_ipv4();
        let ip_arg: IpAddr = local_ip.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        let mut props: HashMap<String, String> = HashMap::new();
        props.insert("device_id".into(), inner.config.device_id.clone());
        props.insert("name".into(), inner.config.device_name.clone());
        props.insert("version".into(), APP_VERSION.into());
        props.insert("group_fp".into(), fp);
        let info = match ServiceInfo::new(SERVICE_TYPE, &instance, &host, ip_arg, inner.config.listen_port, props) {
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
        let listen_port = inner.config.listen_port;
        let inner_l = self.inner.clone();
        let app_l = self.app.clone();
        let server_handle = tauri::async_runtime::spawn(async move {
            let listener = match TcpListener::bind(("0.0.0.0", listen_port)).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[lan] 监听 {listen_port} 失败（端口可能被占用）: {e}");
                    // 明确通知前端：端口被占用，引导用户到设置中更换端口。
                    let _ = app_l.emit(
                        "lan-port-in-use",
                        PortInUsePayload { port: listen_port },
                    );
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
        // 记录监听任务句柄，stop() 时中止以释放端口（否则下次 start() 重绑会误报占用）。
        inner.server_task = Some(server_handle);

        // 5) 手动 peer：每个地址起一个客户端连接任务。省略端口时自动补 LAN_PORT。
        for addr in inner.config.manual_peers.clone() {
            let resolved = addr
                .parse::<SocketAddr>()
                .or_else(|_| format!("{addr}:{LAN_PORT}").parse::<SocketAddr>());
            match resolved {
                Ok(sa) => spawn_client_retry(sa, &self.inner, &self.app, None).await,
                Err(_) => eprintln!("[lan] 忽略无法解析的手动对端地址: {addr}"),
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
            // 持久化全部配置（含密钥包装），重启后仍生效。
            persist_config(&inner.db, &cfg);
        }
        // 停掉旧的 mDNS，重新 start。
        self.stop().await;
        self.start().await;
    }

    /// 停止发现与所有连接。
    pub async fn stop(&self) {
        let server_handle = {
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
            inner.peer_names.clear();
            // 取出监听任务句柄，在释放锁后再中止（避免持锁 await）。
            inner.server_task.take()
        };
        // 中止 TCP 监听任务并等待其退出，确保监听端口被释放，
        // 否则 set_config 重启时发现（start）会因旧监听仍占用端口而误报「端口被占用」。
        if let Some(h) = server_handle {
            h.abort();
            let _ = h.await;
        }
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
        let handles: Vec<(String, mpsc::UnboundedSender<Message>)> = inner
            .conns
            .iter()
            .map(|(id, h)| (id.clone(), h.tx.clone()))
            .collect();
        drop(inner);
        for (_, tx) in handles {
            if tx.send(Message::Binary(bytes.clone().into())).is_ok() {
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
                name: inner
                    .peer_names
                    .get(&h.device_id)
                    .cloned()
                    .unwrap_or_else(|| h.device_id.clone()),
                addr: String::new(),
                connected: true,
            })
            .collect()
    }

    /// L3 · 切换发布开关（不影响 mDNS 指纹，无需重启发现）。
    pub async fn set_share_out(&self, enabled: bool) {
        let db = {
            let mut inner = self.inner.lock().await;
            inner.config.share_out = enabled;
            inner.db.clone()
        };
        // 仅发布开关变更也要持久化，否则重启后回退。
        let cfg = self.config().await;
        persist_config(&db, &cfg);
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
            let name = info.get_property_val_str("name").unwrap_or("").to_string();
            // 分组指纹不一致 -> 跳过（不同组 / 不同密钥）。
            {
                let mut g = inner.lock().await;
                if fp != crypto::group_fingerprint(&g.config.share_group, &g.config.share_key) {
                    return;
                }
                // 记录对端友好名（供共享列表展示），优先于握手 hello。
                g.peer_names.insert(device_id.clone(), name.clone());
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
                g.peer_names.remove(&id);
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

    // 建一个 mpsc，写任务从它取消息（信封二进制 / 握手文本）发往对端。
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let write_task = tauri::async_runtime::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if write.send(msg).await.is_err() {
                break;
            }
        }
    });

    // 读循环：解信封 -> 解密 -> 入库(ClipStore) -> 发事件。
    let inner_r = inner.clone();
    let app_r = app.clone();
    let tx_r = tx.clone();

    // 连接建立即处理「在线」：
    // - 客户端侧已知对端 device_id（来自 mDNS），立即登记，无需等首条剪贴板信封；
    // - 双方各发一条 hello 握手（携带本端 device_id + 名称），使服务端侧也能立即获知对端并登记。
    // 这样「共享列表」在连接建立后即可显示对端，而非要等某次复制同步。
    let (my_id, my_name) = {
        let g = inner.lock().await;
        (g.config.device_id.clone(), g.config.device_name.clone())
    };
    if let Some(id) = &peer_id {
        register_conn(&inner, &app, id, tx_r.clone()).await;
    }
    let hello = serde_json::json!({
        "type": "hello",
        "device_id": my_id,
        "name": my_name,
    })
    .to_string();
    let _ = tx.send(Message::Text(hello.into()));
    let read_task = tauri::async_runtime::spawn(async move {
        let mut learned_id: Option<String> = peer_id.clone();
        while let Some(msg) = read.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(_) => break,
            };
            match msg {
                Message::Text(s) => {
                    // 握手：对端告知其 device_id + 名称，使本端（服务端侧）立即登记「在线」，
                    // 而无需等待首条剪贴板信封。客户端侧 learned_id 已由 mDNS 预置，此处仅更新名称。
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                        if v.get("type").and_then(|t| t.as_str()) == Some("hello") {
                            if let Some(rid) = v.get("device_id").and_then(|d| d.as_str()) {
                                let nm = v
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if learned_id.is_none() {
                                    learned_id = Some(rid.to_string());
                                    register_conn(&inner_r, &app_r, rid, tx_r.clone()).await;
                                } else {
                                    update_peer_name(&inner_r, &app_r, rid, &nm).await;
                                }
                            }
                        }
                    }
                }
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
/// 仅在对端「新上线」时广播 `lan-peer-online`，避免握手 hello 与首条信封重复触发。
/// 名称优先取 `peer_names`（mDNS TXT / hello 提供），缺失时回退 device_id。
async fn register_conn(
    inner: &Arc<Mutex<Inner>>,
    app: &AppHandle,
    device_id: &str,
    tx: mpsc::UnboundedSender<Message>,
) {
    let (is_new, resolved) = {
        let mut g = inner.lock().await;
        let resolved = g
            .peer_names
            .get(device_id)
            .cloned()
            .unwrap_or_else(|| device_id.to_string());
        let is_new = !g.conns.contains_key(device_id);
        g.conns.insert(
            device_id.to_string(),
            ConnHandle {
                device_id: device_id.to_string(),
                tx,
            },
        );
        if !g.peer_names.contains_key(device_id) {
            g.peer_names.insert(device_id.to_string(), resolved.clone());
        }
        (is_new, resolved)
    };
    if is_new {
        let _ = app.emit(
            "lan-peer-online",
            PeerInfo {
                device_id: device_id.to_string(),
                name: resolved,
                addr: String::new(),
                connected: true,
            },
        );
    }
}

/// 更新已知对端名称（来自握手 hello / mDNS 刷新）；名称变化才重新广播在线事件。
async fn update_peer_name(
    inner: &Arc<Mutex<Inner>>,
    app: &AppHandle,
    device_id: &str,
    name: &str,
) {
    let changed = {
        let mut g = inner.lock().await;
        match g.peer_names.get(device_id) {
            Some(existing) if existing == name => false,
            _ => {
                g.peer_names.insert(device_id.to_string(), name.to_string());
                true
            }
        }
    };
    if changed {
        let _ = app.emit(
            "lan-peer-online",
            PeerInfo {
                device_id: device_id.to_string(),
                name: name.to_string(),
                addr: String::new(),
                connected: true,
            },
        );
    }
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
pub(crate) fn local_ipv4() -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip())
}

/// mDNS 实例标签：仅「纯实例名」，绝不可附带服务类型后缀。
/// `mdns-sd` 的 `ServiceInfo::new` 会自动拼接为 `实例名.服务类型`
/// （见 service_info.rs: `fullname = "{name}.{ty_domain}"`），
/// 若这里再带上 `_clipstack._tcp.local.` 会生成畸形全名，使 `browse` 永远匹配不上。
fn lan_instance_name(name: &str) -> String {
    sanitize_name(name)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_device_name_is_nonempty() {
        // 跨平台获取机器名，任一路径（含回退 "ClipStack"）都应返回非空字符串。
        assert!(!default_device_name().is_empty());
    }

    #[test]
    fn default_config_device_name_uses_hostname() {
        // 默认设备名应来自系统机器名而非旧的硬编码占位。
        let name = LanConfig::default().device_name;
        assert!(!name.is_empty());
        assert_ne!(name, "ClipStack");
    }

    #[test]
    fn default_listen_port_is_lan_port() {
        // 默认监听端口应等于常量，供冲突时在设置中修改。
        assert_eq!(LanConfig::default().listen_port, LAN_PORT);
    }

    #[test]
    fn lan_instance_name_has_no_service_type_suffix() {
        // 回归：mDNS 实例标签绝不能包含服务类型后缀，否则 mdns-sd 会拼出
        // `name._clipstack._tcp.local.._clipstack._tcp.local.` 这样的畸形全名，
        // 导致 browse("_clipstack._tcp.local.") 永远匹配不上、两端互相发现失败。
        let label = lan_instance_name("MyMacBook");
        assert_eq!(label, "MyMacBook");
        assert!(!label.contains("_clipstack"));
        assert!(!label.contains('.'));
    }

    #[test]
    fn config_persist_and_reload_roundtrip() {
        // 回归：全部局域网共享参数（含经包装的密钥）应能被持久化并在重启后还原，
        // 否则用户设置会丢失（此前仅 lan_listen_port 被持久化）。
        use crate::crypto::Key;
        use crate::db::AppDb;
        use std::sync::{Arc, Mutex};

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        let db: DbState = Arc::new(AppDb {
            conn: Mutex::new(conn),
            key: Mutex::new(Some(Key([7u8; 32]))),
        });

        let mut cfg = LanConfig::default();
        cfg.share_group = "team-a".into();
        cfg.share_key = "s3cret".into();
        cfg.device_name = "my-pc".into();
        cfg.share_out = true;
        cfg.file_limit_mb = 50;
        cfg.manual_peers = vec!["192.168.1.5:21995".to_string()];
        cfg.listen_port = 21996;
        persist_config(&db, &cfg);

        let mut reloaded = LanConfig::default();
        load_persisted_config(&db, &mut reloaded);

        assert_eq!(reloaded.share_group, "team-a");
        assert_eq!(reloaded.share_key, "s3cret"); // 密钥经包装后仍能还原明文
        assert_eq!(reloaded.device_name, "my-pc");
        assert!(reloaded.share_out);
        assert_eq!(reloaded.file_limit_mb, 50);
        assert_eq!(reloaded.manual_peers, vec!["192.168.1.5:21995".to_string()]);
        assert_eq!(reloaded.listen_port, 21996);
    }
}
