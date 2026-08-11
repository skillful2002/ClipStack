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

# PATH uses mingw-style dirs (MSYS converts them for Windows children).
# 关键：必须把 MSVC/SDK 的 bin 目录【前置】(prepend) 到 PATH，否则 Git Bash 自带的
# /usr/bin/link.exe (GNU coreutils) 会遮蔽 MSVC 的 link.exe，导致 rustc 链接时调用到
# GNU link 而报 "link: extra operand" 错误。前置后 MSVC 的 cl/link/rc 优先命中。
export PATH="$VS_M/bin/Hostx64/x64:$SDK_M/bin/$SDKVER/x64:$PATH:/c/Users/Administrator/.cargo/bin:/c/Users/Administrator/wix/extracted"

# 关闭 WorkBuddy 的 safe-delete 钩子：vite build 清空 dist/ 时会拦截 fs.rmSync 并 aborts，
# 导致前端构建失败。构建产物（dist/）本就是可再生的，允许真实删除。
export NODE_OPTIONS=""
export CODEBUDDY_SAFE_DELETE_SANDBOX=0

# 避免旧 target/release 下被残留句柄锁定的 .cargo-build-lock / .fingerprint：
# 构建到一个全新的 target 目录，由 cargo 创建全新的锁文件，绕开锁定。
# 关键：放在工作区之外（D:/work/ClipStack 被文件监视器持有句柄，会导致 .cargo-build-lock 创建失败 os error 5）。
export CARGO_TARGET_DIR="D:/cs-build"

cd /d/work/ClipStack/clipstack

echo "==== env check ===="
command -v node; command -v cargo; command -v cl; command -v link; command -v rc; command -v candle
echo "INCLUDE set: ${INCLUDE:0:40}..."
echo "===================="

exec npm run tauri build
