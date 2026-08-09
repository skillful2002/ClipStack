#!/usr/bin/env bash
# 对 tauri dev 运行的「裸二进制」target/debug/clipstack 进行 ad-hoc 代码签名
# （含钥匙串权益），使 Touch ID / 钥匙串生物识别访问控制可正常工作
# （避免 -34018 errSecMissingEntitlement）。
#
# 说明：tauri dev 运行的是裸二进制（非 .app 包），钥匙串按「进程可执行文件签名」
# 校验，因此必须对 target/debug/clipstack 签名。
#
# 用法：
#   1) 终端：npm run tauri -- dev        （正常启动开发版）
#   2) 另开终端：bash scripts/sign-dev.sh
#   3) 退出并重新打开 ClipStack（让 tauri dev 从已签名二进制启动）
# 之后即可在应用内开启 / 使用 Touch ID。
# 注意：修改 Rust 代码触发重建后，二进制会变回未签名，请重新运行本脚本。
set -u
cd "$(dirname "$0")/.." || exit 1
ENT="src-tauri/entitlements.plist"
BIN="src-tauri/target/debug/clipstack"

if [ ! -f "$BIN" ]; then
  echo "未找到 $BIN（请先运行 tauri dev 让它编译出二进制）" >&2
  exit 1
fi

echo "正在对 $BIN 进行 ad-hoc 签名（含钥匙串权益）..."
if codesign --force --sign - --options runtime --entitlements "$ENT" "$BIN"; then
  echo "完成。请退出并重新打开 ClipStack，随后即可开启 / 使用 Touch ID。"
  echo "提示：Rust 代码改动触发重建后，请重新运行本脚本。"
else
  echo "签名失败，请确认本机 codesign 可用（Xcode Command Line Tools 已安装）。" >&2
  exit 1
fi
