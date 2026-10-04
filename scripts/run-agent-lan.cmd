@echo off
setlocal
title Rat Next - Agent (LAN)
set "EXE=%~dp0..\target\release\rat-agent.exe"
if not exist "%EXE%" (
  echo [rat-next] rat-agent.exe not found:
  echo   %EXE%
  echo Build it first:
  echo   cargo build --release -p rat-agent --features rat-agent/ffmpeg
  pause
  exit /b 1
)
echo [rat-next] Starting agent on all interfaces, port 55140.
echo [rat-next] If Windows Firewall asks - allow access for Private networks.
echo.
"%EXE%" --listen 0.0.0.0:55140
echo.
echo [rat-next] Agent stopped.
pause