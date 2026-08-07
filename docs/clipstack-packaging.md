# ClipStack 打包、签名与分发

> 对应开发阶段 **P7**。代码层面（bundle 配置、CI 工作流）已完成；
> 真正的签名 / 公证需要你的开发者证书，无法在沙箱中执行。

## 一、本地构建（无需证书）

```bash
# 前置：cargo 需在 PATH（新终端默认已生效；旧终端先 source）
source $HOME/.cargo/env
cd clipstack

# 开发预览
npm run tauri dev

# 产出安装包（macOS 默认 dmg + app；Windows 需 msi 目标在 Windows 上跑）
npm run tauri build
```

- macOS 产物：`src-tauri/target/release/bundle/dmg/*.dmg` 与 `.../app/`。
- 未配置签名身份时，Tauri 会做 **ad-hoc 签名**，本地可运行，但分发给他人的 dmg 会被 Gatekeeper 拦截——正式分发必须走下方签名 + 公证。

## 二、macOS 签名 + 公证（正式分发）

1. 加入 [Apple Developer Program](https://developer.apple.com/programs/)，取得：
   - **Developer ID Application** 证书（用于签名）。
   - **App Store Connect** 账号（用于公证，`APPLE_ID` / `APPLE_PASSWORD` 用专用「App 专用密码」）。
   - **Team ID**（`APPLE_TEAM_ID`）。
2. 钥匙串登录后，构建时通过环境变量传入（无需写死到配置）：

   ```bash
   export APPLE_SIGN_IDENTITY="Developer ID Application: <你的名称> (<TEAM_ID>)"
   export APPLE_ID="you@example.com"
   export APPLE_PASSWORD="xxxx-xxxx-xxxx-xxxx"   # App 专用密码
   export APPLE_TEAM_ID="<TEAM_ID>"
   npm run tauri build
   ```

3. Tauri 会自动：用 Developer ID 签名 → 提交公证 →  Stapling 票据到 dmg。
4. 验证：`spctl -a -vv -t install ./target/release/bundle/dmg/*.dmg` 应显示 `accepted`。

## 三、Windows 构建（.msi）

- 在 **Windows**  runner 上执行 `npm run tauri build`，产物 `target/release/bundle/msi/*.msi`。
- 默认 `webviewInstallMode: downloadBootstrapper`：安装包内嵌 WebView2 引导器，目标机无 WebView2 时自动下载（无需系统预装）。
- 如需代码签名，用 signtool 对 msi 签名（补充 CI Secrets 与步骤）。

## 四、CI（GitHub Actions）

见 `.github/workflows/release.yml`：

- 推送 `v*` 标签或手动触发（`Actions` 页 → `Release` → `Run workflow`）。
- `build-macos`：macOS runner。
  - 仓库**未配置** `APPLE_SIGN_IDENTITY` 等 Secrets 时，自动回退为 **ad-hoc 签名（`-`）**，仍能产出可本地测试的 `.dmg`；
  - 配置了 `APPLE_SIGN_IDENTITY` / `APPLE_ID` / `APPLE_PASSWORD`（App 专用密码）/ `APPLE_TEAM_ID` 后，自动走 **Developer ID 签名 + 公证**，产出可分发的 `.dmg`。
- `build-windows`：Windows runner，产出 `.msi`（WebV开启 downloadBootstrapper 引导）。
- 两个 job 的产物（`.dmg` / `.msi`）以 **GitHub Artifacts** 形式留存，构建完成后在 `Actions` → 对应 run → `Artifacts` 处下载；如需自动建 Release，可在 `tauri build` 后接 `softprops/action-gh-release`。

## 五、体积 / 资源基线（验收参考）

- 单二进制（Tauri + WebView），目标安装包几 MB ~ 几十 MB。
- 常驻内存目标低于同类 Electron 工具一个数量级（Tauri 后端为 Rust，无 Node 运行时）。

## 六、已知约束

- **签名 / 公证需要你的开发者证书**：本仓库在沙箱/CI 中**可以正常执行 `tauri build` 出包**（依赖已缓存、crates.io 可达），但本机钥匙串无 Developer ID 证书，因此本地与「无 Secret 的 CI」产物为 **ad-hoc 签名（未公证）**——自己机器可运行，分发给他人时 macOS 会报「无法验证开发者」、Windows SmartScreen 会拦截。正式分发务必在自有 Mac 配齐 4 个 `APPLE_*` Secrets 后由 CI 签名 + 公证，或对 Windows `.msi` 用 signtool 签名。
- **Windows 无法在 macOS 交叉编译**：Tauri 不支持从 macOS 产出 Windows 安装包，`.msi` 必须由 Windows 机器或 `build-windows` CI job 构建。
- **bundle identifier 以 `.app` 结尾**：`tauri.conf.json` 中 `identifier: tech.newxin-clipstack.app` 会被 Tauri 警告「与 macOS 应用包扩展冲突」，但不影响构建；如需消除警告，可改为 `tech.newxin.clipstack`（属此前设定，改动需同步考虑已发布版本）。
- 图片 / 文件的一键复制已通过 arboard 实现（图片写回 PNG、文件写回真实文件 URL，库内仅存文件路径），跨机粘贴验证仍需在真实系统上完成。

## 七、应用图标与托盘图标

### 图标来源
- 应用图标由 `src-tauri/icons/gen_app_icon.py`（PIL 脚本）生成 1024×1024 源图 `icon-source.png`，再运行 `tauri icon src-tauri/icons/icon-source.png` 生成全套平台图标（mac `icon.icns`、Win `icon.ico`、通用 `icon.png` 及 32/64/128/iOS/Android/StoreLogo 系列），替换默认 Tauri 图标。
- 托盘图标**复用应用图标**：`tray.rs` 通过 `app.default_window_icon()` 设置，无需单独维护。
- 托盘**菜单项**图标（`打开主界面` 窗口 glyph、`设置` 齿轮 glyph）为独立文件 `icons/menu-open.png` / `icons/menu-settings.png`（由 `gen_menu_icons.py` 生成，编译期内嵌）。

### dev 与 build 的差异（重要）
1. **Dock / 启动台不显示自定义图标**：`npm run tauri dev` 不打包成正式 `.app`，macOS 在 Dock 只显示默认执行文件图标；自定义图标仅在 `npm run tauri build` 生成正式 `.app` 包后才会出现。这是 Tauri 的固有行为，**不是 bug**。
2. **图标是编译期嵌入的**：`tauri-build`（`build.rs`）在 `cargo build` 时把 `tauri.conf.json` 引用的图标嵌入二进制；它**只在 `tauri.conf.json` 变动时**（`rerun-if-changed`）重新嵌入，**不会**监视图标 PNG 本身的变化。因此：
   - 仅重新生成 PNG（如跑 `tauri icon`）后，增量 dev/build **仍会用旧的嵌入图标**；
   - 修复方法：改完 PNG 后执行 `touch src-tauri/tauri.conf.json`，再**彻底退出** dev 进程重新 `npm run tauri dev`（或 `cargo clean` 后重建），托盘/窗口图标才会更新。
3. **macOS 菜单栏托盘图标渲染**：菜单栏托盘图标默认按「模板图」渲染（忽略彩色、用系统色反色）。彩色图标用作托盘时可能偏单色/发灰，属原生行为；若需更原生观感，可单独提供单色描边版并设为模板图。

### 托盘菜单项图标（彩色）
- 托盘下拉历史每条前的**类型图标**为彩色 PNG，由 `src-tauri/icons/gen_type_icons.py` 生成 `menu-type-{text,link,code,image}.png`，颜色取自前端 `src/lib/format.ts` 的 `TYPE_META`（文本 #059669 / 链接 #2563eb / 代码 #7c3aed / 图片 #ea580c），编译期内嵌。
- 该图标仅用于「菜单项」（`IconMenuItem`），不影响应用 / Dock 图标（仍按模板图渲染，见上）。改完 PNG 后同样需 `touch src-tauri/tauri.conf.json` 并重启 dev/build 才会更新（见第七节第 2 点）。
