// P0 · 安全设置区：设置 / 修改主密码、Touch ID 解锁、自动锁定策略、立即锁定。
//
// 仅在已解锁时可见（锁定态由 LockGate 遮罩，无法进入设置页）。
// 未启用安全时展示「设置主密码」；已启用时展示「修改主密码 / 锁定 / 自动锁」。

import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import * as api from "../lib/tauri";
import { useLock } from "../store/lock";
import { useHistory } from "../store/history";
import { useT } from "../lib/i18n";
import { ConfirmDialog } from "./ConfirmDialog";

// 闲置自动锁定可选项（秒）：1 / 5 / 15 / 30 / 60 分钟 / 1 天 / 2 天 / 永久(0)。
// 注意：永久(0) 置于列表末尾，作为最后一项。
const IDLE_OPTIONS = [60, 300, 900, 1800, 3600, 7200, 10800, 14400, 28800, 43200, 86400, 172800, 0];

/// 根据秒数生成「闲置自动锁定」下拉文案：0=永久，>=1 天按天，>=1 小时按小时，其余按分钟。
function idleLabel(sec: number, t: (k: string, p?: Record<string, string | number>) => string): string {
  if (sec === 0) return t("security.idleOff");
  if (sec >= 86400) return t("security.idleDays", { n: sec / 86400 });
  if (sec >= 3600) return t("security.idleHours", { n: sec / 3600 });
  return t("security.idleN", { n: Math.round(sec / 60) });
}

