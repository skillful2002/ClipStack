// 关于系统视图：展示应用版本、Tauri 版本、运行平台与架构；入口来自托盘菜单。

import { useEffect, useState } from "react";
import { getVersion, getTauriVersion } from "@tauri-apps/api/app";
import { useT } from "../lib/i18n";
import { useHistory } from "../store/history";
import { getSystemInfo } from "../lib/tauri";

interface AboutInfo {
  version: string;
  tauriVersion: string;
  platform: string;
  arch: string;
}

export function AboutView() {
  const t = useT();
  const setView = useHistory((s) => s.setView);
  const [info, setInfo] = useState<AboutInfo | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const [version, tauriVersion, sys] = await Promise.all([
          getVersion(),
          getTauriVersion(),
          getSystemInfo(),
        ]);
        setInfo({ version, tauriVersion, platform: sys.platform, arch: sys.arch });
      } catch {
        setInfo({ version: "—", tauriVersion: "—", platform: "—", arch: "—" });
      }
    })();
  }, []);

  const rows: { label: string; value: string }[] = info
    ? [
        { label: t("about.version"), value: info.version },
        { label: t("about.tauriVersion"), value: info.tauriVersion },
        { label: t("about.platform"), value: info.platform },
        { label: t("about.arch"), value: info.arch },
      ]
    : [{ label: t("settings.loading"), value: "" }];

  return (
    <section className="settings-pane">
      <h2 className="settings-title">{t("about.title")}</h2>

      <div className="settings-card about-card">
        <div className="about-brand">ClipStack</div>
        <p className="settings-hint about-desc">{t("about.desc")}</p>
        <div className="about-info">
          {rows.map((r, i) => (
            <div className="settings-row" key={i}>
              <span>{r.label}</span>
              <span className="about-value">{r.value}</span>
            </div>
          ))}
        </div>
      </div>

      <button className="about-back" onClick={() => setView("main")}>
        {t("about.back")}
      </button>
    </section>
  );
}
