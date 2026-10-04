@echo off
chcp 65001 >nul
title Rat Next — Вьюер
set /p IP=Введите IP-адрес машины агента (например 192.168.1.50): 
if "%IP%"=="" (
  echo IP не введён.
  pause
  exit /b 1
)
echo Подключение к %IP%:55140 ...
"%~dp0..\target\release\rat-viewer.exe" --connect %IP%:55140
echo.
echo Вьюер закрыт.
pause