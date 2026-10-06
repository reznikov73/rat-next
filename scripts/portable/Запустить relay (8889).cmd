@echo off
setlocal
title Rat Next relay
set "EXE=%~dp0rat-relay.exe"
if not exist "%EXE%" (
  echo [rat-next] rat-relay.exe not found in this folder.
  pause
  exit /b 1
)
set "PORT=%~1"
if "%PORT%"=="" set "PORT=8889"
echo [rat-next] Starting relay on 0.0.0.0:%PORT% ...
echo [rat-next] Allow the Windows Firewall prompt if it appears.
echo [rat-next] A control window opens: logs, connections, settings, tray icon.
echo [rat-next] Closing the window (X) keeps relay running in the tray.
"%EXE%" --listen 0.0.0.0:%PORT%
echo.
echo Relay stopped.
pause