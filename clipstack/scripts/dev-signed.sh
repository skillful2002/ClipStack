#!/usr/bin/env bash
# 以「自动签名」方式启动 tauri dev：后台启动开发服务器，待裸二进制生成后 ad-hoc 签名，
# 并在每次 Rust 重建后自动重新签名，使开发期 Touch ID / 钥匙串生物识别尽量可用
# （避免 -34018）。适合需要本地验证 Touch ID 解锁的场景。
#
# 说明：tauri dev 运行的是裸二进制 target/debug/clipstack；重建会变回未签名，
# 本脚本在重建后自动补签，使下次启动的进程为已签名。
#
# 用法：bash scripts/dev-signed.sh   （代替 npm run tauri -- dev）
set -u
cd "$(dirname "$0")/.." || exit 1
ENT="src-tauri/entitlements.plist"
BIN="src-tauri/target/debug/clipstack"

npm run tauri -- dev &
TAURI_PID=$!
cleanup() { kill "$TAURI_PID" 2>/dev/null; wait "$TAURI_PID" 2>/dev/null; }
trap cleanup EXIT INT TERM

echo "等待 $BIN 生成（首次构建可能较慢）..."
for i in $(seq 1 300); do
  [ -f "$BIN" ] && break
  sleep 2
done
if [ ! -f "$BIN" ]; then
  echo "未找到 $BIN，请检查 tauri dev 是否启动成功" >&2
  exit 1
fi

LAST=""
echo "开始监听 $BIN，改动即重新签名..."
while kill -0 "$TAURI_PID" 2>/dev/null; do
  CUR=$(stat -f "%m" "$BIN" 2>/dev/null || echo "")
  if [ "$CUR" != "$LAST" ]; then
    if codesign --force --sign - --options runtime --entitlements "$ENT" "$BIN" 2>/dev/null; then
      echo "$(date +%H:%M:%S) 已签名 $BIN"
    else
      echo "$(date +%H:%M:%S) 签名进行中（二进制占用），下一轮重试"
    fi
    LAST="$CUR"
  fi
  sleep 2
done
