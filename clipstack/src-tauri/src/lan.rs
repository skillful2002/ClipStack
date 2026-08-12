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
use tauri::{AppHandle, Emitter, Manager};
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
    /// 允许共享的内容类型白名单，取值为 "text" | "image" | "file" 的子集，默认三者皆共享。
    pub share_types: Vec<String>,
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
            file_limit_mb: 10,
            manual_peers: Vec::new(),
            share_types: vec!["text".into(), "image".into(), "file".into()],
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
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "ClipStack".into())
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
    let types_json = serde_json::to_string(&cfg.share_types).unwrap_or_else(|_| "[]".into());
    let _ = db::update_setting(&conn, "lan_share_types", &types_json);
    let _ = db::update_setting(&conn, "lan_listen_port", &cfg.listen_port.to_string());
    // 本机设备 ID 必须持久化：它是 mDNS 指纹、对端识别与连接角色判定的稳定依据，
    // 一旦随每次启动随机变化，会导致对端缓存失效、连接/识别类 BUG（详见代码评审）。
    let _ = db::update_setting(&conn, "lan_device_id", &cfg.device_id);
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
    let fl = db::get_int_setting(&conn, "lan_file_limit_mb", 10);
    if fl > 0 {
        cfg.file_limit_mb = fl as u64;
    }
    if let Ok(peers) = serde_json::from_str::<Vec<String>>(
        &db::get_string_setting(&conn, "lan_manual_peers", "[]"),
    ) {
        cfg.manual_peers = peers;
    }
    if let Ok(types) = serde_json::from_str::<Vec<String>>(
        &db::get_string_setting(&conn, "lan_share_types", "[]"),
    ) {
        // 仅保留白名单内的合法类型，避免旧数据/脏数据写入无效值。
        let valid: Vec<String> = types
            .into_iter()
            .filter(|t| matches!(t.as_str(), "text" | "image" | "file"))
            .collect();
        if !valid.is_empty() {
            cfg.share_types = valid;
        }
    }
    let saved = db::get_int_setting(&conn, "lan_listen_port", LAN_PORT as i64) as u16;
    if (1..=65535).contains(&saved) {
        cfg.listen_port = saved;
    }
    // 本机设备 ID：持久化保证跨启动稳定。无记录（首次启动）时生成一次并立即写回，
    // 之后始终复用，避免因 device_id 每次随机变化引发的连接/识别类 BUG。
    let saved_id = db::get_string_setting(&conn, "lan_device_id", "");
    if saved_id.is_empty() {
        cfg.device_id = Uuid::new_v4().to_string();
        let _ = db::update_setting(&conn, "lan_device_id", &cfg.device_id);
    } else {
        cfg.device_id = saved_id;
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
    /// 网络栈代际计数：每次 start()（含 stop+start 重启）递增，stop() 也会递增。连接的读任务在
    /// spawn 时捕获自己的代际；读任务在 register_conn（把对端插入 conns）前检查代际是否仍有效，
    /// 失效则跳过登记——从而避免 stop 清空 conns 后、协作式取消的读任务又把对端插回、污染
    /// 前端 refreshPeers 读到的「在线设备」列表。
    gen: u64,
    /// 每条已建连接读任务的中止句柄（device_id -> AbortHandle）。
    /// 关闭共享时必须主动 abort 这些读任务，才能真正关闭底层 socket（发 FIN），
    /// 否则读任务仍存活、持续回 Pong，对端心跳永不超时，设备永远不从列表移除（僵尸连接）。
    conn_aborts: HashMap<String, tokio::task::AbortHandle>,
    /// 已知对端的 mDNS 全名（device_id -> fullname），用于 ServiceRemoved 精确匹配。
    /// 因 mDNS fullname 取自已方设备名而非 device_id(UUID)，不能靠 contains(device_id) 匹配。
    peer_fullnames: HashMap<String, String>,
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
    /// 共享发布开关的原子镜像：供托盘菜单等同步上下文无锁读取，
    /// 避免在非异步线程 `block_on(config())` 造成嵌套阻塞 / panic。
    share_out_flag: Arc<AtomicBool>,
    app: AppHandle,
}

impl LanManager {
    pub fn new(app: AppHandle, db: DbState) -> Self {
        let mut cfg = LanConfig::default();
        // 载入持久化的全部局域网共享配置（组/密钥/设备名/发布开关/文件上限/手动对端/端口）。
        load_persisted_config(&db, &mut cfg);
        let psk = crypto::derive_psk(&cfg.share_group, &cfg.share_key);
        let share_out = cfg.share_out;
        Self {
            inner: Arc::new(Mutex::new(Inner {
                config: cfg,
                psk,
                store: ClipStore::new(""), // device_id 在 start() 后设置
                conns: HashMap::new(),
                client_stops: HashMap::new(),
                known_peers: HashMap::new(),
                peer_names: HashMap::new(),
                conn_aborts: HashMap::new(),
                gen: 0,
                peer_fullnames: HashMap::new(),
                mdns: None,
                db,
                server_task: None,
            })),
            share_out_flag: Arc::new(AtomicBool::new(share_out)),
            app,
        }
    }

