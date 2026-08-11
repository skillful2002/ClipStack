// L4 · i18n 校验：确保局域网共享相关 key 在所有语言均存在，且参数替换正常。

import { describe, it, expect } from "vitest";
import { translations } from "./translations";
import { translate } from "./index";
import type { ResolvedLang } from "./types";

const LAN_KEYS = [
  "lan.title",
  "lan.hint",
  "lan.group",
  "lan.groupPlaceholder",
  "lan.key",
  "lan.keyPlaceholder",
  "lan.toggleKey",
  "lan.keyPlaceholderKeep",
  "lan.name",
  "lan.namePlaceholder",
  "lan.shareOut",
  "lan.shareOutHint",
  "lan.fileLimit",
  "lan.manualPeers",
  "lan.manualPeersPlaceholder",
  "lan.port",
  "lan.listenPort",
  "lan.listenPortHint",
  "lan.localIp",
  "lan.localIpUnknown",
  "lan.portInUse",
  "lan.manualPeersHint",
  "lan.onlineDevices",
  "lan.noPeers",
  "lan.deviceId",
  "lan.thisDevice",
  "lan.sharing",
  "lan.shareStopped",
  "lan.save",
  "lan.noOtherPeers",
  "lan.testSend",
  "lan.testing",
  "lan.saved",
  "lan.saveFailed",
  "lan.testSent",
  "lan.testFailed",
  "lan.peerOnline",
  "lan.peerOffline",
  "lan.receivedFrom",
  "item.local",
];

describe("i18n LAN keys", () => {
  const langs = Object.keys(translations) as ResolvedLang[];

  it("all languages define every LAN key", () => {
    expect(langs.length).toBeGreaterThanOrEqual(6);
    for (const lang of langs) {
      for (const k of LAN_KEYS) {
        expect(translations[lang][k], `${lang} missing ${k}`).toBeDefined();
        expect(translations[lang][k], `${lang} empty ${k}`).not.toBe("");
      }
    }
  });

  it("translate substitutes {params}", () => {
    expect(translate("zh-CN", "lan.testSent", { n: 3 })).toContain("3");
    expect(translate("en", "lan.peerOnline", { name: "MacBook" })).toContain("MacBook");
  });

  it("missing key falls back to key itself", () => {
    expect(translate("en", "lan.does.not.exist")).toBe("lan.does.not.exist");
  });
});
