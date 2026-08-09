#!/usr/bin/env bash
# 本地构建 macOS 安装包（.dmg），支持指定架构（Apple Silicon / Intel / Universal）。
#
# 背景：Tauri 自带的 create-dmg 在最后一步会通过 osascript 调用 Finder 美化窗口，
# 在无 GUI / 沙箱 / CI 无窗口会话的环境下会报 “Finder 遇到一个错误：发生权限违例 (-10004)”
# 而导致 tauri build 在打包 dmg 这一步失败（但 .app 已经产出）。
#
# 本脚本用 hdiutil 直接生成一份功能完整、可本地安装的 .dmg（含“拖到 Applications”快捷方式），
# 绕过 create-dmg 的 AppleScript 步骤，避免上述环境问题。
#
# 用法：
#   bash scripts/build-macos-dmg.sh                  # 默认构建本机架构（Apple Silicon）
#   bash scripts/build-macos-dmg.sh x86_64-apple-darwin   # 仅 Intel 包（产物名 ..._x64.dmg）
#   bash scripts/build-macos-dmg.sh universal-apple-darwin # 通用二进制（..._universal.dmg）

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PRODUCT_NAME="ClipStack"
APP_BIN="clipstack"          # .app 内可执行文件名（cargo bin name）
VERSION="0.1.2"

# 目标架构：默认本机 host triple
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
TARGET="${1:-$HOST_TRIPLE}"

# 跨架构构建时 cargo 产物目录统一为 src-tauri/target/<triple>/release
# （即使 TARGET 等于本机 triple，因为脚本总是显式传 --target，产物也在 triple 子目录）
CARGO_DIR="src-tauri/target/$TARGET/release"

# 产物名后缀
case "$TARGET" in
  aarch64-apple-darwin)   SUFFIX="aarch64" ;;
  x86_64-apple-darwin)    SUFFIX="x64" ;;
  universal-apple-darwin) SUFFIX="universal" ;;
  *)                      SUFFIX="$TARGET" ;;
esac

APP_DIR="$CARGO_DIR/bundle/macos/${PRODUCT_NAME}.app"
DMG_DIR="$CARGO_DIR/bundle/dmg"
DMG_OUT="${DMG_DIR}/${PRODUCT_NAME}_${VERSION}_${SUFFIX}.dmg"
STAGE="$(mktemp -d)"

cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

echo "==> 目标架构: ${TARGET}  (产物: ${PRODUCT_NAME}_${VERSION}_${SUFFIX}.dmg)"

# 1) 执行 tauri build。在无法控制 Finder 的环境里，dmg 步骤会失败，但 .app 已先于该步骤产出，
#    因此这里允许其非零退出，随后用 hdiutil 自行打包。
echo "==> 构建应用（tauri build --target ${TARGET}）..."
npm run tauri -- build --target "${TARGET}" || echo "注意：tauri build 在 dmg 步骤返回非零（通常是 create-dmg 的 AppleScript 环境问题），继续手动打包 .dmg。"

# 2) 校验 .app 是否产出
if [[ ! -d "$APP_DIR" ]]; then
  echo "错误：未找到已构建的 ${APP_DIR}，请检查上面的构建日志。" >&2
  exit 1
fi

# 3) 校验二进制架构
BIN="$APP_DIR/Contents/MacOS/$APP_BIN"
if [[ -f "$BIN" ]]; then
  echo "==> 二进制架构: $(lipo -archs "$BIN" 2>/dev/null || echo unknown)"
else
  echo "警告：未找到可执行文件 $BIN，跳过架构校验。" >&2
fi

# 4) 准备 dmg 内容：App + Applications 快捷方式
echo "==> 准备 dmg 内容..."
cp -R "$APP_DIR" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

# 5) 生成 dmg
echo "==> 生成 ${DMG_OUT} ..."
mkdir -p "$DMG_DIR"
rm -f "$DMG_OUT"
hdiutil create -volname "$PRODUCT_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG_OUT"

echo "==> 校验 dmg ..."
hdiutil verify "$DMG_OUT" >/dev/null && echo "dmg 校验通过：$(du -h "$DMG_OUT" | cut -f1)"
echo "完成：安装包位于 $DMG_OUT"
