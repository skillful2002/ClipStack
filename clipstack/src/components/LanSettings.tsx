// L4 · 局域网共享设置面板：共享组 / 密钥 / 本机名 / 发布开关 / 文件上限 / 手动对端 / 在线设备。
//
// 仅暴露「单一激活配置」（与后端 LanManager 的当前 config 对应）。密钥在前端不回显，
// 留空时表示「保持现有密钥」（后端 lan_set_config 对空密钥不覆盖）。

import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import { useT } from "../lib/i18n";
import { useHistory } from "../store/history";

/** 可共享的内容类型白名单，顺序即界面展示顺序。 */
const SHARE_TYPES = ["text", "image", "file"] as const;

export function LanSettings() {
  const t = useT();
  const setToast = useHistory((s) => s.setToast);

  const [deviceId, setDeviceId] = useState("");
  const [group, setGroup] = useState("");
  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [shareOut, setShareOut] = useState(false);
  const [fileLimitMb, setFileLimitMb] = useState(10);
  const [manualPeers, setManualPeers] = useState("");
  const [shareTypes, setShareTypes] = useState<string[]>([...SHARE_TYPES]);
  const [hasKey, setHasKey] = useState(false);
  const [port, setPort] = useState(21995);
  const [listenPort, setListenPort] = useState(21995);
  const [localIp, setLocalIp] = useState("");
  const [portError, setPortError] = useState("");
  const [showKey, setShowKey] = useState(false);

  const [peers, setPeers] = useState<api.PeerInfo[]>([]);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);

  const refreshPeers = async () => {
    try {
      setPeers(await api.lanGetPeers());
    } catch {
      /* 不可用时静默 */
    }
  };

  // 初始化：读取当前配置 + 在线设备，并订阅上下线 / 收到共享事件。
  useEffect(() => {
    void (async () => {
      try {
        const cfg = await api.lanGetConfig();
        setDeviceId(cfg.deviceId);
        setGroup(cfg.group);
        setName(cfg.name);
        setShareOut(cfg.shareOut);
        setFileLimitMb(cfg.fileLimitMb);
        setShareTypes(
          cfg.shareTypes && cfg.shareTypes.length > 0
            ? cfg.shareTypes.filter((x) => (SHARE_TYPES as readonly string[]).includes(x))
            : [...SHARE_TYPES],
        );
        setHasKey(cfg.hasKey);
        setPort(cfg.port);
        setListenPort(cfg.port);
        setLocalIp(cfg.localIp);
        setManualPeers(cfg.manualPeers.join("\n"));
      } catch (e) {
        setToast(t("lan.saveFailed", { error: String(e) }));
      }
      await refreshPeers();
    })();

    const unlisteners = Promise.all([
      api.onLanPeerOnline((p) => setPeers((prev) => upsertPeer(prev, p))),
      api.onLanPeerOffline((p) =>
        setPeers((prev) => prev.filter((x) => x.deviceId !== p.deviceId)),
      ),
      api.onLanClipboardReceived((payload) =>
        setToast(t("lan.receivedFrom", { name: payload.originDevice || t("item.local") })),
      ),
      api.onLanPortInUse((payload) => {
        setPortError(t("lan.portInUse", { port: payload.port }));
        setToast(t("lan.portInUse", { port: payload.port }));
      }),
    ]);
    return () => {
      void unlisteners.then((fns) => fns.forEach((fn) => fn()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 仅更新本地状态；是否共享与其它共享参数统一在「立即生效」时落库，
  // 避免未点保存就已对局域网广播。
  const onShareOutChange = (next: boolean) => {
    setShareOut(next);
  };

  const onSave = async () => {
    setBusy(true);
    try {
      const peersList = manualPeers
        .split("\n")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
      await api.lanSetConfig({
        group: group.trim(),
        key: key, // 留空 = 保持现有密钥
        name: name.trim(),
        shareOut,
        fileLimitMb: Math.max(1, Math.min(1024, fileLimitMb || 10)),
        shareTypes,
        manualPeers: peersList,
        port: listenPort || 0, // 0 由后端回退为默认 LAN_PORT
      });
      setHasKey(true); // 保存后视为已设（除非组/密钥被清空）
      setPortError("");
      setToast(t("lan.saved"));
      await refreshPeers();
    } catch (e) {
      setToast(t("lan.saveFailed", { error: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const onTest = async () => {
    setTesting(true);
    try {
      const n = await api.lanSendTest(
        t("lan.testSend") + " — " + new Date().toLocaleTimeString(),
      );
      setToast(t("lan.testSent", { n }));
      await refreshPeers();
    } catch (e) {
      setToast(t("lan.testFailed", { error: String(e) }));
    } finally {
      setTesting(false);
    }
  };

  return (
    <>
      <div className="settings-card">
        <div className="settings-card-title">{t("lan.title")}</div>
        <p className="settings-hint">{t("lan.hint")}</p>

        <div className="settings-row">
          <span>{t("lan.shareOut")}</span>
          <label className="switch">
            <input
              type="checkbox"
              checked={shareOut}
              onChange={(e) => onShareOutChange(e.target.checked)}
            />
            <span className="slider" />
          </label>
        </div>
        <p className="settings-hint">{t("lan.shareOutHint")}</p>

        <div className="settings-row">
          <span>{t("lan.group")}</span>
          <input
            type="text"
            placeholder={t("lan.groupPlaceholder")}
            value={group}
            disabled={!shareOut}
            onChange={(e) => setGroup(e.target.value)}
          />
        </div>

        <div className="settings-row">
          <span>{t("lan.key")}</span>
          <div className="input-with-action">
            <input
              type={showKey ? "text" : "password"}
              autoComplete="new-password"
              placeholder={hasKey ? t("lan.keyPlaceholderKeep") : t("lan.keyPlaceholder")}
              value={key}
              disabled={!shareOut}
              onChange={(e) => setKey(e.target.value)}
            />
            <button
              type="button"
              className="input-reveal"
              title={t("lan.toggleKey")}
              aria-label={t("lan.toggleKey")}
              onClick={async () => {
                const next = !showKey;
                // 已设置过密钥但输入框为空（留空=保持现有密钥）时，
                // 点击显示则从后端取回明文密钥，便于核对旧密码。
                if (next && key === "" && hasKey) {
                  try {
                    setKey(await api.lanGetKey());
                  } catch {
                    /* 取回失败则保持隐藏 */
                  }
                }
                setShowKey(next);
              }}
            >
              {showKey ? (
                <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M1 12s4-7 11-7 11 7 11 7-4 7-11 7-11-7-11-7z" />
                  <circle cx="12" cy="12" r="3" />
                </svg>
              ) : (
                <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24" />
                  <line x1="1" y1="1" x2="23" y2="23" />
                </svg>
              )}
            </button>
          </div>
        </div>

        <div className="settings-row">
          <span>{t("lan.name")}</span>
          <input
            type="text"
            placeholder={t("lan.namePlaceholder")}
            value={name}
            disabled={!shareOut}
            onChange={(e) => setName(e.target.value)}
          />
        </div>

        <div className="settings-row">
          <span>{t("lan.localIp")}</span>
          <input
            type="text"
            className="readonly-input"
            readOnly
            value={localIp || t("lan.localIpUnknown")}
          />
        </div>

        <div className="settings-row">
          <span>{t("lan.listenPort")}</span>
          <input
            type="number"
            min={1}
            max={65535}
            value={listenPort}
            disabled={!shareOut}
            onChange={(e) => {
              setListenPort(Number(e.target.value));
              setPortError("");
            }}
          />
        </div>
        <p className="settings-hint">{t("lan.listenPortHint", { port })}</p>
        {portError && <p className="settings-hint lan-port-error">{portError}</p>}

        <div className="settings-row">
          <span>{t("lan.fileLimit")}</span>
          <input
            type="number"
            min={1}
            max={1024}
            value={fileLimitMb}
            disabled={!shareOut}
            onChange={(e) => setFileLimitMb(Number(e.target.value))}
          />
        </div>

        <div className="settings-row settings-row-column">
          <span>{t("lan.shareTypes")}</span>
          <div className="lan-share-types">
            {SHARE_TYPES.map((tp) => (
              <label key={tp} className="checkbox-inline">
                <input
                  type="checkbox"
                  checked={shareTypes.includes(tp)}
                  disabled={!shareOut}
                  onChange={(e) => {
                    const next = e.target.checked
                      ? [...shareTypes, tp]
                      : shareTypes.filter((x) => x !== tp);
                    setShareTypes(next);
                  }}
                />
                <span>{t(`type.${tp}`)}</span>
              </label>
            ))}
          </div>
          <p className="settings-hint">{t("lan.shareTypesHint")}</p>
        </div>

        <div className="settings-row settings-row-column">
          <span className="lan-manual-head">
            {t("lan.manualPeers")}
            <code className="lan-port-badge">
              {t("lan.port")} {port}
            </code>
          </span>
          <textarea
            className="lan-peers"
            rows={3}
            placeholder={t("lan.manualPeersPlaceholder", { port })}
            value={manualPeers}
            disabled={!shareOut}
            onChange={(e) => setManualPeers(e.target.value)}
          />
          <p className="settings-hint">{t("lan.manualPeersHint", { port })}</p>
        </div>

        <div className="settings-actions">
          <button className="primary" disabled={busy} onClick={() => void onSave()}>
            {t("lan.applyNow")}
          </button>
          <button disabled={testing} onClick={() => void onTest()}>
            {testing ? t("lan.testing") : t("lan.testSend")}
          </button>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-card-title lan-subtitle">{t("lan.onlineDevices")}</div>
        <div className="settings-list">
          {/* 本机置顶：确认本机在线与共享状态（本机不进入远端连接列表） */}
          <div className="settings-list-item lan-self">
            <span className="item-name">
              {t("lan.thisDevice")}
              <span className="lan-peer-addr"> {deviceId}</span>
            </span>
            <span className={`lan-self-status${shareOut ? " on" : ""}`}>
              {shareOut ? t("lan.sharing") : t("lan.shareStopped")}
            </span>
          </div>
          {peers.map((p) => (
            <div key={p.deviceId} className="settings-list-item">
              <span className="item-name">
                {p.name || p.deviceId}
                <span className="lan-peer-addr"> {p.addr}</span>
              </span>
              <span className={`lan-peer-status${p.connected ? " on" : ""}`}>
                {p.connected ? "●" : "○"}
              </span>
            </div>
          ))}
        </div>
        {peers.length === 0 && (
          <p className="settings-hint lan-no-others">{t("lan.noOtherPeers")}</p>
        )}
      </div>
    </>
  );
}

/** 上线事件：按 deviceId 更新或插入对端。 */
function upsertPeer(prev: api.PeerInfo[], p: api.PeerInfo): api.PeerInfo[] {
  const idx = prev.findIndex((x) => x.deviceId === p.deviceId);
  if (idx < 0) return [...prev, p];
  const next = prev.slice();
  next[idx] = p;
  return next;
}