export function SecuritySettings() {
  const t = useT();
  const hasPassword = useLock((s) => s.hasPassword);
  const setHasPassword = useLock((s) => s.setHasPassword);
  const setLocked = useLock((s) => s.setLocked);
  const platform = useLock((s) => s.platform);
  const load = useHistory((s) => s.load);
  const setToast = useHistory((s) => s.setToast);

  // 设置主密码表单。
  const [pwd, setPwd] = useState("");
  const [confirmPwd, setConfirmPwd] = useState("");
  // 修改主密码表单。
  const [oldPwd, setOldPwd] = useState("");
  const [newPwd, setNewPwd] = useState("");
  const [confirmNew, setConfirmNew] = useState("");
  // 策略开关。
  const [useTouch, setUseTouch] = useState(false);
  const [idleSec, setIdleSec] = useState(0);
  // 敏感内容掩码开关。
  const [maskSensitive, setMaskSensitive] = useState(false);
  // 提示。
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  // 清除主密码确认对话框（Tauri WebView 不支持 window.confirm，必须用自定义模态）。
  const [confirmClear, setConfirmClear] = useState(false);
  // Touch ID 硬件可用性（需要 Xcode CLT 才能运行 swift 子进程）。
  const [touchIdAvailable, setTouchIdAvailable] = useState(false);

  // 仅 macOS + CLT 已安装才展示 Touch ID 选项。
  const showTouch = platform === "macOS" && touchIdAvailable;

  // 初始化：读取安全相关设置项（Touch ID / 闲置锁定 / 敏感掩码）+ CLT 可用性。
  useEffect(() => {
    void (async () => {
      try {
        const settings = await api.getSettings();
        const ut = settings.find((s) => s.key === "use_touch_id")?.value;
        if (ut != null) setUseTouch(ut !== "0");
        const idle = Number(settings.find((s) => s.key === "auto_lock_idle_seconds")?.value);
        if (!Number.isNaN(idle)) setIdleSec(idle);
        const ms = settings.find((s) => s.key === "mask_sensitive")?.value;
        if (ms != null) setMaskSensitive(ms !== "0");
        // 检测 Xcode CLT：Touch ID 解锁依赖 swift 子进程，swift 需要 CLT。
        const cltOk = await api.checkTouchIdAvailable();
        setTouchIdAvailable(cltOk);
      } catch {
        /* 读取失败静默，使用默认值 */
      }
    })();
  }, []);


  const onSetup = async () => {
    setError("");
    if (pwd.length < 6) {
      setError(t("security.pwdTooShort"));
      return;
    }
    if (pwd !== confirmPwd) {
      setError(t("security.pwdMismatch"));
      return;
    }
    setBusy(true);
    let ok = false;
    try {
      await api.setupMasterPassword(pwd);
      setHasPassword(true);
      setLocked(false);
      setPwd("");
      setConfirmPwd("");
      void load();
      setToast(t("security.setupSuccess"));
      ok = true;
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
    // 同步 Touch ID 开关到后端：确保设置页勾选真实生效。仅 macOS 会写入受
    // BiometryCurrentSet 保护的随机令牌；其它平台静默忽略失败，不阻断设置。
    if (ok) {
      try {
        await api.setTouchId(useTouch);
      } catch {
        /* 非 macOS 或钥匙串暂不可用，忽略 */
      }
    }
  };

  const onChangePwd = async () => {
    setError("");
    if (newPwd.length < 6) {
      setError(t("security.pwdTooShort"));
      return;
    }
    if (newPwd !== confirmNew) {
      setError(t("security.pwdMismatch"));
      return;
    }
    setBusy(true);
    try {
      await api.changeMasterPassword(oldPwd, newPwd);
      setOldPwd("");
      setNewPwd("");
      setConfirmNew("");
      setToast(t("security.changedSuccess"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // 点击「清除主密码」：弹出确认对话框（提示安全隐患），确认后由 doClear 真正清除。
  const onClear = () => {
    setError("");
    setConfirmClear(true);
  };

  // 确认清除：执行后端清除，并把界面同步回「未设置密码」状态。
  const doClear = async () => {
    setConfirmClear(false);
    setBusy(true);
    try {
      await api.clearMasterPassword();
      // 清除后向后端确认真实状态再同步前端，确保可靠回到「未设置密码」状态，
      // 避免任何状态残留导致再次设置时仍被误判为「已设置」而要求输入旧密码。
      const stillSet = await api.hasMasterPassword();
      setHasPassword(stillSet);
      setLocked(false);
      setUseTouch(false);
      setOldPwd("");
      setNewPwd("");
      setConfirmNew("");
      void load();
      setToast(t("security.clearSuccess"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const onLockNow = async () => {
    try {
      await api.lockApp();
      // 先隐藏主窗体（destroyOnClose=false，等同关闭行为），再更新锁定状态，
      // 避免 LockGate 遮罩在窗口仍可见时被渲染出来。解锁对话框仅在下次从托盘
      // 重新打开主窗体时出现（tray.rs 发 app-lock-changed + App.tsx 焦点同步）。
      await getCurrentWindow().hide();
      setLocked(true);
    } catch (e) {
      setToast(String(e));
    }
  };

  const onToggleTouch = async (next: boolean) => {
    setUseTouch(next);
    try {
      // 联动后端：写入 use_touch_id 设置，并在启用时把密钥写入受 Touch ID 保护的
      // 钥匙串项（禁用时删除该项），避免「关闭后仍可 Touch ID 解锁 / 启用却无校验」。
      await api.setTouchId(next);
    } catch (e) {
      setToast(String(e));
      setUseTouch(!next); // 回滚 UI 状态
    }
  };

  const onToggleMask = async (next: boolean) => {
    setMaskSensitive(next);
    try {
      await api.updateSetting("mask_sensitive", next ? "1" : "0");
      // 立即刷新主列表与回收站，使「掩码敏感内容」开关对已有条目即时生效。
      const store = useHistory.getState();
      void store.load();
      void store.loadTrash();
    } catch (e) {
      setToast(String(e));
    }
  };

  const onIdleChange = async (sec: number) => {
    setIdleSec(sec);
    try {
      await api.updateSetting("auto_lock_idle_seconds", String(sec));
    } catch (e) {
      setToast(String(e));
    }
  };

  return (
    <div className="settings-card">
      <div className="settings-card-title">{t("security.title")}</div>
      <div className="settings-row">
        <span>{t("security.status")}</span>
        <span className={hasPassword ? "sec-on" : "sec-off"}>
          {hasPassword ? t("security.statusEnabled") : t("security.statusDisabled")}
        </span>
      </div>

      <div className="settings-row">
        <span>{t("security.maskSensitive")}</span>
        <label className="switch">
          <input
            type="checkbox"
            checked={maskSensitive}
            onChange={(e) => void onToggleMask(e.target.checked)}
          />
          <span className="slider" />
        </label>
      </div>
      {/* 掩码提示随开关状态变化：关闭时明确提示「已关闭」，避免与上方主密码状态行混淆 */}
      <p className="settings-hint">
        {maskSensitive ? t("security.maskSensitiveHint") : t("security.maskSensitiveHintOff")}
      </p>

      {!hasPassword ? (
        <div className="sec-form">
          <p className="settings-hint">{t("security.setPasswordHint")}</p>
          <input
            type="password"
            placeholder={t("security.passwordPlaceholder")}
            value={pwd}
            onChange={(e) => setPwd(e.target.value)}
          />
          <input
            type="password"
            placeholder={t("security.confirmPlaceholder")}
            value={confirmPwd}
            onChange={(e) => setConfirmPwd(e.target.value)}
          />
          {showTouch && (
            <div className="settings-row">
              <span>{t("security.useTouchId")}</span>
              <label className="switch">
                <input
                  type="checkbox"
                  checked={useTouch}
                  onChange={(e) => void onToggleTouch(e.target.checked)}
                />
                <span className="slider" />
              </label>
            </div>
          )}
          {showTouch && <p className="settings-hint">{t("security.useTouchIdHint")}</p>}
          {error && <div className="lock-error">{error}</div>}
          <button className="sec-btn" disabled={busy} onClick={() => void onSetup()}>
            {t("security.setPassword")}
          </button>
        </div>
      ) : (
        <div className="sec-form">
          <div className="settings-row">
            <span>{t("security.autoLockIdle")}</span>
            <select
              className="app-select"
              value={idleSec}
              onChange={(e) => void onIdleChange(Number(e.target.value))}
            >
              {IDLE_OPTIONS.map((s) => (
                <option key={s} value={s}>
                  {idleLabel(s, t)}
                </option>
              ))}
            </select>
          </div>
          {showTouch && (
            <div className="settings-row">
              <span>{t("security.useTouchId")}</span>
              <label className="switch">
                <input
                  type="checkbox"
                  checked={useTouch}
                  onChange={(e) => void onToggleTouch(e.target.checked)}
                />
                <span className="slider" />
              </label>
            </div>
          )}
          {showTouch && <p className="settings-hint">{t("security.useTouchIdHint")}</p>}
          {/* 清除主密码确认/进行中时禁用：避免清除后仍可锁定（无解锁凭据会锁死） */}
          <button
            className="sec-btn"
            disabled={busy || confirmClear}
            onClick={() => void onLockNow()}
          >
            {t("security.lockNow")}
          </button>

          <p className="settings-hint">{t("security.changePasswordHint")}</p>
          <input
            type="password"
            placeholder={t("security.currentPassword")}
            value={oldPwd}
            onChange={(e) => setOldPwd(e.target.value)}
          />
          <input
            type="password"
            placeholder={t("security.newPassword")}
            value={newPwd}
            onChange={(e) => setNewPwd(e.target.value)}
          />
          <input
            type="password"
            placeholder={t("security.confirmPassword")}
            value={confirmNew}
            onChange={(e) => setConfirmNew(e.target.value)}
          />
          {error && <div className="lock-error">{error}</div>}
          <div className="settings-btn-row">
            <button className="sec-btn" disabled={busy} onClick={() => void onChangePwd()}>
              {t("security.changePassword")}
            </button>
            <button
              className="sec-btn danger"
              disabled={busy}
              onClick={() => void onClear()}
            >
              {t("security.clearPassword")}
            </button>
          </div>
          <p className="settings-hint">{t("security.clearHint")}</p>
        </div>
      )}

      {/* 清除主密码确认对话框：提示清除后的安全隐患，确认后真正清除 */}
      <ConfirmDialog
        open={confirmClear}
        title={t("security.clearPassword")}
        message={t("security.clearConfirm")}
        confirmLabel={t("security.clearConfirmAction")}
        cancelLabel={t("confirm.cancel")}
        danger
        busy={busy}
        onConfirm={() => void doClear()}
        onCancel={() => setConfirmClear(false)}
      />
    </div>
  );
}