    /// 启动：注册 mDNS + 监听 + 浏览。幂等（重复调用安全）。
    pub async fn start(&self) {
        let mut inner = self.inner.lock().await;
        // 设置回环检测用的本机 device_id。
        let dev_id = inner.config.device_id.clone();
        inner.store.set_self_device(dev_id);
        // 新网络栈代际：使上一轮遗留的连接读任务失效（它们会在 register_conn 前因 gen 不匹配而跳过）。
        inner.gen = inner.gen.wrapping_add(1);
        // 未开启共享：不注册 mDNS / 不监听，避免未共享时被同网发现，
        // 也确保关闭共享后本机从对端「在线设备」列表消失（而非残留 / 重复）。
        if !inner.config.share_out {
            return;
        }
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

    /// 更新配置。
    ///
    /// 仅当真正需要重建网络栈的参数变化时才 `stop()` + `start()`：
    /// 共享组 / 密钥（决定 mDNS 指纹）、本机端口（需重新绑定监听）、共享开关
    /// （关闭要停发现、开启要起发现）。这三类变更频率低且用户有明确预期。
    ///
    /// 文件大小上限、共享类型白名单、本机设备名属于「热参数」：仅就地更新配置与派生
    /// 密钥，**不拆除任何连接、不清空对端名称缓存**。否则每次改动（尤其是文件上限输入框
    /// 的逐字符输入）都会 `stop()` + `start()`，导致：① 正在传输的文件因连接被 tearing
    /// down 而丢失；② `peer_names` 被清空，随后收到的剪贴板来源回退为设备 ID。
    pub async fn set_config(&self, cfg: LanConfig) -> Result<(), String> {
        // 若要开启共享，必须先配置组 / 密钥 / 端口；缺失则返回错误，且不改动任何状态。
        if cfg.share_out {
            let missing = Self::missing_share_prereqs(&cfg);
            if !missing.is_empty() {
                return Err(format!("SHARE_PREREQ_MISSING:{}", missing.join(",")));
            }
        }
        let need_restart = {
            let inner = self.inner.lock().await;
            let old = &inner.config;
            old.share_group != cfg.share_group
                || old.share_key != cfg.share_key
                || old.listen_port != cfg.listen_port
                || old.share_out != cfg.share_out
        };
        let share_changed = {
            let inner = self.inner.lock().await;
            inner.config.share_out != cfg.share_out
        };
        {
            let mut inner = self.inner.lock().await;
            inner.config = cfg.clone();
            inner.psk = crypto::derive_psk(&cfg.share_group, &cfg.share_key);
            inner.store.set_self_device(cfg.device_id.clone());
            // 同步更新原子镜像，供托盘菜单等同步读取（与 set_share_out 保持一致）。
            self.share_out_flag.store(cfg.share_out, Ordering::SeqCst);
            // 持久化全部配置（含密钥包装），重启后仍生效。
            persist_config(&inner.db, &cfg);
        }
        if need_restart {
            // 停掉旧的 mDNS，重新 start。
            self.stop().await;
            self.start().await;
        }
        if share_changed {
            // 共享开关变化：通知前端同步（lan-config-changed）并刷新托盘菜单的圆点 / 状态文字。
            let _ = self.app.emit("lan-config-changed", ());
            let _ = self.app.emit("refresh-tray", ());
        }
        Ok(())
    }

    /// 停止发现与所有连接。
    pub async fn stop(&self) {
        let (server_handle, removed, aborts) = {
            let mut inner = self.inner.lock().await;
            // 收集将被移除的对端，便于在共享关闭时广播离线事件。
            let removed: Vec<String> = inner.conns.keys().cloned().collect();
            // 代际失效：使所有旧连接读任务失效。它们随后在 register_conn 前检查 gen 不匹配，
            // 不再把对端插回 conns，从而消除「stop 清空 conns 后读任务又把设备加回」的竞态
            // （前端 refreshPeers 整体覆盖 peers，读到脏 conns 会让「在线设备」面板残留已断开设备）。
            inner.gen = inner.gen.wrapping_add(1);
            if let Some(mdns) = inner.mdns.take() {
                let _ = mdns.shutdown();
            }
            for stop in inner.client_stops.values() {
                stop.store(true, Ordering::SeqCst);
            }
            inner.client_stops.clear();
            // 收集所有对端连接读任务的中止句柄：释放锁后再 abort，才能真正关闭底层
            // socket（发 FIN），使对端立即检测到断线并 remove_conn，从「在线设备」列表移除本机。
            // 仅靠 drop `tx`（写半边）不够：读任务仍存活、持读半边并自动回 Pong，
            // 连接会变成僵尸，对端心跳永不超时。
            let aborts: Vec<tokio::task::AbortHandle> =
                inner.conn_aborts.values().cloned().collect();
            inner.conns.clear();
            inner.conn_aborts.clear();
            inner.known_peers.clear();
            inner.peer_fullnames.clear();
            // 注意：不清空 `peer_names`。它是「device_id -> 友好名」的发现缓存，
            // 与连接状态无关；保留它可避免配置热更新 / 重启后发现的对端来源回退为设备 ID。
            // 对端重连后会通过 mDNS TXT / 握手 hello 重新刷新名称，旧名称不会造成误显示。
            // 取出监听任务句柄，在释放锁后再中止（避免持锁 await）。
            (inner.server_task.take(), removed, aborts)
        };
        // 中止 TCP 监听任务并等待其退出，确保监听端口被释放，
        // 否则 set_config 重启时发现（start）会因旧监听仍占用端口而误报「端口被占用」。
        if let Some(h) = server_handle {
            h.abort();
            let _ = h.await;
        }
        // 主动中止所有对端连接读任务：底层 socket 关闭（FIN），对端 read.next() 立即返回 None
        // -> remove_conn -> 广播 lan-peer-offline，本机从对方「在线设备」列表消失。
        for a in aborts {
            a.abort();
        }
        // 仅当共享确实关闭时广播离线：配置热更新（共享仍开）会立即重连，
        // 不广播离线可避免「在线设备」列表闪烁。
        if !self.inner.lock().await.config.share_out {
            for id in removed {
                let _ = self.app.emit(
                    "lan-peer-offline",
                    PeerInfo {
                        device_id: id,
                        name: String::new(),
                        addr: String::new(),
                        connected: false,
                    },
                );
            }
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
        // 将整条信封切分为 32KB 的小帧发送：单条超大 WebSocket 消息会以「一整帧」传输，
        // 期间无法插入 Ping/Pong 控制帧，弱网下大文件传输超过心跳宽限会被误判为断线而掐断。
        // 分片后控制帧可在分片之间穿插，连接始终存活，大文件也能完整送达。
        let frames = build_chunk_frames(&bytes);
        let mut count = 0;
        // 复制发送端，避免持有锁期间 await。
        let handles: Vec<(String, mpsc::UnboundedSender<Message>)> = inner
            .conns
            .iter()
            .map(|(id, h)| (id.clone(), h.tx.clone()))
            .collect();
        drop(inner);
        for (_, tx) in handles {
            let mut sent = true;
            for f in &frames {
                if tx.send(f.clone()).is_err() {
                    sent = false;
                    break;
                }
            }
            if sent {
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
    /// 同步读取当前「共享发布」开关状态。
    /// 供托盘菜单重建（build_menu 可能在事件监听线程调用）读取，避免在非异步上下文
    /// 使用 `block_on(config())` 造成潜在嵌套 block_on 风险。
    pub fn is_share_out(&self) -> bool {
        self.share_out_flag.load(Ordering::SeqCst)
    }

    /// 校验「开启共享」的前置条件：共享组、共享密钥、监听端口三者都必须已设置。
    /// 返回缺失参数的内码（`["group","key","port"]` 的子集，空表示通过）。
    /// 端口为 `u16`，默认即 `LAN_PORT`（非零），故通常已满足；仅当用户显式清空为 0 时不通过。
    fn missing_share_prereqs(cfg: &LanConfig) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if cfg.share_group.trim().is_empty() {
            missing.push("group");
        }
        if cfg.share_key.trim().is_empty() {
            missing.push("key");
        }
        if cfg.listen_port == 0 {
            missing.push("port");
        }
        missing
    }

    pub async fn set_share_out(&self, enabled: bool) -> Result<(), String> {
        // 开启共享前校验前置条件：组 / 密钥 / 端口三者缺一不可，否则返回错误提示，
        // 且不改变任何状态（托盘 / 前端据此提示用户先完成配置）。
        if enabled {
            let missing = {
                let inner = self.inner.lock().await;
                Self::missing_share_prereqs(&inner.config)
            };
            if !missing.is_empty() {
                return Err(format!("SHARE_PREREQ_MISSING:{}", missing.join(",")));
            }
        }
        let changed = {
            let mut inner = self.inner.lock().await;
            if inner.config.share_out == enabled {
                // 已是目标状态，无需改动（也避免无谓的 stop/start）。
                false
            } else {
                inner.config.share_out = enabled;
                // 同步更新原子镜像，供托盘菜单等同步读取。
                self.share_out_flag.store(enabled, Ordering::SeqCst);
                // 仅开关变更也要持久化，否则重启后回退。
                persist_config(&inner.db, &inner.config);
                true
            }
        };
        if changed {
            // 重启发现 / 监听，使开关真正生效（与 set_config 中 share_out 变化的路径一致）。
            self.stop().await;
            self.start().await;
            // 通知前端立即同步开关状态（主要服务「托盘切换共享」与「设置页切换」场景）。
            let _ = self.app.emit("lan-config-changed", ());
            // 通知托盘重建菜单，刷新「共享」项的圆点与状态文字（设置页切换后托盘也要更新）。
            let _ = self.app.emit("refresh-tray", ());
        }
        Ok(())
    }

    pub async fn config(&self) -> LanConfig {
        self.inner.lock().await.config.clone()
    }

    /// L3 · 本地捕获后广播（由监控线程调用）。仅当 `share_out` 开启时广播；
    /// 未配置共享（组/密钥空）时 `share_out` 恒为关闭，故不会误广播。
    /// `content_blob` 为本地落库的二进制（图片为 PNG 字节、文件为 JSON 路径数组），
    /// 文本 / 链接 / 代码类为 `None`——据此决定真正的明文负载。
    /// 返回推送到的对端数。
    pub async fn broadcast_local(
        &self,
        content_type: &str,
        content_text: &str,
        source_app: &str,
        content_blob: Option<Vec<u8>>,
    ) -> usize {
        let (share_out, file_limit_mb, share_types) = {
            let inner = self.inner.lock().await;
            (
                inner.config.share_out,
                inner.config.file_limit_mb,
                inner.config.share_types.clone(),
            )
        };
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
        // 按「共享类型」白名单过滤：未勾选的类型（全部取消则不共享任何内容）直接跳过。
        // 文本 / 链接 / 代码统一归入「text」开关；图片归「image」；文件归「file」。
        if !kind_allowed(&share_types, kind) {
            return 0;
        }
        // 真实明文负载：
        // - 图片用 PNG 字节（之前误把「WxH 图片」占位文本当负载，导致对端收不到图片）；
        // - 文件用二进制 bundle（文件名 + 字节），跨端物理传输后落盘到本机 share 目录；
        // - 文本 / 链接 / 代码用原文。
        let (payload, hash) = match kind {
            ClipKind::Image => {
                let b = content_blob.unwrap_or_default();
                let h = ClipboardItem::content_hash(&b);
                (b, h)
            }
            ClipKind::File => {
                let limit = file_limit_mb * 1024 * 1024;
                let entries = build_file_entries(&content_blob, limit);
                let bundle = encode_file_bundle(&entries);
                let h = ClipboardItem::content_hash(&bundle);
                (bundle, h)
            }
            _ => {
                let t = content_text.as_bytes().to_vec();
                let h = ClipboardItem::content_hash(&t);
                (t, h)
            }
        };
        let item = ClipboardItem {
            sync_id: Uuid::new_v4().to_string(),
            device_id: String::new(), // 由 broadcast_clip 时覆盖为本地 device_id
            source_app: source_app.to_string(),
            lamport: 0,
            kind,
            hash,
            payload,
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
                // 记录 mDNS 全名，供 ServiceRemoved 精确匹配（fullname 取自设备名，不含 device_id）。
                g.peer_fullnames
                    .insert(device_id.clone(), info.get_fullname().to_string());
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
            // mDNS 全名取自已方设备名（实例名），不含 device_id(UUID)，无法用 contains 匹配；
            // 改用 ServiceResolved 时记录的 peer_fullnames 做精确匹配。
            let (id, abort, was) = {
                let mut g = inner.lock().await;
                let target = g
                    .peer_fullnames
                    .iter()
                    .find(|(_, fn_)| fn_.as_str() == fullname.as_str())
                    .map(|(id, _)| id.clone());
                match target {
                    Some(id) => {
                        if let Some(stop) = g.client_stops.remove(&id) {
                            stop.store(true, Ordering::SeqCst);
                        }
                        g.known_peers.remove(&id);
                        g.peer_names.remove(&id);
                        g.peer_fullnames.remove(&id);
                        let abort = g.conn_aborts.remove(&id);
                        let was = g.conns.remove(&id).is_some();
                        (Some(id), abort, was)
                    }
                    None => (None, None, false),
                }
            };
            // 主动中止该对端连接读任务，真正关闭 socket（FIN），使对端检测到本端移除；
            // 仅从 conns 移除而不关 socket 会留下僵尸连接（对端仍收 Pong，列表清不掉）。
            if let Some(abort) = abort {
                abort.abort();
            }
            if let (Some(id), true) = (id, was) {
                let _ = app.emit(
                    "lan-peer-offline",
                    PeerInfo {
                        device_id: id,
                        name: String::new(),
                        addr: String::new(),
                        connected: false,
                    },
                );
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
    // 本连接所属网络栈代际：stop()/start() 会递增 gen 使旧连接失效。读任务在 register_conn 前
    // 检查 gen 不匹配则跳过登记，避免把对端重新插回 conns（前端 refreshPeers 读到脏列表会残留设备）。
    let my_gen = { let g = inner.lock().await; g.gen };

    // 连接建立即处理「在线」：
    // - 客户端侧已知对端 device_id（来自 mDNS），立即登记，无需等首条剪贴板信封；
    // - 双方各发一条 hello 握手（携带本端 device_id + 名称），使服务端侧也能立即获知对端并登记。
    // 这样「共享列表」在连接建立后即可显示对端，而非要等某次复制同步。
    let (my_id, my_name) = {
        let g = inner.lock().await;
        (g.config.device_id.clone(), g.config.device_name.clone())
    };
    if let Some(id) = &peer_id {
        register_conn(&inner, &app, id, tx_r.clone(), my_gen).await;
    }
    let hello = serde_json::json!({
        "type": "hello",
        "device_id": my_id,
        "name": my_name,
    })
    .to_string();
    let _ = tx.send(Message::Text(hello.into()));
    // 中止句柄槽：spawn 后取得 read_task 的 abort_handle 写入槽，供读循环在「学到对端 id」时
    // 登记进 Inner.conn_aborts，使 stop() 能主动中止本连接读任务、真正关闭底层 socket（发 FIN），
    // 对端随即检测到断线并 remove_conn。否则仅 drop 写半边会让连接变僵尸、对端心跳永不超时。
    let abort_slot: Arc<std::sync::Mutex<Option<tokio::task::AbortHandle>>> =
        Arc::new(std::sync::Mutex::new(None));
    let slot_for_task = abort_slot.clone();
    let peer_id_for_outside = peer_id.clone();
    let read_task = tokio::task::spawn(async move {
        let abort_slot = slot_for_task;
        let mut learned_id: Option<String> = peer_id.clone();
        // 心跳保活：每 15s 发送一次 Ping，对端自动回 Pong；若 45s 内无任何消息
        // （含 Pong），视为对端已断线（断电 / 断网等静默断开），主动关闭连接，
        // 使「在线设备」列表能及时移除该设备。
        let mut ping = tokio::time::interval(std::time::Duration::from_secs(15));
        let idle = std::time::Duration::from_secs(45);
        // 最近一次「收到任何帧（含对端自动回的 Pong 控制帧）」的时间，用于判断对端存活。
        // 用「连接活跃度」而非「读空闲计时器」：大文件传输时接收方忙于读取整条消息、暂时
        // 无法回应上层消息，但底层仍会自动回 Pong，故不会被误判为断线；真正静默断开
        // （断电 / 断网）时 Pong 停止，约 45s（idle）后下方 ping.tick 触发超时关闭连接。
        let mut last_activity = std::time::Instant::now();
        // 信封分片重组缓冲：发送端按 32KB 分片发送，本端按 START(total) + DATA 累加，
        // 凑齐 total 字节后再反序列化为信封。避免单条超大消息以整帧传输、期间无法穿插
        // 心跳控制帧，导致弱网下大文件传输被误判断线。
        let mut asm_total: Option<usize> = None;
        let mut asm_buf: Vec<u8> = Vec::new();
        loop {
            tokio::select! {
                next = read.next() => {
                    let msg = match next {
                        Some(Ok(m)) => m,
                        _ => break,
                    };
                    // 收到任何帧都视为连接活跃（大文件传输期间底层仍会回 Pong 控制帧）。
                    last_activity = std::time::Instant::now();
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
                                    // 手动对端无 mDNS，名称只能来自 hello：先把友好名写入
                                    // peer_names 再注册，使首次「在线」事件即带机器名称（而非 ID）。
                                    {
                                        let mut g = inner_r.lock().await;
                                        g.peer_names.entry(rid.to_string()).or_insert(nm.clone());
                                    }
                                    register_conn(&inner_r, &app_r, rid, tx_r.clone(), my_gen).await;
                                    // 服务端侧此时才学到对端 id：登记中止句柄，使 stop() 能断开本连接。
                                    let handle_opt = abort_slot.lock().unwrap().clone();
                                    if let Some(h) = handle_opt {
                                        let mut g = inner_r.lock().await;
                                        g.conn_aborts.insert(rid.to_string(), h);
                                    }
                                } else {
                                    update_peer_name(&inner_r, &app_r, rid, &nm).await;
                                }
                            }
                        }
                    }
                }
                Message::Binary(bytes) => {
                    // 信封分片重组：首字节为帧类型标签。START 携带总长度(u32 BE) + 首片；
                    // DATA 携带后续分片；按总长度累加，凑齐后再反序列化为信封。
                    if bytes.is_empty() {
                        continue;
                    }
                    let tag = bytes[0];
                    if tag == FRAME_START {
                        if bytes.len() < 5 {
                            continue;
                        }
                        let total = u32::from_be_bytes([
                            bytes[1], bytes[2], bytes[3], bytes[4],
                        ]) as usize;
                        asm_total = Some(total);
                        asm_buf.clear();
                        asm_buf.extend_from_slice(&bytes[5..]);
                    } else if tag == FRAME_DATA {
                        if asm_total.is_none() {
                            continue;
                        }
                        asm_buf.extend_from_slice(&bytes[1..]);
                    } else {
                        continue;
                    }
                    // 未凑齐完整信封，等待后续分片。
                    let need = asm_total.unwrap_or(0);
                    if asm_buf.len() < need {
                        continue;
                    }
                    let env_bytes = asm_buf[..need].to_vec();
                    asm_buf.clear();
                    asm_total = None;
                    let env = match SyncEnvelope::from_bytes(&env_bytes) {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let rid = env.device_id.clone();
                    // 不接收来自本机自身的共享数据（回环 / 同机多实例互发），也不将自身登记为在线对端。
                    {
                        let g = inner_r.lock().await;
                        if !g.config.device_id.is_empty() && rid == g.config.device_id {
                            continue;
                        }
                    }
                    // 首个信封获知对端 id（服务端侧）。
                    if learned_id.is_none() {
                        learned_id = Some(rid.clone());
                        register_conn(&inner_r, &app_r, &rid, tx_r.clone(), my_gen).await;
                        // 服务端侧此时才学到对端 id：登记中止句柄，使 stop() 能断开本连接。
                        let handle_opt = abort_slot.lock().unwrap().clone();
                        if let Some(h) = handle_opt {
                            let mut g = inner_r.lock().await;
                            g.conn_aborts.insert(rid, h);
                        }
                    }
                    // 按本端「共享类型」白名单过滤：未勾选的类型不入库（在线列表不受影响）。
                    let allowed = {
                        let g = inner_r.lock().await;
                        kind_allowed(&g.config.share_types, env.kind)
                    };
                    if !allowed {
                        continue;
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
                            // 文本 / 链接 / 代码：payload 即原文；图片 / 文件：payload 为二进制，
                            // 落库为 content_blob（图片为 PNG 字节、文件为 JSON 路径数组）。
                            // 之前 image 把乱码 UTF-8 当 content_text、content_blob 恒为 None，
                            // 导致对端 get_item_blob 读到空，前端显示「加载图片失败」。
                            let is_binary = matches!(r.item.kind, ClipKind::Image | ClipKind::File);
                            let (text, blob, size_bytes) = if is_binary {
                                if r.item.kind == ClipKind::Image {
                                    let label = png_dimensions_label(&r.item.payload)
                                        .unwrap_or_else(|| "图片".to_string());
                                    (
                                        label,
                                        Some(r.item.payload.clone()),
                                        r.item.payload.len() as i64,
                                    )
                                } else {
                                    // 文件：按本端文件大小上限过滤，超过上限的单个文件不落盘（参数双向生效）。
                                    let limit_bytes = {
                                        let g = inner_r.lock().await;
                                        g.config.file_limit_mb.saturating_mul(1024 * 1024)
                                    };
                                    // 文件：解 bundle 并物理落盘到 `~/.clipstack/share/<YYYY-MM>/`，
                                    // content_text / content_blob 改存本机本地路径，使对端可真正复制使用。
                                    // 旧端兼容：bundle 解析失败（对端为旧版，payload 为 JSON 路径）则按原样存储（本机不可用，仅展示）。
                                    match decode_file_bundle(&r.item.payload) {
                                        Some(entries) => {
                                            // 按本端文件大小上限过滤：单个文件超过上限则不落盘。
                                            let mut skipped_files: Vec<String> = Vec::new();
                                            let entries: Vec<(String, Vec<u8>)> = entries
                                                .into_iter()
                                                .filter_map(|(name, data)| {
                                                    if (data.len() as u64) > limit_bytes {
                                                        skipped_files.push(name);
                                                        None
                                                    } else {
                                                        Some((name, data))
                                                    }
                                                })
                                                .collect();
                                            let (joined, blob, size) = match app_r.path().home_dir() {
                                                Ok(home) => {
                                                    // 按当前月份分目录落盘；条目 name 可能含子目录（目录包），
                                                    // 据此重建层级；顶层名冲突时加 -1/-2 后缀避免覆盖。
                                                    let dir = share_month_dir(&home);
                                                    let _ = std::fs::create_dir_all(&dir);
                                                    let mut local_paths: Vec<String> = Vec::new();
                                                    let mut used_tops: std::collections::HashSet<String> =
                                                        std::collections::HashSet::new();
                                                    // 同一顶层目录（目录包）的所有条目必须映射到同一个去重名，
                                                    // 否则目录内文件会被拆到不同目录。
                                                    let mut top_map: std::collections::HashMap<String, String> =
                                                        std::collections::HashMap::new();
                                                    let mut total: u64 = 0;
                                                    for (name, data) in entries {
                                                        // 防目录穿越：拒绝绝对路径与 ".." 分量。
                                                        if name.starts_with('/') {
                                                            continue;
                                                        }
                                                        let rel = name.replace('\\', "/");
                                                        if rel.split('/').any(|c| c == "..") {
                                                            continue;
                                                        }
                                                        // 顶层段（目录包为目录名、单文件为文件名）去重。
                                                        let top =
                                                            rel.split('/').next().unwrap_or(&rel).to_string();
                                                        let unique_top = if let Some(u) = top_map.get(&top) {
                                                            u.clone()
                                                        } else {
                                                            let u = unique_share_name(&dir, &top, &mut used_tops);
                                                            top_map.insert(top.clone(), u.clone());
                                                            u
                                                        };
                                                        // 将 rel 的顶层段替换为去重后的版本，保留子目录层级。
                                                        let rel_unique = if unique_top == top {
                                                            rel.clone()
                                                        } else {
                                                            let rest: Vec<&str> =
                                rel.split('/').skip(1).collect();
                                                            if rest.is_empty() {
                                                                unique_top.clone()
                                                            } else {
                                                                format!("{}/{}", unique_top, rest.join("/"))
                                                            }
                                                        };
                                                        let dest = dir.join(&rel_unique);
                                                        if let Some(parent) = dest.parent() {
                                                            let _ = std::fs::create_dir_all(parent);
                                                        }
                                                        if std::fs::write(&dest, &data).is_ok() {
                                                            total += data.len() as u64;
                                                            // 含子目录则记录顶层目录（便于「另存为」整体复制），
                                                            // 否则记录文件本身。
                                                            if rel.contains('/') {
                                                                let tp = dir
                                                                    .join(&unique_top)
                                                                    .to_string_lossy()
                                                                    .into_owned();
                                                                if !local_paths.contains(&tp) {
                                                                    local_paths.push(tp);
                                                                }
                                                            } else {
                                                                local_paths
                                                                    .push(dest.to_string_lossy().into_owned());
                                                            }
                                                        } else {
                                                            eprintln!("[lan] 写入共享文件失败: {dest:?}");
                                                        }
                                                    }
                                                    let joined = local_paths.join(", ");
                                                    let blob = serde_json::to_vec(&local_paths)
                                                        .unwrap_or_default();
                                                    (joined, blob, total as i64)
                                                }
                                                Err(_) => {
                                                    eprintln!("[lan] 无法定位 home 目录，共享文件未落盘");
                                                    let paths: Vec<String> =
                                                        serde_json::from_slice(&r.item.payload)
                                                            .unwrap_or_default();
                                                    let joined = paths.join(", ");
                                                    let len = joined.len() as i64;
                                                    (joined, r.item.payload.clone(), len)
                                                }
                                            };
                                            let joined = if joined.is_empty() && !skipped_files.is_empty() {
                                                format!(
                                                    "{} 个文件超过本端大小上限({}MB)已跳过",
                                                    skipped_files.len(),
                                                    limit_bytes / 1024 / 1024
                                                )
                                            } else if !skipped_files.is_empty() {
                                                format!(
                                                    "{joined}（{n} 个文件因超本端上限已跳过）",
                                                    n = skipped_files.len()
                                                )
                                            } else {
                                                joined
                                            };
                                            (joined, Some(blob), size as i64)
                                        }
                                        None => {
                                            let paths: Vec<String> =
                                                serde_json::from_slice(&r.item.payload)
                                                    .unwrap_or_default();
                                            let joined = paths.join(", ");
                                            let size: u64 = paths
                                                .iter()
                                                .filter_map(|p| std::fs::metadata(p).ok())
                                                .map(|m| m.len())
                                                .sum();
                                            (joined, Some(r.item.payload.clone()), size as i64)
                                        }
                                    }
                                }
                            } else {
                                let text = String::from_utf8_lossy(&r.item.payload).to_string();
                                let len = text.len() as i64;
                                (text, None, len)
                            };
                            let sync_id = r.item.sync_id.clone();
                            let src_app = r.item.source_app.clone();
                            // 对端友好设备名：mDNS ServiceResolved 时按 device_id 存入 peer_names。
                            let dev_name = {
                                let g = inner_r.lock().await;
                                g.peer_names
                                    .get(&r.item.device_id)
                                    .cloned()
                                    .filter(|n| !n.is_empty())
                                    .unwrap_or_else(|| r.item.device_id.clone())
                            };
                            let lamport = r.item.lamport as i64;
                            let hash = r.item.hash.clone();
                            // 先落库，再在释放 db 锁之后才 emit 事件。
                            // 注意：clipboard-changed 的监听回调会再次获取 db.key / db.conn，
                            // 若在持有 std::sync::Mutex 锁的上下文中 emit 会重入死锁，进而永久
                            // 卡死接收线程并拖垮整个 UI（表现为一方复制、另一方卡死无响应）。
                            let mut changed_item: Option<HistoryItem> = None;
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
                                        content_blob: blob.as_deref(),
                                        source_app: &src_app,
                                        size_bytes,
                                        hash: &hash,
                                        is_sensitive: false,
                                        origin_device: &dev_name,
                                        sync_id: &sync_id,
                                        lamport,
                                        profile_id: "",
                                    },
                                ) {
                                    Ok(Some(rid)) => {
                                        // 新条目：记录待发送事件，稍后于锁外 emit。
                                        // 用真实插入行 id，保证前端 prepend 后该条目可被选中 / 详情查看。
                                        changed_item = Some(HistoryItem {
                                            id: rid,
                                            content_type: content_type_from_str(&ct_str),
                                            content_text: text.clone(),
                                            preview: text.clone(),
                                            source_app: src_app.clone(),
                                            size_bytes,
                                            hash: hash.clone(),
                                            is_pinned: false,
                                            is_favorite: false,
                                            is_sensitive: false,
                                            created_at: db::now_ms(),
                                            origin_device: dev_name.clone(),
                                            is_remote: true,
                                            deleted_at: None,
                                        });
                                    }
                                    Ok(None) => {} // 已存在（去重），不刷新
                                    Err(e) => eprintln!("[lan] 写入对端条目失败: {e}"),
                                }
                            }
                            // 锁已释放，再通知前端 / 托盘刷新，避免重入死锁。
                            if let Some(hi) = changed_item {
                                let _ = app_r.emit("clipboard-changed", hi);
                            }
                            let _ = app_r.emit(
                                "lan-clipboard-received",
                                ReceivedClipPayload {
                                    sync_id,
                                    origin_device: dev_name,
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
                _ = ping.tick() => {
                    let _ = tx_r.send(Message::Ping(Default::default()));
                    // 仅在「长时间未收到任何消息 / Pong」时判定对端已断线，避免大文件传输
                    // 期间被误杀：正常连接每 15s 互发 Ping/Pong，即便正忙于接收大消息，
                    // 底层仍会自动回 Pong，连接活跃度（last_activity）会刷新。
                    if last_activity.elapsed() > idle {
                        break;
                    }
                }
            }
        }
        // 连接关闭：移除并通知。
        if let Some(id) = learned_id {
            remove_conn(&inner_r, &app_r, &id).await;
        }
    });

    // 取得读任务中止句柄：客户端侧（peer_id 已知）立即登记；服务端侧待握手/首条信封学到
    // 对端 id 后再从 abort_slot 取出登记（见读循环内两处 learned 分支）。
    let read_abort = read_task.abort_handle();
    *abort_slot.lock().unwrap() = Some(read_abort.clone());
    if let Some(id) = &peer_id_for_outside {
        inner.lock().await.conn_aborts.insert(id.clone(), read_abort);
    }
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
    my_gen: u64,
) {
    let (is_new, resolved) = {
        let mut g = inner.lock().await;
        // 代际守卫：stop()/start() 已使本连接失效（gen 已递增），跳过登记。
        // 否则协作式取消的读任务会在 stop 清空 conns 后再次把对端插回，前端 refreshPeers
        // 读到脏列表会让「在线设备」面板残留已断开设备。
        if g.gen != my_gen {
            return;
        }
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
    // 同步清理中止句柄，避免泄漏（stop() 已统一 abort 并清空，此处兜底常态断线场景）。
    inner.lock().await.conn_aborts.remove(device_id);
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
    // 明文即剪贴板真实内容字节，而非整个 ClipboardItem 的 JSON 序列化。
    // 否则接收端会把 {sync_id,device_id,lamport,kind,hash,payload} 的 JSON 当成内容显示。
    let plain = item.payload;
    let mut sealed = crypto::encrypt(psk, &plain); // nonce(12) || ct
    if sealed.len() < NONCE_LEN {
        return None;
    }
    let nonce = sealed[..NONCE_LEN].to_vec();
    let ciphertext = sealed.split_off(NONCE_LEN);
    Some(SyncEnvelope {
        sync_id: item.sync_id,
        device_id: cfg.device_id.clone(),
        source_app: item.source_app,
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
    // 通过向若干公共 DNS 发起 UDP connect 探测本机出口 IPv4。
    // 程序启动早期网络路由可能尚未就绪（或某 DNS 被网络屏蔽），
    // 故逐一尝试多个目标，任一可达即可拿到真实出口地址；全部失败才返回 None
    // （调用方会退化为 0.0.0.0，并依赖启动兜底逻辑在稍后重启时重新注册真实地址）。
    const PROBES: [&str; 3] = ["8.8.8.8:80", "1.1.1.1:80", "9.9.9.9:80"];
    for target in PROBES {
        if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if sock.connect(target).is_ok() {
                if let Ok(addr) = sock.local_addr() {
                    if !addr.ip().is_unspecified() {
                        return Some(addr.ip());
                    }
                }
            }
        }
    }
    None
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

/// 判断某类型是否在「共享类型」白名单内。文本 / 链接 / 代码统一归入 `text` 开关；
/// 图片归 `image`；文件归 `file`。白名单为空（全部取消）时任何类型都不允许。
fn kind_allowed(share_types: &[String], kind: ClipKind) -> bool {
    let want = match kind {
        ClipKind::Image => "image",
        ClipKind::File => "file",
        _ => "text",
    };
    share_types.iter().any(|t| t == want)
}

/// 单帧数据负载上限（字节）。将大信封分片为多个小帧，使 Ping/Pong 控制帧可在分片间
/// 穿插，避免弱网下大文件传输期间连接被心跳误判为断线而中断。
const LAN_CHUNK: usize = 32 * 1024;

/// 帧类型标签（首字节）：`START` 携带总长度(u32 BE) + 首片；`DATA` 携带后续分片；
/// 接收端按总长度重组，凑齐后再 `SyncEnvelope::from_bytes`。
const FRAME_START: u8 = 0x01;
const FRAME_DATA: u8 = 0x02;

/// 把一条完整信封字节流切分为多帧 `Message::Binary`，供 WebSocket 顺序发送。
fn build_chunk_frames(data: &[u8]) -> Vec<Message> {
    let total = (data.len() as u64).min(u32::MAX as u64) as u32;
    let mut frames: Vec<Message> = Vec::new();
    let mut offset = 0usize;
    let mut first = true;
    while offset < data.len() {
        let end = (offset + LAN_CHUNK).min(data.len());
        let chunk = &data[offset..end];
        let v: Vec<u8> = if first {
            let mut b = Vec::with_capacity(5 + chunk.len());
            b.push(FRAME_START);
            b.extend_from_slice(&total.to_be_bytes());
            b.extend_from_slice(chunk);
            b
        } else {
            let mut b = Vec::with_capacity(1 + chunk.len());
            b.push(FRAME_DATA);
            b.extend_from_slice(chunk);
            b
        };
        frames.push(Message::Binary(v.into()));
        offset = end;
        first = false;
    }
    // 空数据兜底：保证至少发出一帧 START（total=0），接收端据此正常复位重组状态。
    if frames.is_empty() {
        let mut b = Vec::with_capacity(5);
        b.push(FRAME_START);
        b.extend_from_slice(&0u32.to_be_bytes());
        frames.push(Message::Binary(b.into()));
    }
    frames
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

/// 从 PNG 字节解析尺寸，生成「WxH 图片」占位文本（与本地捕获展示一致）。
/// 解析失败（非 PNG / 字节不足）返回 None，调用方回退为「图片」。
fn png_dimensions_label(blob: &[u8]) -> Option<String> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if blob.len() < 24 || &blob[..8] != SIG {
        return None;
    }
    let width = u32::from_be_bytes([blob[16], blob[17], blob[18], blob[19]]);
    let height = u32::from_be_bytes([blob[20], blob[21], blob[22], blob[23]]);
    Some(format!("{width}×{height} 图片"))
}

/// 局域网共享「文件」的物理落盘目录：`~/.clipstack/share/<YYYY-MM>/`。
/// 按当前 UTC 月份分目录，便于按时间归类与清理。
fn share_month_dir(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".clipstack").join("share").join(current_utc_month())
}

/// 计算共享目录下的不冲突顶层名（文件或目录）：若 `dir/name` 已存在或已在本批次占用，
/// 则按 `name-1` / `name-2` 顺序寻找未占用名。`used` 跨本批次条目共享，避免同包内重复分配。
fn unique_share_name(
    dir: &std::path::Path,
    name: &str,
    used: &mut std::collections::HashSet<String>,
) -> String {
    let cand = dir.join(name);
    if !cand.exists() && used.insert(name.to_string()) {
        return name.to_string();
    }
    used.remove(name); // 上面仅当已存在时才可能误插入，撤销
    let mut n: u32 = 1;
    loop {
        let cand_name = format!("{name}-{n}");
        let cand = dir.join(&cand_name);
        if !cand.exists() && used.insert(cand_name.clone()) {
            return cand_name;
        }
        n += 1;
    }
}

/// 局域网共享文件根目录：`~/.clipstack/share/`（所有月份子目录的父目录）。
/// 供命令层统计/打开/清空共享文件夹时使用。
pub fn share_root(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".clipstack").join("share")
}

/// 当前 UTC 年-月（如 "2026-08"），用于共享文件按月分目录。
/// 不引入额外依赖，基于 UNIX 时间戳逐年份 / 月份折算。
fn current_utc_month() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut days = (secs / 86400) as i64;
    let mut year = 1970i64;
    loop {
        let ydays = if is_leap_year(year) { 366 } else { 365 };
        if days >= ydays {
            days -= ydays;
            year += 1;
        } else {
            break;
        }
    }
    let month_days: [i64; 12] = [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut month = 1;
    let mut d = days;
    while month <= 12 {
        if d >= month_days[month - 1] {
            d -= month_days[month - 1];
            month += 1;
        } else {
            break;
        }
    }
    format!("{year:04}-{month:02}")
}

/// 闰年判定（公历）。
fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// 将一组「文件名 + 字节」编码为自描述的二进制 bundle（用于跨端传输文件内容）。
/// 文件类型（`ClipKind::File`）的 `payload` 由本格式承载，与图片（裸 PNG）、文本（裸 UTF-8）区分。
/// 格式：文件数(u32 BE) + 每文件[名称长度(u32 BE) + 名称(UTF-8) + 数据长度(u64 BE) + 数据]。
fn encode_file_bundle(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (name, data) in entries {
        let nb = name.as_bytes();
        out.extend_from_slice(&(nb.len() as u32).to_be_bytes());
        out.extend_from_slice(nb);
        out.extend_from_slice(&(data.len() as u64).to_be_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// 解码 `encode_file_bundle` 的产物；格式损坏返回 None（接收端据此回退为旧版 JSON 路径兼容）。
fn decode_file_bundle(blob: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    if blob.len() < 4 {
        return None;
    }
    let count = u32::from_be_bytes([blob[0], blob[1], blob[2], blob[3]]) as usize;
    let mut pos = 4;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if pos + 4 > blob.len() {
            return None;
        }
        let nl = u32::from_be_bytes([blob[pos], blob[pos + 1], blob[pos + 2], blob[pos + 3]]) as usize;
        pos += 4;
        if pos + nl > blob.len() {
            return None;
        }
        let name = String::from_utf8(blob[pos..pos + nl].to_vec()).ok()?;
        pos += nl;
        if pos + 8 > blob.len() {
            return None;
        }
        let dl = u64::from_be_bytes([
            blob[pos],
            blob[pos + 1],
            blob[pos + 2],
            blob[pos + 3],
            blob[pos + 4],
            blob[pos + 5],
            blob[pos + 6],
            blob[pos + 7],
        ]) as usize;
        pos += 8;
        if pos + dl > blob.len() {
            return None;
        }
        let data = blob[pos..pos + dl].to_vec();
        pos += dl;
        out.push((name, data));
    }
    Some(out)
}

/// 由本地文件的 JSON 路径数组（`content_blob`）读取每个文件字节，组装跨端传输用 entries。
/// 单个文件超过 `limit_bytes`（= `file_limit_mb` MB）则跳过，避免超大文件撑爆 WebSocket。
/// `name` 仅取文件名校验分量（防目录穿越），对端按文件名落盘到本地 share 目录。
fn build_file_entries(json_paths: &Option<Vec<u8>>, limit_bytes: u64) -> Vec<(String, Vec<u8>)> {
    let Some(paths) = json_paths else {
        return Vec::new();
    };
    let Ok(paths) = serde_json::from_slice::<Vec<String>>(paths) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for p in paths {
        let path = std::path::Path::new(&p);
        let Ok(meta) = std::fs::metadata(path) else {
            eprintln!("[lan] 共享文件不存在，跳过: {p}");
            continue;
        };
        if meta.is_dir() {
            // 目录（如 macOS 的 .rtfd / .app 包，Finder 显示为单个文件）无法用 std::fs::read
            // 直接读取，需递归拍平为多条条目；name 用相对目录的路径（含顶层目录名），
            // 对端据此在本地 share 目录重建层级结构。
            collect_dir_entries(path, path, limit_bytes, &mut out);
            continue;
        }
        if meta.len() > limit_bytes {
            eprintln!(
                "[lan] 共享文件超过大小上限({}MB)，跳过: {p}",
                limit_bytes / 1024 / 1024
            );
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("[lan] 读取共享文件失败，跳过: {p}");
            continue;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        out.push((name, bytes));
    }
    out
}

/// 递归收集目录内常规文件为传输条目。
/// `root` 为被共享的目录本身；条目 `name` 取相对 `root.parent()` 的路径（含顶层目录名，
/// 如 `activiti.rtfd/TXT.rtf`），对端按此重建目录结构。
/// 单个文件超过 `limit_bytes` 则跳过；符号链接按目标内容读取（仅常规文件入列）。
fn collect_dir_entries(
    root: &std::path::Path,
    dir: &std::path::Path,
    limit_bytes: u64,
    out: &mut Vec<(String, Vec<u8>)>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    // 顶层目录名保留：相对 root 的父目录取路径。
    let base = root.parent().unwrap_or_else(|| std::path::Path::new(""));
    for entry in rd.flatten() {
        let p = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        let rel = match p.strip_prefix(base) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        if rel.is_empty() {
            continue;
        }
        if ft.is_dir() {
            collect_dir_entries(root, &p, limit_bytes, out);
        } else if ft.is_symlink() {
            if let Ok(bytes) = std::fs::read(&p) {
                if (bytes.len() as u64) <= limit_bytes {
                    out.push((rel, bytes));
                }
            }
        } else if ft.is_file() {
            let Ok(meta) = p.metadata() else { continue };
            if (meta.len() as u64) > limit_bytes {
                eprintln!(
                    "[lan] 共享文件超过大小上限({}MB)，跳过: {rel}",
                    limit_bytes / 1024 / 1024
                );
                continue;
            }
            if let Ok(bytes) = std::fs::read(&p) {
                out.push((rel, bytes));
            }
        }
    }
}

/// 由文本构造一条待广播的 ClipboardItem（测试 / L3 监控钩子复用）。
pub fn text_item(text: &str) -> ClipboardItem {
    let payload = text.as_bytes().to_vec();
    let hash = ClipboardItem::content_hash(&payload);
    ClipboardItem {
        sync_id: Uuid::new_v4().to_string(),
        device_id: String::new(), // 由 broadcast 时覆盖为本地
        source_app: String::new(),
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
    fn chunk_frames_roundtrip_reassembles_envelope() {
        // 验证大信封分片发送后能被接收端按 START(total)+DATA 完整重组，
        // 这是「大文件在弱网下也能传完」协议层正确性的核心保障。
        let data: Vec<u8> = (0u8..=255).cycle().take(200_000).collect();
        let frames = build_chunk_frames(&data);
        // 应当被切成多帧（单帧上限 LAN_CHUNK）。
        assert!(frames.len() > 1, "大信封应被分片");

        let mut asm_total: Option<usize> = None;
        let mut asm_buf: Vec<u8> = Vec::new();
        for f in &frames {
            let Message::Binary(bytes) = f else {
                panic!("分片应为 Binary");
            };
            match bytes[0] {
                FRAME_START => {
                    let total = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
                    asm_total = Some(total);
                    asm_buf.clear();
                    asm_buf.extend_from_slice(&bytes[5..]);
                }
                FRAME_DATA => {
                    asm_buf.extend_from_slice(&bytes[1..]);
                }
                _ => panic!("未知帧类型"),
            }
        }
        let need = asm_total.unwrap();
        assert_eq!(asm_buf.len(), need, "重组长度应与声明总长度一致");
        assert_eq!(asm_buf, data, "重组内容应与原始信封一致");
    }

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
        // 设备 ID 必须持久化并在重载后保持一致，否则跨启动随机变化会破坏对端识别与连接。
        assert_eq!(reloaded.device_id, cfg.device_id);
        // 再次重载（模拟再次启动）仍应保持同一 ID，不会重新生成。
        let mut reload2 = LanConfig::default();
        load_persisted_config(&db, &mut reload2);
        assert_eq!(reload2.device_id, cfg.device_id);
    }

    #[test]
    fn file_bundle_encode_decode_roundtrip() {
        // 文件跨端传输的二进制 bundle 必须能无损往返：名称与字节均还原。
        let entries = vec![
            ("a.txt".to_string(), b"hello".to_vec()),
            ("b.bin".to_string(), vec![0u8, 1, 2, 255, 254]),
            ("".to_string(), Vec::new()),
        ];
        let bundle = encode_file_bundle(&entries);
        let back = decode_file_bundle(&bundle).expect("bundle 应可解码");
        assert_eq!(back.len(), entries.len());
        assert_eq!(back[0], entries[0]);
        assert_eq!(back[1], entries[1]);
        assert_eq!(back[2], entries[2]);
    }

    #[test]
    fn file_bundle_decode_rejects_garbage() {
        // 非 bundle 数据（如旧版 JSON 路径数组）解析必须失败，触发旧端兼容回退。
        let json = serde_json::to_vec(&vec!["/Users/x/a.png"]).unwrap();
        assert!(decode_file_bundle(&json).is_none());
        assert!(decode_file_bundle(&[]).is_none());
        assert!(decode_file_bundle(&[1, 2, 3]).is_none());
    }

    #[test]
    fn build_file_entries_flattens_directory_with_subpaths() {
        // 目录（如 macOS .rtfd 包）曾因 std::fs::read 失败被整体跳过 → 对端收到空 bundle。
        // 修复后：递归拍平为多条条目，name 保留含顶层目录名的相对路径。
        use std::io::Write;
        let base = std::env::temp_dir().join(format!("clipstack_lan_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pkg = base.join("demo.rtfd");
        std::fs::create_dir_all(pkg.join("sub")).unwrap();
        let mut f1 = std::fs::File::create(pkg.join("TXT.rtf")).unwrap();
        f1.write_all(b"hello").unwrap();
        let mut f2 = std::fs::File::create(pkg.join("sub").join("img.png")).unwrap();
        f2.write_all(b"PNGDATA").unwrap();

        let json = serde_json::to_vec(&vec![pkg.to_string_lossy().to_string()]).unwrap();
        let entries = build_file_entries(&Some(json), 10 * 1024 * 1024);
        // 两个文件都应被拍平，且 name 含顶层目录名与相对层级。
        assert_eq!(entries.len(), 2);
        let names: Vec<String> = entries.iter().map(|(n, _)| n.clone()).collect();
        assert!(names.iter().any(|n| n == "demo.rtfd/TXT.rtf"));
        assert!(names.iter().any(|n| n == "demo.rtfd/sub/img.png"));
        // 解码后子路径应完整保留（接收端据此重建目录）。
        let bundle = encode_file_bundle(&entries);
        let back = decode_file_bundle(&bundle).expect("bundle 应可解码");
        let back_names: Vec<String> = back.iter().map(|(n, _)| n.clone()).collect();
        assert!(back_names.iter().any(|n| n == "demo.rtfd/TXT.rtf"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn png_dimensions_label_parses_ihdr() {
        // 构造最小合法 PNG（仅 8 字节签名 + IHDR），验证尺寸解析与「WxH 图片」占位。
        let mut png = vec![
            137, 80, 78, 71, 13, 10, 26, 10, // 签名
            0, 0, 0, 13, // IHDR 长度
            73, 72, 68, 82, // "IHDR"
        ];
        // 宽 0x0000000A=10，高 0x00000014=20
        png.extend_from_slice(&[0, 0, 0, 10, 0, 0, 0, 20]);
        png.extend_from_slice(&[8, 6, 0, 0, 0]); // 其余 IHDR 字段
        assert_eq!(png_dimensions_label(&png), Some("10×20 图片".to_string()));
        // 非 PNG 数据应返回 None。
        assert_eq!(png_dimensions_label(b"not a png"), None);
    }
}
