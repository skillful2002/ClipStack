// P0 · 锁屏遮罩：锁定态覆盖全界面，必须先解锁才能查看剪贴板内容。
//
// 锁定态下后端内存无密钥，历史内容不可读；解锁成功后由本组件把 locked 置 false、
// 重新加载历史，露出解密后的内容。Touch ID 解锁仅在 macOS 且已设主密码时提供。

import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import { useLock } from "../store/lock";
import { useHistory } from "../store/history";
import { useT } from "../lib/i18n";

function LockIcon() {
  return (
    <svg
      className="lock-icon"
      width="40"
      height="40"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <rect x="5" y="11" width="14" height="10" rx="2" />
      <path d="M8 11V7a4 4 0 0 1 8 0v4" />
    </svg>
  );
}

export function LockGate() {
  const t = useT();
  const locked = useLock((s) => s.locked);
  const hasPassword = useLock((s) => s.hasPassword);
  const platform = useLock((s) => s.platform);
  const setLocked = useLock((s) => s.setLocked);
  const load = useHistory((s) => s.load);

  const [pwd, setPwd] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  // 是否实际启用了 Touch ID 解锁（受 use_touch_id 设置控制，而非仅看平台/主密码）。
  const [touchEnabled, setTouchEnabled] = useState(false);
  // Touch ID 硬件可用性（需要 Xcode CLT 才能运行 swift 子进程）。
  const [touchIdAvailable, setTouchIdAvailable] = useState(false);

  // 锁定态下读取 use_touch_id 设置 + 检测 CLT 可用性。
  useEffect(() => {
    if (!locked) return;
    void (async () => {
      try {
        const settings = await api.getSettings();
        const ut = settings.find((s) => s.key === "use_touch_id")?.value;
        // 仅当设置项明确为 "1"（用户显式开启）才展示 Touch ID 按钮；缺省/关闭/其它值均隐藏。
        setTouchEnabled(ut === "1");
        // 检测 CLT：Touch ID 解锁依赖 swift 子进程。
        const cltOk = await api.checkTouchIdAvailable();
        setTouchIdAvailable(cltOk);
      } catch {
        /* 读取失败则保守地隐藏 Touch ID 按钮 */
      }
    })();
  }, [locked]);

  if (!locked) return null;

  const afterUnlock = () => {
    setLocked(false);
    setPwd("");
    setError("");
    // 解锁后内存已载入密钥，重新拉取历史以显示解密内容。
    void load();
  };

  const onUnlock = async () => {
    if (!pwd) {
      setError(t("security.passwordPlaceholder"));
      return;
    }
    setBusy(true);
    setError("");
    try {
      const ok = await api.unlock(pwd);
      if (ok) {
        afterUnlock();
      } else {
        setError(t("security.wrongPassword"));
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const onTouchId = async () => {
    setBusy(true);
    setError("");
    try {
      const ok = await api.unlockTouchId();
      if (ok) {
        afterUnlock();
      } else {
        setError(t("security.wrongPassword"));
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const showTouch = platform === "macOS" && hasPassword && touchEnabled && touchIdAvailable;

  return (
    <div className="lock-gate">
      <div className="lock-card">
        <LockIcon />
        <h2 className="lock-title">{t("security.lockedTitle")}</h2>
        <p className="lock-hint">{t("security.lockedHint")}</p>
        <input
          className="lock-input"
          type="password"
          autoFocus
          placeholder={t("security.passwordPlaceholder")}
          value={pwd}
          disabled={busy}
          onChange={(e) => setPwd(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void onUnlock();
          }}
        />
        {error && <div className="lock-error">{error}</div>}
        <button className="lock-btn" disabled={busy} onClick={() => void onUnlock()}>
          {t("security.unlock")}
        </button>
        {showTouch && (
          <button
            className="lock-btn lock-btn-ghost"
            disabled={busy}
            onClick={() => void onTouchId()}
          >
            {t("security.unlockTouchId")}
          </button>
        )}
      </div>
    </div>
  );
}
