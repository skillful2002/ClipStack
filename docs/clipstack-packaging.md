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

- 推送 `v*` 标签或手动触发。
- `build-macos`：macOS runner，读取上述 4 个 Secrets 完成签名 + 公证，上传 `.dmg`。
- `build-windows`：Windows runner，产出 `.msi`。
- 产物以 GitHub Artifacts 形式留存；如需自动建 Release，可在 `tauri build` 后接 `softprops/action-gh-release`。

## 五、体积 / 资源基线（验收参考）

- 单二进制（Tauri + WebView），目标安装包几 MB ~ 几十 MB。
- 常驻内存目标低于同类 Electron 工具一个数量级（Tauri 后端为 Rust，无 Node 运行时）。

## 六、已知约束

- **沙箱无法执行签名 / 公证 / 真实 `tauri build` 出包**：本阶段代码配置已就绪，最终出包与 Gatekeeper/WebView2 验证需在你本机或 CI 完成。
- 图片 / 文件的一键复制因平台 API 限制暂未实现（前端对这两类已禁用复制按钮），后续可接入。
