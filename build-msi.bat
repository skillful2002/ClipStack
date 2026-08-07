@echo off
REM ClipStack Windows MSI build helper
REM Sets up MSVC (cl/link/rc), WiX, cargo and runs `npm run tauri build`
setlocal

REM 1) MSVC build environment (cl.exe, link.exe, rc.exe, INCLUDE, LIB)
call "C:\Program Files (x86)\Microsoft Visual Studio\2019\Enterprise\VC\Auxiliary\Build\vcvarsall.bat" x64
if errorlevel 1 (
  echo [ERR] vcvarsall.bat failed
  exit /b 1
)

REM 2) Ensure cargo + WiX are on PATH
set "PATH=%USERPROFILE%\.cargo\bin;%USERPROFILE%\wix\extracted;%PATH%"

REM 3) Build
cd /d D:\work\ClipStack\clipstack
echo [INFO] node:   & where node
echo [INFO] cargo:  & where cargo
echo [INFO] candle: & where candle
echo [INFO] starting `npm run tauri build` ...
call npm run tauri build
set "RC=%errorlevel%"
echo [DONE] tauri build exit code = %RC%
exit /b %RC%
