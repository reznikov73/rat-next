@echo off
setlocal
title Rat Next - Viewer
set "EXE=%~dp0rat-viewer.exe"
if not exist "%EXE%" (
  echo [rat-next] rat-viewer.exe not found:
  echo   %EXE%
  echo Build it first:
  echo   cargo build --release -p rat-viewer --features rat-viewer/ffmpeg
  pause
  exit /b 1
)
set "IP="
set /p "IP=Enter agent machine IP (e.g. 192.168.1.50): "
set "IP=%IP: =%"
if "%IP%"=="" (
  echo [rat-next] No IP entered.
  pause
  exit /b 1
)
echo [rat-next] Connecting to %IP%:55140 ...
"%EXE%" --connect %IP%:55140
echo.
echo [rat-next] Viewer closed.
pause