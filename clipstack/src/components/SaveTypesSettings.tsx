// P1a · 保存的历史类型设置：文本 / 图片 / 文件三类开关，默认均启用。
//
// 关闭某类型后，该类型内容在捕获阶段被跳过，不再写入历史（已存在记录保留）。
// 按既定决策，此处不提供「一键清理已禁用类型历史」按钮。

import { useEffect, useState } from "react";
import * as api from "../lib/tauri";
import { useHistory } from "../store/history";
import { useT } from "../lib/i18n";

export function SaveTypesSettings() {
  const t = useT();
  const setToast = useHistory((s) => s.setToast);
  const [saveText, setSaveText] = useState(true);
  const [saveImage, setSaveImage] = useState(true);
  const [saveFile, setSaveFile] = useState(true);

  // 初始化：读取三类保存开关。
  useEffect(() => {
    void (async () => {
      try {
        const settings = await api.getSettings();
        const st = settings.find((s) => s.key === "save_text")?.value;
        if (st != null) setSaveText(st !== "0");
        const si = settings.find((s) => s.key === "save_image")?.value;
        if (si != null) setSaveImage(si !== "0");
        const sf = settings.find((s) => s.key === "save_file")?.value;
        if (sf != null) setSaveFile(sf !== "0");
      } catch {
        /* 读取失败静默，使用默认值 */
      }
    })();
  }, []);

  const toggle = async (
    key: string,
    next: boolean,
    setter: (v: boolean) => void,
  ) => {
    setter(next);
    try {
      await api.updateSetting(key, next ? "1" : "0");
    } catch (e) {
      setToast(String(e));
    }
  };

  return (
    <>
      <div className="settings-subtitle">{t("savetypes.title")}</div>
      <p className="settings-hint">{t("savetypes.hint")}</p>
      <div className="type-toggles">
        <label className="type-toggle">
          <span>{t("type.text")}</span>
          <span className="switch">
            <input
              type="checkbox"
              checked={saveText}
              onChange={(e) => void toggle("save_text", e.target.checked, setSaveText)}
            />
            <span className="slider" />
          </span>
        </label>
        <label className="type-toggle">
          <span>{t("type.image")}</span>
          <span className="switch">
            <input
              type="checkbox"
              checked={saveImage}
              onChange={(e) => void toggle("save_image", e.target.checked, setSaveImage)}
            />
            <span className="slider" />
          </span>
        </label>
        <label className="type-toggle">
          <span>{t("type.file")}</span>
          <span className="switch">
            <input
              type="checkbox"
              checked={saveFile}
              onChange={(e) => void toggle("save_file", e.target.checked, setSaveFile)}
            />
            <span className="slider" />
          </span>
        </label>
      </div>
    </>
  );
}
