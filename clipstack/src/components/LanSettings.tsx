// L4 · 局域网共享设置面板：共享组 / 密钥 / 本机名 / 发布开关 / 文件上限 / 手动对端 / 在线设备。
//
// 仅暴露「单一激活配置」（与后端 LanManager 的当前 config 对应）。密钥在前端不回显，
// 留空时表示「保持现有密钥」（后端 lan_set_config 对空密钥不覆盖）。

import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import { useT } from "../lib/i18n";
import { useHistory } from "../store/history";

export function LanSettings() {
  const t = useT();
  const setToast = useHistory((s) => s.setToast);

  const [deviceId, setDeviceId] = useState("");
  const [group, setGroup] = useState("");
  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [shareOut, setShareOut] = useState(false);
  const [fileLimitMb, setFileLimitMb] = useState(100);
  const [manualPeers, setManualPeers] = useState("");
  const [hasKey, setHasKey] = useState(false);
  const [port, setPort] = useState(21995);
  const [listenPort, setListenPort] = useState(21995);
  const [portError, setPortError] = useState("");

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
        setHasKey(cfg.hasKey);
        setPort(cfg.port);
        setListenPort(cfg.port);
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

  const onShareOutChange = async (next: boolean) => {
    setShareOut(next);
    try {
      await api.lanSetShareOut(next);
    } catch (e) {
      setToast(t("lan.saveFailed", { error: String(e) }));
    }
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
        fileLimitMb: Math.max(1, Math.min(1024, fileLimitMb || 100)),
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
          <span>{t("lan.group")}</span>
          <input
            type="text"
            placeholder={t("lan.groupPlaceholder")}
            value={group}
            onChange={(e) => setGroup(e.target.value)}
          />
        </div>

        <div className="settings-row">
          <span>{t("lan.key")}</span>
          <input
            type="password"
            autoComplete="new-password"
            placeholder={hasKey ? t("lan.keyPlaceholderKeep") : t("lan.keyPlaceholder")}
            value={key}
            onChange={(e) => setKey(e.target.value)}
          />
        </div>

        <div className="settings-row">
          <span>{t("lan.name")}</span>
          <input
            type="text"
            placeholder={t("lan.namePlaceholder")}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>

        <div className="settings-row">
          <span>{t("lan.shareOut")}</span>
          <label className="switch">
            <input
              type="checkbox"
              checked={shareOut}
              onChange={(e) => void onShareOutChange(e.target.checked)}
            />
            <span className="slider" />
          </label>
        </div>
        <p className="settings-hint">{t("lan.shareOutHint")}</p>

        <div className="settings-row">
          <span>{t("lan.fileLimit")}</span>
          <input
            type="number"
            min={1}
            max={1024}
            value={fileLimitMb}
            onChange={(e) => setFileLimitMb(Number(e.target.value))}
          />
        </div>

        <div className="settings-row">
          <span>{t("lan.listenPort")}</span>
          <input
            type="number"
            min={1}
            max={65535}
            value={listenPort}
            onChange={(e) => {
              setListenPort(Number(e.target.value));
              setPortError("");
            }}
          />
        </div>
        <p className="settings-hint">{t("lan.listenPortHint", { port })}</p>
        {portError && <p className="settings-hint lan-port-error">{portError}</p>}

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
            onChange={(e) => setManualPeers(e.target.value)}
          />
          <p className="settings-hint">{t("lan.manualPeersHint", { port })}</p>
        </div>

        <div className="settings-actions">
          <button className="primary" disabled={busy} onClick={() => void onSave()}>
            {t("settings.add")}
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
