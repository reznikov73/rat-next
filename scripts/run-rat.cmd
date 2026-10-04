@echo off
setlocal
title Rat Next
set "EXE=%~dp0..\target\release\rat.exe"
if not exist "%EXE%" (
  echo [rat-next] rat.exe not found: "%EXE%"
  echo Build: cargo build --release -p rat --features rat/ffmpeg
  pause
  exit /b 1
)
"%EXE%"
echo.
echo [rat-next] Rat Next closed.
pause