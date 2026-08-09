// P0 · 应用锁前端状态：锁定态 / 是否已设主密码 / 运行平台。
//
// 与后端 `AppState` 对齐：锁定态下内存无密钥，历史内容不可读；
// 解锁成功后由 LockGate / 设置页把 locked 置 false 并重新加载历史。

import { create } from "zustand";

interface LockStore {
  /** 当前是否锁定（锁定态展示锁屏遮罩，阻塞所有内容查看）。 */
  locked: boolean;
  /** 是否已设置主密码（决定显示「设置密码」还是「解锁 / 修改」）。 */
  hasPassword: boolean;
  /** 运行平台（"macOS" / "Windows" / "Linux"），用于决定是否展示 Touch ID 解锁。 */
  platform: string;
  setLocked: (v: boolean) => void;
  setHasPassword: (v: boolean) => void;
  setPlatform: (p: string) => void;
}

export const useLock = create<LockStore>((set) => ({
  locked: false,
  hasPassword: false,
  platform: "",
  setLocked: (v) => set({ locked: v }),
  setHasPassword: (v) => set({ hasPassword: v }),
  setPlatform: (p) => set({ platform: p }),
}));
