// L4 · 局域网共享设置面板：共享组 / 密钥 / 本机名 / 发布开关 / 文件上限 / 手动对端 / 在线设备。
//
// 仅暴露「单一激活配置」（与后端 LanManager 的当前 config 对应）。密钥在前端不回显，
// 留空时表示「保持现有密钥」（后端 lan_set_config 对空密钥不覆盖）。

import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import { useT } from "../lib/i18n";
import { useHistory } from "../store/history";
import { parseLanPrereqError, lanPrereqToastMessage } from "../lib/tauri";

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
  // 共享文件统计 [文件数, 总字节数] 与保存位置。
  const [shareStats, setShareStats] = useState<[number, number]>([0, 0]);
  const [shareFolderPath, setShareFolderPath] = useState("");
  // 清空共享文件确认弹窗。
  const [showClearConfirm, setShowClearConfirm] = useState(false);

  const refreshPeers = async () => {
    try {
      setPeers(await api.lanGetPeers());
    } catch {
      /* 不可用时静默 */
    }
  };

  // 从后端重新拉取完整配置并刷新本地状态（挂载时与「托盘切换共享」等后端主动变更时复用）。
  const refreshConfig = async () => {
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
    await refreshShareFiles();
  };

  // 初始化：读取当前配置 + 在线设备，并订阅上下线 / 收到共享 / 后端配置变更事件。
  useEffect(() => {
    void refreshConfig();

    // 兜底：后端 online/offline 事件可能因 mDNS 组播在 VPN / IP 共享下转发不可靠而丢失，
    // 周期性全量拉取在线设备（后端按「机器名 + IP」去重），消除「关闭共享后列表残留 /
    // 同一台电脑重复显示」的问题。仅页面挂载期间生效。
    const refreshTimer = window.setInterval(() => {
      void refreshPeers();
    }, 10_000);

    const unlisteners = Promise.all([
      api.onLanPeerOnline((p) => setPeers((prev) => upsertPeer(prev, p))),
      api.onLanPeerOffline((p) =>
        setPeers((prev) => prev.filter((x) => x.deviceId !== p.deviceId)),
      ),
      api.onLanClipboardReceived((payload) => {
        setToast(t("lan.receivedFrom", { name: payload.originDevice || t("item.local") }));
        // 收到共享内容（可能是文件）后及时刷新「共享文件」数量与大小。
        void refreshShareFiles();
      }),
      api.onLanPortInUse((payload) => {
        setPortError(t("lan.portInUse", { port: payload.port }));
        setToast(t("lan.portInUse", { port: payload.port }));
      }),
      // 后端配置被外部变更（如托盘菜单切换「共享」开关）时，重新拉取以实时同步 UI。
      api.onLanConfigChanged(() => {
        void refreshConfig();
      }),
    ]);
    return () => {
      window.clearInterval(refreshTimer);
      void unlisteners.then((fns) => fns.forEach((fn) => fn()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 将当前共享配置提交到后端。可传入覆盖项（如切换开关、即时修改文件上限/共享类型）。
  // 「是否共享」开关的每次切换都直接走这里：一并提交全部配置并启停共享，无需额外按钮。
  const applyConfig = async (
    override?: {
      shareOut?: boolean;
      fileLimitMb?: number;
      shareTypes?: string[];
    },
    silent = false,
  ): Promise<boolean> => {
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
        shareOut: override?.shareOut ?? shareOut,
        fileLimitMb: Math.max(1, Math.min(1024, override?.fileLimitMb ?? (fileLimitMb || 10))),
        shareTypes: override?.shareTypes ?? shareTypes,
        manualPeers: peersList,
        port: listenPort || 0, // 0 由后端回退为默认 LAN_PORT
      });
      setHasKey(true); // 提交后视为已设（除非组/密钥被清空）
      setPortError("");
      if (!silent) setToast(t("lan.saved"));
      await refreshPeers();
      return true;
    } catch (e) {
      // 开启共享时前置条件（组 / 密钥 / 端口）缺失：给出明确本地化提示。
      const codes = parseLanPrereqError(e);
      if (codes) {
        setToast(lanPrereqToastMessage(codes, t));
      } else {
        setToast(t("lan.saveFailed", { error: String(e) }));
      }
      return false;
    } finally {
      setBusy(false);
    }
  };

  // 「是否共享」开关：切换即立即生效（提交全部配置并启停共享）。
  // 若后端因前置条件（组 / 密钥 / 端口）不满足而拒绝，回退开关状态。
  const onShareOutChange = async (next: boolean) => {
    setShareOut(next);
    const ok = await applyConfig({ shareOut: next });
    if (!ok) setShareOut(!next);
  };

  // 组 / 密钥 / 端口 / 手动对端 仅在失焦时落库：避免每次按键都触发 set_config（会重启 mDNS 带来连接抖动）。
  // 失焦发生在切走设置页之前，因此可覆盖「改完直接换页未保存」的场景。
  const onFieldBlur = () => {
    void applyConfig(undefined, true);
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

  // 字节数 -> 人类可读（B / KB / MB / GB）。
  const formatBytes = (n: number): string => {
    if (n < 1024) return `${n} B`;
    const kb = n / 1024;
    if (kb < 1024) return `${kb.toFixed(1)} KB`;
    const mb = kb / 1024;
    if (mb < 1024) return `${mb.toFixed(1)} MB`;
    return `${(mb / 1024).toFixed(2)} GB`;
  };

  // 刷新共享文件统计与保存位置。
  const refreshShareFiles = async () => {
    try {
      const [count, size] = await api.lanShareStats();
      setShareStats([count, size]);
      setShareFolderPath(await api.lanShareFolderPath());
    } catch {
      /* 不可用时静默 */
    }
  };

  // 在文件管理器中打开共享文件夹。
  const onOpenShareFolder = async () => {
    try {
      await api.lanOpenShareFolder();
    } catch (e) {
      setToast(t("lan.operationFailed", { error: String(e) }));
    }
  };

  // 清空共享文件夹：为空时直接提示；否则先弹出确认框（展示文件数量与大小）。
  const onClearShareFiles = () => {
    const [count] = shareStats;
    if (count === 0) {
      setToast(t("lan.shareFilesEmpty"));
      return;
    }
    setShowClearConfirm(true);
  };

  // 确认清空：执行删除并刷新统计。
  const onConfirmClear = async () => {
    setShowClearConfirm(false);
    try {
      const removed = await api.lanClearShareFiles();
      setToast(t("lan.clearShareDone", { count: removed }));
      await refreshShareFiles();
    } catch (e) {
      setToast(t("lan.operationFailed", { error: String(e) }));
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

        {/* 在线设备：位于「共享本机剪贴板」开关之后、「共享组」之前，始终显示 */}
        <div className="lan-online-devices">
          <div className="settings-subtitle">{t("lan.onlineDevices")}</div>
          <div className="settings-list">
            {/* 本机置顶：确认本机在线与共享状态（本机不进入远端连接列表） */}
            <div className="settings-list-item lan-self">
              <span className="item-name">{name || t("lan.thisDevice")}</span>
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

        <div className="settings-row">
          <span>{t("lan.group")}</span>
          <input
            type="text"
            placeholder={t("lan.groupPlaceholder")}
            value={group}
            disabled={shareOut}
            onChange={(e) => setGroup(e.target.value)}
            onBlur={onFieldBlur}
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
              disabled={shareOut}
              onChange={(e) => setKey(e.target.value)}
              onBlur={onFieldBlur}
            />
            <button
              type="button"
              className="input-reveal"
              title={t("lan.toggleKey")}
              aria-label={t("lan.toggleKey")}
              disabled={shareOut}
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
          <span>{t("lan.listenPort")}</span>
          <input
            type="number"
            min={1}
            max={65535}
            value={listenPort}
            disabled={shareOut}
            onChange={(e) => {
              setListenPort(Number(e.target.value));
              setPortError("");
            }}
            onBlur={onFieldBlur}
          />
        </div>
        <p className="settings-hint">{t("lan.listenPortHint", { port })}</p>
        {portError && <p className="settings-hint lan-port-error">{portError}</p>}

        <div className="settings-row lan-device-info-head">
          <span>{t("lan.deviceInfo")}</span>
        </div>
        <p className="settings-hint lan-device-info-value">
          {t("lan.name")}：{name || "-"}，{t("lan.deviceId")}：{deviceId || "-"}，
          {t("lan.localIp")}：{localIp || t("lan.localIpUnknown")}
        </p>

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
            disabled={shareOut}
            onChange={(e) => setManualPeers(e.target.value)}
            onBlur={onFieldBlur}
          />
          <p className="settings-hint">{t("lan.manualPeersHint", { port })}</p>
        </div>

        <hr className="lan-divider" />

        <div className="settings-row">
          <span>{t("lan.fileLimit")}</span>
          <input
            type="number"
            min={1}
            max={1024}
            value={fileLimitMb}
            onChange={(e) => {
              const v = Number(e.target.value);
              setFileLimitMb(v);
              // 文件上限改动即时生效（静默，避免每次按键弹提示），与共享是否开启无关。
              void applyConfig({ fileLimitMb: v }, true);
            }}
          />
        </div>
        <p className="settings-hint">{t("lan.fileLimitHint")}</p>

        <div className="settings-row settings-row-column">
          <span className="lan-share-types-head">{t("lan.shareTypes")}</span>
          <div className="lan-share-types">
            {SHARE_TYPES.map((tp) => (
              <div key={tp} className="lan-share-type-row">
                <span>{t(`type.${tp}`)}</span>
                <label className="switch">
                  <input
                    type="checkbox"
                    checked={shareTypes.includes(tp)}
                    onChange={(e) => {
                      const next = e.target.checked
                        ? [...shareTypes, tp]
                        : shareTypes.filter((x) => x !== tp);
                      setShareTypes(next);
                      // 共享类型改动即时生效（静默），与共享是否开启无关。
                      void applyConfig({ shareTypes: next }, true);
                    }}
                  />
                  <span className="slider" />
                </label>
              </div>
            ))}
          </div>
          <p className="settings-hint">{t("lan.shareTypesHint")}</p>
        </div>

        <div className="settings-actions">
          <button disabled={testing} onClick={() => void onTest()}>
            {testing ? t("lan.testing") : t("lan.testSend")}
          </button>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-card-title lan-subtitle">{t("lan.shareFiles")}</div>
        <p className="settings-hint">
          {t("lan.shareFilesStat", { count: shareStats[0], size: formatBytes(shareStats[1]) })}
        </p>
        <div className="settings-row settings-row-column">
          <span>{t("lan.shareFilesLocation")}</span>
          <p className="share-folder-path">{shareFolderPath || "-"}</p>
        </div>
        <div className="settings-row settings-row-actions">
          <button className="btn-secondary" disabled={busy} onClick={() => void onOpenShareFolder()}>
            {t("lan.openShareFolder")}
          </button>
          <button className="btn-danger" disabled={busy} onClick={() => void onClearShareFiles()}>
            {t("lan.clearShareFiles")}
          </button>
        </div>
      </div>

      {showClearConfirm && (
        <div className="modal-overlay" onClick={() => setShowClearConfirm(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">{t("lan.clearShareConfirmTitle")}</div>
            <p className="modal-body">
              {t("lan.clearShareConfirm", {
                count: shareStats[0],
                size: formatBytes(shareStats[1]),
              })}
            </p>
            <div className="modal-actions">
              <button className="btn-secondary" onClick={() => setShowClearConfirm(false)}>
                {t("confirm.cancel")}
              </button>
              <button className="btn-danger" onClick={() => void onConfirmClear()}>
                {t("lan.clearShareFiles")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

/** 上线事件：按「机器名」合并插入对端（与后端 peers() 展示层去重一致——同名只保留
 * 一条，避免 VPN / IP 共享下同一台机器因 device_id 重生 / 多路径连接重复展示）。
 * p 无有效名称（空或等于 deviceId）时退化为按 deviceId 更新。 */
function upsertPeer(prev: api.PeerInfo[], p: api.PeerInfo): api.PeerInfo[] {
  const byName =
    p.name && p.name !== p.deviceId
      ? prev.filter((x) => x.name !== p.name)
      : prev.filter((x) => x.deviceId !== p.deviceId);
  return [...byName, p];
}
