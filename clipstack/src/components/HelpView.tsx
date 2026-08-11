// 帮助视图：展示 ClipStack 使用说明（与 docs/USER_GUIDE.md 内容一致），
// 入口来自侧边栏与托盘菜单的「帮助」项。所有文案通过 i18n 翻译键获取，
// 切换界面语言时（useT 订阅语言状态）会自动重渲染。

import type { ReactNode } from "react";
import { useT } from "../lib/i18n";
import { useHistory } from "../store/history";

// 远程仓库与安装包下载地址（与 USER_GUIDE.md 保持一致，不随语言变化）。
const REPO_GITHUB = "https://github.com/skillful2002/ClipStack";
const REPO_GITEE = "https://gitee.com/liuzhengguo/ClipStack";
const RELEASE_GITHUB = "https://github.com/skillful2002/ClipStack/releases";
const RELEASE_GITEE = "https://gitee.com/liuzhengguo/ClipStack/releases";

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="help-section">
      <h3 className="help-h">{title}</h3>
      <div className="help-content">{children}</div>
    </section>
  );
}

export function HelpView() {
  const t = useT();
  const setView = useHistory((s) => s.setView);

  return (
    <section className="settings-pane">
      <h2 className="settings-title">{t("sidebar.help")}</h2>

      <div className="settings-card help-card">
        <p className="settings-hint help-intro">{t("help.intro")}</p>

        <Section title={t("help.install")}>
          <p className="help-p">{t("help.installText")}</p>
          <ul className="help-ul">
            <li>
              {t("help.giteeReleasesLabel")}
              <a className="help-link" href={RELEASE_GITEE} target="_blank" rel="noreferrer">
                {RELEASE_GITEE}
              </a>
            </li>
            <li>
              {t("help.githubReleasesLabel")}
              <a className="help-link" href={RELEASE_GITHUB} target="_blank" rel="noreferrer">
                {RELEASE_GITHUB}
              </a>
            </li>
          </ul>
          <p className="help-p">{t("help.installTip")}</p>
        </Section>

        <Section title={t("help.basic")}>
          <ul className="help-ul">
            <li>{t("help.basicAuto")}</li>
            <li>{t("help.basicCopy")}</li>
            <li>{t("help.basicPin")}</li>
            <li>{t("help.basicDelete")}</li>
          </ul>
        </Section>

        <Section title={t("help.filter")}>
          <p className="help-p">{t("help.filterText")}</p>
        </Section>

        <Section title={t("help.shortcuts")}>
          <table className="help-table">
            <tbody>
              <tr><td>{t("help.sc.categories")}</td><td><kbd>⌘1</kbd>–<kbd>⌘6</kbd></td></tr>
              <tr><td>{t("sidebar.settings")}</td><td><kbd>⌘,</kbd></td></tr>
              <tr><td>{t("sidebar.trash")}</td><td><kbd>⌘⇧T</kbd></td></tr>
              <tr><td>{t("help.sc.searchFocus")}</td><td><kbd>⌘/</kbd></td></tr>
              <tr><td>{t("action.copy")}</td><td><kbd>Enter</kbd></td></tr>
              <tr><td>{t("help.sc.pinFavDel")}</td><td><kbd>P</kbd> / <kbd>F</kbd> / <kbd>Delete</kbd></td></tr>
              <tr><td>{t("help.sc.tray")}</td><td><kbd>⌘⇧V</kbd> / <kbd>⌘1</kbd>–<kbd>⌘9</kbd></td></tr>
            </tbody>
          </table>
          <p className="help-p">{t("help.shortcutsNote")}</p>
        </Section>

        <Section title={t("help.settings")}>
          <ul className="help-ul">
            <li>{t("help.settingsAppearance")}</li>
            <li>{t("help.settingsLanguage")}</li>
            <li>{t("help.settingsStorage")}</li>
            <li>{t("help.settingsStartup")}</li>
            <li>{t("help.settingsSecurity")}</li>
          </ul>
        </Section>

        <Section title={t("help.security")}>
          <ul className="help-ul">
            <li>{t("help.securityEnc")}</li>
            <li>{t("help.securityLock")}</li>
            <li>{t("help.securityMask")}</li>
            <li>{t("help.securityRetention")}</li>
            <li>{t("help.securitySaveTypes")}</li>
            <li>{t("help.securityForgot")}</li>
          </ul>
        </Section>

        <Section title={t("help.trashTray")}>
          <p className="help-p">{t("help.trashTrayText")}</p>
        </Section>

        <Section title={t("help.sync")}>
          <p className="help-p">{t("help.syncText")}</p>
          <ul className="help-ul">
            <li>{t("help.lanPrereq")}</li>
            <li>{t("help.lanTray")}</li>
            <li>{t("help.lanSaveAs")}</li>
          </ul>
        </Section>

        <Section title={t("help.source")}>
          <ul className="help-ul">
            <li>
              {t("help.sourceGiteeLabel")}
              <a className="help-link" href={REPO_GITEE} target="_blank" rel="noreferrer">{REPO_GITEE}</a>
            </li>
            <li>
              {t("help.sourceGithubLabel")}
              <a className="help-link" href={REPO_GITHUB} target="_blank" rel="noreferrer">{REPO_GITHUB}</a>
            </li>
            <li>
              {t("help.sourceGiteeReleasesLabel")}
              <a className="help-link" href={RELEASE_GITEE} target="_blank" rel="noreferrer">{RELEASE_GITEE}</a>
            </li>
            <li>
              {t("help.sourceGithubReleasesLabel")}
              <a className="help-link" href={RELEASE_GITHUB} target="_blank" rel="noreferrer">{RELEASE_GITHUB}</a>
            </li>
          </ul>
        </Section>
      </div>

      <button className="about-back" onClick={() => setView("main")}>
        {t("about.back")}
      </button>
    </section>
  );
}
