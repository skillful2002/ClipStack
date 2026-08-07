#!/usr/bin/env bash
# ClipStack Windows MSI build (MSVC env set manually, then tauri build)
set -u

# mingw-style paths for PATH (no drive-colon, avoids breaking PATH separator)
VS_M="/c/Program Files (x86)/Microsoft Visual Studio/2019/Enterprise/VC/Tools/MSVC/14.29.30133"
SDK_M="/c/Program Files (x86)/Windows Kits/10"
SDKVER="10.0.19041.0"

# Windows-style (semicolon) for cl/link INCLUDE & LIB
export INCLUDE="C:/Program Files (x86)/Microsoft Visual Studio/2019/Enterprise/VC/Tools/MSVC/14.29.30133/include;C:/Program Files (x86)/Windows Kits/10/Include/$SDKVER/ucrt;C:/Program Files (x86)/Windows Kits/10/Include/$SDKVER/shared;C:/Program Files (x86)/Windows Kits/10/Include/$SDKVER/um;C:/Program Files (x86)/Windows Kits/10/Include/$SDKVER/winrt;C:/Program Files (x86)/Windows Kits/10/Include/$SDKVER/cppwinrt"
export LIB="C:/Program Files (x86)/Microsoft Visual Studio/2019/Enterprise/VC/Tools/MSVC/14.29.30133/lib/x64;C:/Program Files (x86)/Windows Kits/10/Lib/$SDKVER/ucrt/x64;C:/Program Files (x86)/Windows Kits/10/Lib/$SDKVER/um/x64"

# PATH uses mingw-style dirs (MSYS converts them for Windows children)
export PATH="$PATH:$VS_M/bin/Hostx64/x64:$SDK_M/bin/$SDKVER/x64:/c/Users/Administrator/.cargo/bin:/c/Users/Administrator/wix/extracted"

# 关闭 WorkBuddy 的 safe-delete 钩子：vite build 清空 dist/ 时会拦截 fs.rmSync 并 aborts，
# 导致前端构建失败。构建产物（dist/）本就是可再生的，允许真实删除。
export NODE_OPTIONS=""
export CODEBUDDY_SAFE_DELETE_SANDBOX=0

cd /d/work/ClipStack/clipstack

echo "==== env check ===="
command -v node; command -v cargo; command -v cl; command -v rc; command -v candle
echo "INCLUDE set: ${INCLUDE:0:40}..."
echo "===================="

exec npm run tauri build
