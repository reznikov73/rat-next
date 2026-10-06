@echo off
setlocal
title Rat Next
set "EXE=%~dp0rat.exe"
if not exist "%EXE%" (
  echo [rat-next] rat.exe not found.
  pause
  exit /b 1
)
start "" "%EXE%"
exit /b 0